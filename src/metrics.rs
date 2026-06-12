//! Aggregation of history data into a single `Report` consumed by both the
//! terminal renderer and the Markdown exporter.

use crate::filter::PathFilter;
use crate::history::History;
use chrono::DateTime;
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};

const DAY: i64 = 86_400;
/// Top contributor owning at least this share of commits = bus factor 1.
const BUS_FACTOR_SHARE: f64 = 0.60;
/// Top contributor silent for this long = likely gone.
const INACTIVE_DAYS: i64 = 180;
/// Churn threshold for a single-author file to count as a knowledge silo.
const SILO_MIN_CHURN: u32 = 5;
/// Firefight commits per year considered normal.
const FIREFIGHT_NORMAL_PER_YEAR: usize = 6;

pub struct FileMetrics {
    pub path: String,
    pub churn: u32,
    pub bug_fixes: u32,
    pub authors: Vec<String>,
    pub last_touched: i64,
    pub loc: Option<usize>,
    /// 0–100 churn×size hotspot score; 0 for deleted or low-churn files.
    pub score: f64,
}

impl FileMetrics {
    pub fn risk(&self) -> Risk {
        if self.score >= 50.0 && self.bug_fixes >= 2 {
            Risk::Critical
        } else if self.score >= 50.0 || self.bug_fixes >= 2 {
            Risk::High
        } else {
            Risk::Watch
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Risk {
    Critical,
    High,
    Watch,
}

impl Risk {
    pub fn label(self) -> &'static str {
        match self {
            Risk::Critical => "CRITICAL",
            Risk::High => "HIGH",
            Risk::Watch => "WATCH",
        }
    }
}

pub struct AuthorStats {
    pub name: String,
    pub commits: u32,
    pub share: f64,
    pub last_active: i64,
}

pub struct Velocity {
    /// Commits per calendar month over the entire history, ordered by month.
    pub months: BTreeMap<String, u32>,
    pub trend: Trend,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Trend {
    Steady,
    Declining,
    Cliff,
    Spiky,
    Sparse,
}

impl Trend {
    pub fn describe(self) -> &'static str {
        match self {
            Trend::Steady => "steady rhythm — healthy",
            Trend::Declining => "declining over recent months — team may be losing momentum",
            Trend::Cliff => "sharp single-month drop — did someone leave?",
            Trend::Spiky => "spikes followed by quiet months — work batched into releases",
            Trend::Sparse => "not enough history for a trend",
        }
    }
}

pub struct Firefight {
    pub count_last_year: usize,
    pub recent: Vec<String>,
}

pub struct Report {
    /// All files in the window passing filters, sorted by churn desc.
    pub files: Vec<FileMetrics>,
    /// Indices into `files`, sorted by hotspot score desc (score > 0 only).
    pub hotspots: Vec<usize>,
    /// Indices into `files` with bug fixes, sorted by bug-fix count desc.
    pub bug_clusters: Vec<usize>,
    pub authors: Vec<AuthorStats>,
    /// True when ownership was computed from the included paths in the
    /// analysis window (user passed --path / --exclude-path) instead of the
    /// full history.
    pub ownership_scoped: bool,
    pub total_commits: usize,
    pub active_authors_last_year: usize,
    pub velocity: Velocity,
    pub firefight: Firefight,
    pub now: i64,
}

impl Report {
    pub fn compute(
        history: &History,
        loc: &HashMap<String, usize>,
        filter: &PathFilter,
        now: i64,
    ) -> Self {
        let files = file_metrics(history, loc, filter);

        let mut hotspots: Vec<usize> = (0..files.len()).filter(|&i| files[i].score > 0.0).collect();
        hotspots.sort_by(|&a, &b| files[b].score.total_cmp(&files[a].score));

        let mut bug_clusters: Vec<usize> = (0..files.len())
            .filter(|&i| files[i].bug_fixes > 0)
            .collect();
        bug_clusters.sort_by_key(|&i| std::cmp::Reverse(files[i].bug_fixes));

        // With a user-narrowed path set, ownership answers "who contributed
        // to these paths" from the analysis window; otherwise it keeps
        // `git shortlog -sn --no-merges` full-history semantics.
        let ownership_scoped = filter.is_restrictive();
        let authors = if ownership_scoped {
            scoped_author_stats(history, filter)
        } else {
            author_stats(history)
        };
        let active_authors_last_year = authors
            .iter()
            .filter(|a| now - a.last_active <= 365 * DAY)
            .count();

        Report {
            hotspots,
            bug_clusters,
            authors,
            ownership_scoped,
            total_commits: history.metas.len(),
            active_authors_last_year,
            velocity: velocity(history),
            firefight: firefight(history, now),
            files,
            now,
        }
    }

    /// Top contributor flags: (share-too-high, inactive-for-180-days).
    pub fn bus_factor_flags(&self) -> (bool, bool) {
        match self.authors.first() {
            Some(top) => (
                top.share >= BUS_FACTOR_SHARE,
                self.now - top.last_active > INACTIVE_DAYS * DAY,
            ),
            None => (false, false),
        }
    }

    /// Single-author files with enough churn to be a knowledge silo.
    pub fn silos(&self) -> Vec<&FileMetrics> {
        let mut silos: Vec<&FileMetrics> = self
            .files
            .iter()
            .filter(|f| f.churn >= SILO_MIN_CHURN && f.authors.len() == 1)
            .collect();
        silos.sort_by_key(|f| std::cmp::Reverse(f.churn));
        silos
    }

    /// Paths in both the churn top-5 and the bug-cluster top-5 — the highest
    /// risk files in the repository.
    pub fn churn_bug_overlap(&self) -> Vec<&str> {
        let top_bugs: HashSet<&str> = self
            .bug_clusters
            .iter()
            .take(5)
            .map(|&i| &self.files[i])
            .filter(|f| f.bug_fixes >= 2)
            .map(|f| f.path.as_str())
            .collect();
        self.files
            .iter()
            .take(5)
            .filter(|f| top_bugs.contains(f.path.as_str()))
            .map(|f| f.path.as_str())
            .collect()
    }

    pub fn firefight_verdict(&self) -> &'static str {
        match self.firefight.count_last_year {
            0 => "no revert/hotfix commits found — stable team, or commit messages don't say",
            n if n <= FIREFIGHT_NORMAL_PER_YEAR => "occasional firefighting — normal",
            n if n <= FIREFIGHT_NORMAL_PER_YEAR * 4 => {
                "elevated revert/hotfix rate — review test and deploy reliability"
            }
            _ => "frequent reverts/hotfixes — the team does not trust its deploy process",
        }
    }
}

#[derive(Default)]
struct FileAgg {
    churn: u32,
    bug_fixes: u32,
    authors: HashSet<String>,
    last_touched: i64,
}

fn file_metrics(
    history: &History,
    loc: &HashMap<String, usize>,
    filter: &PathFilter,
) -> Vec<FileMetrics> {
    let map: HashMap<&str, FileAgg> = history
        .changes
        .par_iter()
        .filter(|c| filter.included(&c.path))
        .fold(HashMap::new, |mut map: HashMap<&str, FileAgg>, change| {
            let meta = &history.metas[change.meta_idx];
            let agg = map.entry(change.path.as_str()).or_default();
            agg.churn += 1;
            if meta.is_bugfix {
                agg.bug_fixes += 1;
            }
            agg.authors.insert(meta.author.clone());
            agg.last_touched = agg.last_touched.max(meta.time);
            map
        })
        .reduce(HashMap::new, |mut acc, other| {
            for (path, agg) in other {
                let entry = acc.entry(path).or_default();
                entry.churn += agg.churn;
                entry.bug_fixes += agg.bug_fixes;
                entry.authors.extend(agg.authors);
                entry.last_touched = entry.last_touched.max(agg.last_touched);
            }
            acc
        });

    let max_churn = map.values().map(|a| a.churn).max().unwrap_or(1) as f64;
    let max_loc = map
        .keys()
        .filter_map(|p| loc.get(*p))
        .copied()
        .max()
        .unwrap_or(1) as f64;

    let mut files: Vec<FileMetrics> = map
        .into_iter()
        .map(|(path, agg)| {
            let file_loc = loc.get(path).copied();
            let score = match file_loc {
                Some(l) if agg.churn >= 2 => {
                    100.0 * (f64::from(agg.churn) / max_churn) * ((1.0 + l as f64).ln())
                        / (1.0 + max_loc).ln()
                }
                _ => 0.0,
            };
            let mut authors: Vec<String> = agg.authors.into_iter().collect();
            authors.sort();
            FileMetrics {
                path: path.to_string(),
                churn: agg.churn,
                bug_fixes: agg.bug_fixes,
                authors,
                last_touched: agg.last_touched,
                loc: file_loc,
                score,
            }
        })
        .collect();
    files.sort_by(|a, b| b.churn.cmp(&a.churn).then_with(|| a.path.cmp(&b.path)));
    files
}

/// Ownership over the included paths only: an author is counted once per
/// window commit that touched at least one included file.
fn scoped_author_stats(history: &History, filter: &PathFilter) -> Vec<AuthorStats> {
    let mut commits_by_author: HashMap<&str, HashSet<usize>> = HashMap::new();
    let mut total_commits: HashSet<usize> = HashSet::new();
    for change in history.changes.iter().filter(|c| filter.included(&c.path)) {
        let author = history.metas[change.meta_idx].author.as_str();
        commits_by_author
            .entry(author)
            .or_default()
            .insert(change.meta_idx);
        total_commits.insert(change.meta_idx);
    }
    let total = total_commits.len().max(1) as f64;
    let mut authors: Vec<AuthorStats> = commits_by_author
        .into_iter()
        .map(|(name, commit_idxs)| AuthorStats {
            commits: commit_idxs.len() as u32,
            share: commit_idxs.len() as f64 / total,
            last_active: commit_idxs
                .iter()
                .map(|&i| history.metas[i].time)
                .max()
                .unwrap_or(0),
            name: name.to_string(),
        })
        .collect();
    authors.sort_by(|a, b| b.commits.cmp(&a.commits).then_with(|| a.name.cmp(&b.name)));
    authors
}

fn author_stats(history: &History) -> Vec<AuthorStats> {
    let mut map: HashMap<&str, (u32, i64)> = HashMap::new();
    let mut total = 0u32;
    for meta in history.metas.iter().filter(|m| !m.is_merge) {
        total += 1;
        let entry = map.entry(meta.author.as_str()).or_insert((0, 0));
        entry.0 += 1;
        entry.1 = entry.1.max(meta.time);
    }
    let mut authors: Vec<AuthorStats> = map
        .into_iter()
        .map(|(name, (commits, last_active))| AuthorStats {
            name: name.to_string(),
            commits,
            share: f64::from(commits) / f64::from(total.max(1)),
            last_active,
        })
        .collect();
    authors.sort_by(|a, b| b.commits.cmp(&a.commits).then_with(|| a.name.cmp(&b.name)));
    authors
}

fn velocity(history: &History) -> Velocity {
    let mut months: BTreeMap<String, u32> = BTreeMap::new();
    for meta in &history.metas {
        if let Some(dt) = DateTime::from_timestamp(meta.time, 0) {
            *months.entry(dt.format("%Y-%m").to_string()).or_default() += 1;
        }
    }
    let trend = classify_trend(&months);
    Velocity { months, trend }
}

fn classify_trend(months: &BTreeMap<String, u32>) -> Trend {
    // Drop the current (likely partial) month: the last key.
    let counts: Vec<f64> = months
        .values()
        .map(|&c| f64::from(c))
        .take(months.len().saturating_sub(1))
        .collect();
    if counts.len() < 6 {
        return Trend::Sparse;
    }
    // Cliff: a recent month dropping to less than half its predecessor with
    // the level staying low afterwards (oscillating histories recover and
    // therefore don't qualify).
    let tail = &counts[counts.len().saturating_sub(12)..];
    for i in 0..tail.len() - 1 {
        let (before, after_drop) = (tail[i], tail[i + 1]);
        if before >= 4.0 && after_drop < before * 0.5 {
            let rest = &tail[i + 1..];
            let rest_avg = rest.iter().sum::<f64>() / rest.len() as f64;
            if rest_avg < before * 0.5 {
                return Trend::Cliff;
            }
        }
    }
    let recent = &counts[counts.len() - 3..];
    let prior = &counts[..counts.len() - 3];
    let recent_avg = recent.iter().sum::<f64>() / recent.len() as f64;
    let prior_avg = prior.iter().sum::<f64>() / prior.len() as f64;
    if prior_avg > 0.0 && recent_avg < prior_avg * 0.5 {
        return Trend::Declining;
    }
    // Spiky: high variance relative to the mean (release batching).
    let mean = counts.iter().sum::<f64>() / counts.len() as f64;
    if mean > 0.0 {
        let variance = counts.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / counts.len() as f64;
        if variance.sqrt() > mean * 0.8 {
            return Trend::Spiky;
        }
    }
    Trend::Steady
}

fn firefight(history: &History, now: i64) -> Firefight {
    let recent_window = now - 365 * DAY;
    let hits: Vec<&crate::history::CommitMeta> = history
        .metas
        .iter()
        .filter(|m| !m.is_merge && m.is_firefight && m.time >= recent_window)
        .collect();
    Firefight {
        count_last_year: hits.len(),
        recent: hits.iter().take(5).map(|m| m.summary.clone()).collect(),
    }
}

/// Human-friendly "N days ago" for table cells.
pub fn days_ago(now: i64, then: i64) -> String {
    match (now - then) / DAY {
        0 => "today".to_string(),
        1 => "yesterday".to_string(),
        d => format!("{d}d ago"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{CommitMeta, FileChange, History};

    const NOW: i64 = 1_780_000_000;

    fn meta(author: &str, days_back: i64, bugfix: bool, firefight: bool) -> CommitMeta {
        CommitMeta {
            author: author.to_string(),
            time: NOW - days_back * DAY,
            summary: "summary".to_string(),
            is_merge: false,
            is_bugfix: bugfix,
            is_firefight: firefight,
        }
    }

    fn months(counts: &[u32]) -> BTreeMap<String, u32> {
        counts
            .iter()
            .enumerate()
            .map(|(i, &c)| (format!("2025-{:02}", i + 1), c))
            .collect()
    }

    #[test]
    fn trend_sparse_steady_declining_cliff_spiky() {
        // Last month is dropped as partial, hence the trailing sentinel.
        assert!(classify_trend(&months(&[3, 4, 1])) == Trend::Sparse);
        assert!(classify_trend(&months(&[5, 5, 6, 5, 6, 5, 5, 1])) == Trend::Steady);
        assert!(classify_trend(&months(&[12, 11, 10, 9, 8, 5, 4, 3, 1])) == Trend::Declining);
        assert!(classify_trend(&months(&[10, 10, 10, 10, 2, 2, 2, 2, 1])) == Trend::Cliff);
        assert!(classify_trend(&months(&[1, 30, 1, 30, 1, 30, 1, 30, 1])) == Trend::Spiky);
    }

    #[test]
    fn days_ago_formats() {
        assert_eq!(days_ago(NOW, NOW), "today");
        assert_eq!(days_ago(NOW, NOW - DAY), "yesterday");
        assert_eq!(days_ago(NOW, NOW - 12 * DAY), "12d ago");
    }

    #[test]
    fn risk_classification() {
        let file = |score: f64, bug_fixes: u32| FileMetrics {
            path: "f".into(),
            churn: 5,
            bug_fixes,
            authors: vec!["a".into()],
            last_touched: NOW,
            loc: Some(100),
            score,
        };
        assert_eq!(file(80.0, 3).risk().label(), "CRITICAL");
        assert_eq!(file(80.0, 0).risk().label(), "HIGH");
        assert_eq!(file(10.0, 3).risk().label(), "HIGH");
        assert_eq!(file(10.0, 1).risk().label(), "WATCH");
    }

    #[test]
    fn report_aggregates_files_and_authors() {
        // Alice churns core.rs (one bugfix), Bob touches api.rs once.
        let metas = vec![
            meta("Alice", 10, false, false),
            meta("Alice", 8, true, false),
            meta("Alice", 6, false, false),
            meta("Bob", 2, false, true),
        ];
        let changes = vec![
            FileChange {
                path: "src/core.rs".into(),
                meta_idx: 0,
            },
            FileChange {
                path: "src/core.rs".into(),
                meta_idx: 1,
            },
            FileChange {
                path: "src/core.rs".into(),
                meta_idx: 2,
            },
            FileChange {
                path: "src/api.rs".into(),
                meta_idx: 3,
            },
            FileChange {
                path: "Cargo.lock".into(),
                meta_idx: 0,
            },
        ];
        let history = History {
            metas,
            changes,
            window_commits: 4,
        };
        let loc: HashMap<String, usize> = [
            ("src/core.rs".to_string(), 200),
            ("src/api.rs".to_string(), 20),
        ]
        .into();
        let filter = crate::filter::PathFilter::new(None, true);
        let report = Report::compute(&history, &loc, &filter, NOW);

        assert!(report.files.iter().all(|f| f.path != "Cargo.lock"));
        let core = &report.files[0];
        assert_eq!(core.path, "src/core.rs");
        assert_eq!(core.churn, 3);
        assert_eq!(core.bug_fixes, 1);
        assert_eq!(core.authors, vec!["Alice".to_string()]);
        assert_eq!(core.last_touched, NOW - 6 * DAY);
        assert!(core.score > 0.0);

        // api.rs has churn 1 → no hotspot score.
        assert_eq!(report.hotspots, vec![0]);
        assert_eq!(report.bug_clusters, vec![0]);

        assert_eq!(report.authors[0].name, "Alice");
        assert_eq!(report.authors[0].commits, 3);
        assert!((report.authors[0].share - 0.75).abs() < 1e-9);
        assert_eq!(report.firefight.count_last_year, 1);
        assert_eq!(report.total_commits, 4);
    }

    #[test]
    fn bus_factor_flags_dominant_and_inactive() {
        let metas = vec![
            meta("Alice", 200, false, false),
            meta("Alice", 210, false, false),
            meta("Alice", 220, false, false),
            meta("Bob", 1, false, false),
        ];
        let history = History {
            metas,
            changes: vec![],
            window_commits: 4,
        };
        let report = Report::compute(
            &history,
            &HashMap::new(),
            &crate::filter::PathFilter::new(None, true),
            NOW,
        );
        let (dominant, inactive) = report.bus_factor_flags();
        assert!(dominant, "75% share should flag bus factor");
        assert!(inactive, "top author silent 200 days should flag inactive");
        assert_eq!(report.active_authors_last_year, 2);
    }

    #[test]
    fn scoped_ownership_counts_only_contributors_to_included_paths() {
        let metas = vec![
            meta("Alice", 5, false, false),
            meta("Alice", 4, false, false),
            meta("Bob", 3, false, false),
        ];
        let changes = vec![
            FileChange {
                path: "src/a.rs".into(),
                meta_idx: 0,
            },
            FileChange {
                path: "src/a.rs".into(),
                meta_idx: 1,
            },
            FileChange {
                path: "docs/x.txt".into(),
                meta_idx: 2,
            },
        ];
        let history = History {
            metas,
            changes,
            window_commits: 3,
        };
        let loc = HashMap::new();

        let scoped = crate::filter::PathFilter::new(Some("src".into()), true);
        let report = Report::compute(&history, &loc, &scoped, NOW);
        assert!(report.ownership_scoped);
        assert_eq!(report.authors.len(), 1);
        assert_eq!(report.authors[0].name, "Alice");
        assert_eq!(report.authors[0].commits, 2);
        assert!((report.authors[0].share - 1.0).abs() < 1e-9);
        assert_eq!(report.authors[0].last_active, NOW - 4 * DAY);

        let unscoped = crate::filter::PathFilter::new(None, true);
        let report = Report::compute(&history, &loc, &unscoped, NOW);
        assert!(!report.ownership_scoped);
        assert_eq!(report.authors.len(), 2);
    }

    #[test]
    fn silos_and_overlap() {
        let metas: Vec<CommitMeta> = (0..6).map(|i| meta("Alice", i, i < 2, false)).collect();
        let changes: Vec<FileChange> = (0..6)
            .map(|i| FileChange {
                path: "src/silo.rs".into(),
                meta_idx: i,
            })
            .collect();
        let history = History {
            metas,
            changes,
            window_commits: 6,
        };
        let loc: HashMap<String, usize> = [("src/silo.rs".to_string(), 500)].into();
        let report = Report::compute(
            &history,
            &loc,
            &crate::filter::PathFilter::new(None, true),
            NOW,
        );
        let silos = report.silos();
        assert_eq!(silos.len(), 1);
        assert_eq!(silos[0].path, "src/silo.rs");
        // 6 changes, 2 bug fixes → high churn AND high bug.
        assert_eq!(report.churn_bug_overlap(), vec!["src/silo.rs"]);
    }
}
