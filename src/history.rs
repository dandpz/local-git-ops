//! History walk engine.
//!
//! Two passes:
//! 1. A sequential `Revwalk` over the entire history collecting lightweight
//!    per-commit metadata (needed for velocity / bus-factor fidelity).
//! 2. A rayon-parallel diff pass restricted to the analysis window. A
//!    `git2::Repository` is not `Sync`, so each rayon worker opens its own
//!    handle via `map_init`.

use anyhow::{Context, Result, anyhow};
use git2::{Oid, Repository, Sort};
use rayon::prelude::*;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

static BUG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(fix(es|ed)?|bug|broken)\b").unwrap());
static FIREFIGHT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(revert(ed)?|hotfix|emergency|roll(ed)?[ -]?back|rollback)\b").unwrap()
});

pub struct CommitMeta {
    pub author: String,
    pub time: i64,
    pub summary: String,
    pub is_merge: bool,
    pub is_bugfix: bool,
    pub is_firefight: bool,
}

pub struct FileChange {
    pub path: String,
    /// Index into `History::metas` for the commit that touched this path.
    pub meta_idx: usize,
}

pub struct History {
    /// Every commit reachable from HEAD, newest first.
    pub metas: Vec<CommitMeta>,
    /// Changed paths for the non-merge commits inside the analysis window.
    pub changes: Vec<FileChange>,
    /// Number of non-merge commits whose diffs were analyzed.
    pub window_commits: usize,
}

pub struct WindowOpts {
    pub max_commits: usize,
    pub days: Option<u32>,
    pub now: i64,
}

pub fn collect(repo: &Repository, opts: &WindowOpts) -> Result<History> {
    let gitdir = repo.path().to_path_buf();

    let mut walk = repo.revwalk().context("failed to start revision walk")?;
    walk.push_head()
        .context("failed to resolve HEAD (empty repository?)")?;
    walk.set_sorting(Sort::TIME)?;

    let cutoff = opts.days.map(|d| opts.now - i64::from(d) * 86_400);
    let mut metas = Vec::new();
    let mut window: Vec<(Oid, usize)> = Vec::new();

    for oid in walk {
        let oid = oid?;
        let commit = repo
            .find_commit(oid)
            .with_context(|| format!("failed to read commit {oid}"))?;
        let message = commit.message().unwrap_or("");
        let meta = CommitMeta {
            // Author names and summaries are attacker-controlled in untrusted
            // repos — strip control characters before they reach any output.
            author: crate::sanitize::strip_controls(
                commit.author().name().unwrap_or("<unknown>").trim(),
            ),
            time: commit.time().seconds(),
            summary: crate::sanitize::strip_controls(commit.summary().ok().flatten().unwrap_or("")),
            is_merge: commit.parent_count() > 1,
            is_bugfix: BUG_RE.is_match(message),
            is_firefight: FIREFIGHT_RE.is_match(message),
        };
        let in_window = cutoff.is_none_or(|c| meta.time >= c) && window.len() < opts.max_commits;
        if in_window && !meta.is_merge {
            window.push((oid, metas.len()));
        }
        metas.push(meta);
    }

    let window_commits = window.len();
    let changes = extract_changes(&gitdir, &window)?;

    Ok(History {
        metas,
        changes,
        window_commits,
    })
}

/// Diff every windowed commit against its first parent, in parallel.
fn extract_changes(gitdir: &Path, window: &[(Oid, usize)]) -> Result<Vec<FileChange>> {
    let per_commit: Vec<Vec<FileChange>> = window
        .par_iter()
        .map_init(
            || Repository::open(gitdir),
            |repo, (oid, meta_idx)| {
                let repo = repo
                    .as_ref()
                    .map_err(|e| anyhow!("failed to reopen repository on worker thread: {e}"))?;
                changed_paths(repo, *oid, *meta_idx)
            },
        )
        .collect::<Result<_>>()?;
    Ok(per_commit.into_iter().flatten().collect())
}

fn changed_paths(repo: &Repository, oid: Oid, meta_idx: usize) -> Result<Vec<FileChange>> {
    let commit = repo.find_commit(oid)?;
    let tree = commit.tree()?;
    // Root commits diff against the empty tree.
    let parent_tree = match commit.parent_count() {
        0 => None,
        _ => Some(commit.parent(0)?.tree()?),
    };
    let diff = repo
        .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)
        .with_context(|| format!("failed to diff commit {oid}"))?;
    Ok(diff
        .deltas()
        .filter_map(|delta| {
            delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| FileChange {
                    // Must match the sanitization in `loc::head_line_counts`
                    // so churn paths and line-count paths join correctly.
                    path: crate::sanitize::strip_controls(&p.to_string_lossy()),
                    meta_idx,
                })
        })
        .collect())
}

/// Open the repository containing `start` (walks upward like `git` itself).
pub fn open_repo(start: &Path) -> Result<Repository> {
    Repository::discover(start).with_context(|| {
        format!(
            "not inside a git repository (searched upward from {})",
            start.display()
        )
    })
}

/// Workdir-relative prefix of the user's cwd, used for auto-scoping.
pub fn cwd_scope(repo: &Repository) -> Option<String> {
    let workdir = repo.workdir()?.canonicalize().ok()?;
    let cwd = std::env::current_dir().ok()?.canonicalize().ok()?;
    let rel = cwd.strip_prefix(&workdir).ok()?;
    if rel.as_os_str().is_empty() {
        return None;
    }
    Some(rel.to_string_lossy().into_owned())
}

pub fn workdir(repo: &Repository) -> Result<PathBuf> {
    repo.workdir()
        .map(Path::to_path_buf)
        .context("bare repositories are not supported")
}

#[cfg(test)]
mod tests {
    use super::{BUG_RE, FIREFIGHT_RE};

    #[test]
    fn bug_regex_matches_word_boundaries() {
        assert!(BUG_RE.is_match("Fix crash on empty input"));
        assert!(BUG_RE.is_match("fixes #42"));
        assert!(BUG_RE.is_match("api response was broken"));
        assert!(BUG_RE.is_match("Bug: wrong offset"));
        assert!(!BUG_RE.is_match("add prefix tree"));
        assert!(!BUG_RE.is_match("debug logging"));
    }

    #[test]
    fn firefight_regex_matches() {
        assert!(FIREFIGHT_RE.is_match("Revert \"add api endpoint\""));
        assert!(FIREFIGHT_RE.is_match("hotfix: timeout"));
        assert!(FIREFIGHT_RE.is_match("emergency patch for prod"));
        assert!(FIREFIGHT_RE.is_match("rolled back the migration"));
        assert!(FIREFIGHT_RE.is_match("rollback deploy"));
        assert!(!FIREFIGHT_RE.is_match("add rolling average metric"));
    }
}
