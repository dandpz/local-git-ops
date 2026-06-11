//! HTML report export — same sections as the terminal dashboard, rendered as
//! a single self-contained file (embedded CSS, no external assets) so it can
//! be opened or shared offline.

use crate::metrics::{Report, Risk, Trend, days_ago};
use crate::render::Context;
use crate::sanitize::html as esc;
use anyhow::{Context as _, Result};
use chrono::DateTime;
use std::fmt::Write as _;
use std::path::Path;

const STYLE: &str = r#"
:root { color-scheme: dark; }
* { box-sizing: border-box; }
body {
  margin: 0; padding: 2rem 1rem; background: #0d1117; color: #e6edf3;
  font: 15px/1.5 -apple-system, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
}
main { max-width: 880px; margin: 0 auto; }
h1 { font-size: 1.6rem; border-bottom: 1px solid #30363d; padding-bottom: .5rem; }
h2 { font-size: 1.15rem; margin-top: 2.2rem; }
ul.meta { list-style: none; padding: 0; color: #8b949e; }
ul.meta li { margin: .15rem 0; }
table { border-collapse: collapse; width: 100%; margin: .6rem 0; }
th, td { text-align: left; padding: .35rem .6rem; border-bottom: 1px solid #21262d; }
th { color: #8b949e; font-weight: 600; font-size: .85rem; text-transform: uppercase; }
td.num, th.num { text-align: right; font-variant-numeric: tabular-nums; }
code { font-family: SFMono-Regular, Menlo, Consolas, monospace; font-size: .9em;
       background: #161b22; padding: .1rem .35rem; border-radius: 5px; }
.badge { font: 700 .75rem/1.6 SFMono-Regular, Menlo, Consolas, monospace;
         padding: .1rem .5rem; border-radius: 6px; border: 1px solid; }
.critical { color: #f85149; border-color: #f85149; background: #3d1418; }
.high     { color: #e3b341; border-color: #d29922; background: #3a2d10; }
.watch    { color: #3fb950; border-color: #2ea043; background: #12261e; }
.verdict { border-left: 3px solid #30363d; padding: .4rem .8rem; margin: .5rem 0;
           background: #161b22; border-radius: 0 6px 6px 0; }
.verdict.crit { border-color: #f85149; }
.verdict.warn { border-color: #d29922; }
.verdict.ok   { border-color: #2ea043; }
.note { color: #8b949e; font-size: .85rem; }
.bar { display: block; height: 10px; border-radius: 5px; min-width: 2px;
       background: linear-gradient(90deg, #39d2ff, #2188ff); }
td.barcell { width: 50%; }
footer { margin-top: 3rem; color: #484f58; font-size: .8rem; }
"#;

pub fn write_html(report: &Report, ctx: &Context, dest: &Path) -> Result<()> {
    let html = build(report, ctx);
    std::fs::write(dest, html)
        .with_context(|| format!("failed to write HTML report to {}", dest.display()))?;
    Ok(())
}

fn build(report: &Report, ctx: &Context) -> String {
    let mut h = String::new();
    let date = DateTime::from_timestamp(report.now, 0)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default();

    let _ = write!(
        h,
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>Repository Health Report — {}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n<main>\n",
        esc(ctx.repo_root)
    );
    let _ = writeln!(h, "<h1>Repository Health Report</h1>");
    let _ = writeln!(h, "<ul class=\"meta\">");
    let _ = writeln!(
        h,
        "<li><strong>Repository:</strong> <code>{}</code> @ <code>{}</code></li>",
        esc(ctx.repo_root),
        esc(ctx.branch)
    );
    let _ = writeln!(
        h,
        "<li><strong>Window:</strong> {}</li>",
        esc(ctx.window_desc)
    );
    if let Some(scope) = ctx.scope {
        let _ = writeln!(
            h,
            "<li><strong>Scope:</strong> <code>{}/</code></li>",
            esc(scope)
        );
    }
    if let Some(excluded) = ctx.excluded_authors {
        let _ = writeln!(
            h,
            "<li><strong>Excluded authors:</strong> {}</li>",
            esc(excluded)
        );
    }
    let _ = writeln!(
        h,
        "<li><strong>History:</strong> {} commits</li>",
        report.total_commits
    );
    let _ = writeln!(
        h,
        "<li><strong>Generated:</strong> {date} by local-git-ops</li>"
    );
    let _ = writeln!(h, "</ul>");

    hotspots(&mut h, report, ctx);
    churn(&mut h, report, ctx);
    bug_clusters(&mut h, report, ctx);
    ownership(&mut h, report);
    velocity(&mut h, report);
    firefight(&mut h, report);

    let _ = writeln!(h, "<footer>Generated offline by local-git-ops.</footer>");
    let _ = writeln!(h, "</main>\n</body>\n</html>");
    h
}

fn badge(risk: Risk) -> String {
    let class = match risk {
        Risk::Critical => "critical",
        Risk::High => "high",
        Risk::Watch => "watch",
    };
    format!("<span class=\"badge {class}\">{}</span>", risk.label())
}

fn verdict(h: &mut String, class: &str, text: &str) {
    let _ = writeln!(h, "<p class=\"verdict {class}\">{text}</p>");
}

fn hotspots(h: &mut String, report: &Report, ctx: &Context) {
    let _ = writeln!(h, "<h2>🔥 Maintenance Hotspots (churn × size)</h2>");
    if report.hotspots.is_empty() {
        let _ = writeln!(
            h,
            "<p class=\"note\">No hotspots in the analysis window.</p>"
        );
        return;
    }
    let _ = writeln!(
        h,
        "<table><thead><tr><th>Risk</th><th>File</th><th class=\"num\">Score</th>\
         <th class=\"num\">Changes</th><th class=\"num\">Lines</th>\
         <th class=\"num\">Bug fixes</th><th class=\"num\">Authors</th></tr></thead><tbody>"
    );
    for &i in report.hotspots.iter().take(ctx.top) {
        let f = &report.files[i];
        let _ = writeln!(
            h,
            "<tr><td>{}</td><td><code>{}</code></td><td class=\"num\">{:.0}</td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
            badge(f.risk()),
            esc(&f.path),
            f.score,
            f.churn,
            f.loc.unwrap_or(0),
            f.bug_fixes,
            f.authors.len()
        );
    }
    let _ = writeln!(h, "</tbody></table>");
    let overlap = report.churn_bug_overlap();
    if !overlap.is_empty() {
        let listed: Vec<String> = overlap
            .iter()
            .map(|p| format!("<code>{}</code>", esc(p)))
            .collect();
        verdict(
            h,
            "crit",
            &format!(
                "High-churn <em>and</em> high-bug — your single biggest risk: {}",
                listed.join(", ")
            ),
        );
    }
}

fn churn(h: &mut String, report: &Report, ctx: &Context) {
    let _ = writeln!(h, "<h2>📈 Code Churn (what changes the most)</h2>");
    if report.files.is_empty() {
        let _ = writeln!(
            h,
            "<p class=\"note\">No file changes in the analysis window.</p>"
        );
        return;
    }
    let _ = writeln!(
        h,
        "<table><thead><tr><th>File</th><th class=\"num\">Changes</th>\
         <th class=\"num\">Authors</th><th>Last touched</th></tr></thead><tbody>"
    );
    for f in report.files.iter().take(ctx.top) {
        let _ = writeln!(
            h,
            "<tr><td><code>{}</code></td><td class=\"num\">{}</td>\
             <td class=\"num\">{}</td><td>{}</td></tr>",
            esc(&f.path),
            f.churn,
            f.authors.len(),
            days_ago(report.now, f.last_touched)
        );
    }
    let _ = writeln!(h, "</tbody></table>");
    let _ = writeln!(
        h,
        "<p class=\"note\">Lockfiles, changelogs and generated code are excluded by default.</p>"
    );
}

fn bug_clusters(h: &mut String, report: &Report, ctx: &Context) {
    let _ = writeln!(h, "<h2>🐛 Bug Clusters (files touched by fix commits)</h2>");
    if report.bug_clusters.is_empty() {
        let _ = writeln!(
            h,
            "<p class=\"note\">No bug-related commits matched — check commit message discipline.</p>"
        );
        return;
    }
    let _ = writeln!(
        h,
        "<table><thead><tr><th>File</th><th class=\"num\">Bug fixes</th>\
         <th class=\"num\">Total changes</th></tr></thead><tbody>"
    );
    for &i in report.bug_clusters.iter().take(ctx.top) {
        let f = &report.files[i];
        let _ = writeln!(
            h,
            "<tr><td><code>{}</code></td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
            esc(&f.path),
            f.bug_fixes,
            f.churn
        );
    }
    let _ = writeln!(h, "</tbody></table>");
    let _ = writeln!(
        h,
        "<p class=\"note\">Depends on commit message discipline; a rough map beats no map.</p>"
    );
}

fn ownership(h: &mut String, report: &Report) {
    let _ = writeln!(
        h,
        "<h2>👥 Ownership &amp; Bus Factor (full history, no merges)</h2>"
    );
    let _ = writeln!(
        h,
        "<table><thead><tr><th>Author</th><th class=\"num\">Commits</th>\
         <th class=\"num\">Share</th><th>Last active</th></tr></thead><tbody>"
    );
    for a in report.authors.iter().take(15) {
        let _ = writeln!(
            h,
            "<tr><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{:.0}%</td><td>{}</td></tr>",
            esc(&a.name),
            a.commits,
            a.share * 100.0,
            days_ago(report.now, a.last_active)
        );
    }
    let _ = writeln!(h, "</tbody></table>");
    if report.authors.len() > 15 {
        let _ = writeln!(
            h,
            "<p class=\"note\">… and {} more</p>",
            report.authors.len() - 15
        );
    }

    let (dominant, inactive) = report.bus_factor_flags();
    if let Some(top) = report.authors.first() {
        if dominant && inactive {
            verdict(
                h,
                "crit",
                &format!(
                    "<strong>Bus factor crisis:</strong> {} owns {:.0}% of commits and has been inactive for 6+ months.",
                    esc(&top.name),
                    top.share * 100.0
                ),
            );
        } else if dominant {
            verdict(
                h,
                "warn",
                &format!(
                    "<strong>Bus factor 1:</strong> {} owns {:.0}% of all commits.",
                    esc(&top.name),
                    top.share * 100.0
                ),
            );
        } else if inactive {
            verdict(
                h,
                "warn",
                &format!(
                    "Top all-time contributor {} has been inactive for 6+ months.",
                    esc(&top.name)
                ),
            );
        }
    }
    let tail_class = if report.active_authors_last_year * 3 < report.authors.len() {
        "warn"
    } else {
        "ok"
    };
    verdict(
        h,
        tail_class,
        &format!(
            "{} contributors total, {} active in the last year.",
            report.authors.len(),
            report.active_authors_last_year
        ),
    );
    let silos = report.silos();
    if !silos.is_empty() {
        let listed: Vec<String> = silos
            .iter()
            .take(5)
            .map(|f| {
                format!(
                    "<code>{}</code> (only {})",
                    esc(&f.path),
                    esc(&f.authors[0])
                )
            })
            .collect();
        verdict(
            h,
            "warn",
            &format!(
                "<strong>Knowledge silos</strong> — single-author hot files: {}.",
                listed.join(", ")
            ),
        );
    }
    let _ = writeln!(
        h,
        "<p class=\"note\">Caveat: squash-merge workflows compress authorship to whoever merged.</p>"
    );
}

fn velocity(h: &mut String, report: &Report) {
    let _ = writeln!(h, "<h2>📊 Commit Velocity (entire history)</h2>");
    let months = &report.velocity.months;
    if months.is_empty() {
        let _ = writeln!(h, "<p class=\"note\">No commits found.</p>");
        return;
    }
    let max = months.values().copied().max().unwrap_or(1).max(1) as f64;
    let shown: Vec<(&String, &u32)> = months.iter().collect();
    let start = shown.len().saturating_sub(24);
    let _ = writeln!(
        h,
        "<table><thead><tr><th>Month</th><th class=\"num\">Commits</th><th></th></tr></thead><tbody>"
    );
    for (month, count) in &shown[start..] {
        let width = (f64::from(**count) / max * 100.0).max(1.0);
        let _ = writeln!(
            h,
            "<tr><td><code>{month}</code></td><td class=\"num\">{count}</td>\
             <td class=\"barcell\"><span class=\"bar\" style=\"width:{width:.0}%\"></span></td></tr>"
        );
    }
    let _ = writeln!(h, "</tbody></table>");
    if start > 0 {
        let _ = writeln!(h, "<p class=\"note\">… {start} earlier months omitted</p>");
    }
    let class = match report.velocity.trend {
        Trend::Steady | Trend::Sparse => "ok",
        Trend::Spiky => "warn",
        Trend::Declining | Trend::Cliff => "crit",
    };
    verdict(
        h,
        class,
        &format!("Trend: {}.", report.velocity.trend.describe()),
    );
}

fn firefight(h: &mut String, report: &Report) {
    let _ = writeln!(
        h,
        "<h2>🚒 Firefighting (reverts/hotfixes, last 365 days)</h2>"
    );
    let ff = &report.firefight;
    let _ = writeln!(
        h,
        "<p>{} revert/hotfix/rollback commits ({:.1}/month).</p>",
        ff.count_last_year,
        ff.count_last_year as f64 / 12.0
    );
    if !ff.recent.is_empty() {
        let _ = writeln!(h, "<ul>");
        for summary in &ff.recent {
            let _ = writeln!(h, "<li><code>{}</code></li>", esc(summary));
        }
        let _ = writeln!(h, "</ul>");
    }
    let class = match ff.count_last_year {
        0..=6 => "ok",
        7..=24 => "warn",
        _ => "crit",
    };
    verdict(h, class, &format!("{}.", report.firefight_verdict()));
}
