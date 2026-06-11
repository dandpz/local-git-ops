//! Markdown report export — same sections as the terminal dashboard, rendered
//! as GitHub-flavored Markdown tables with verdicts as blockquotes.

use crate::metrics::{Report, days_ago};
use crate::render::Context;
use crate::sanitize::markdown as md_safe;
use anyhow::{Context as _, Result};
use chrono::DateTime;
use std::fmt::Write as _;
use std::path::Path;

pub fn write_markdown(report: &Report, ctx: &Context, dest: &Path) -> Result<()> {
    let md = build(report, ctx);
    std::fs::write(dest, md)
        .with_context(|| format!("failed to write Markdown report to {}", dest.display()))?;
    Ok(())
}

fn build(report: &Report, ctx: &Context) -> String {
    let mut md = String::new();
    let date = DateTime::from_timestamp(report.now, 0)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default();

    let _ = writeln!(md, "# Repository Health Report\n");
    let _ = writeln!(
        md,
        "- **Repository:** `{}` @ `{}`",
        ctx.repo_root, ctx.branch
    );
    let _ = writeln!(md, "- **Window:** {}", ctx.window_desc);
    if let Some(scope) = ctx.scope {
        let _ = writeln!(md, "- **Scope:** `{scope}/`");
    }
    let _ = writeln!(md, "- **History:** {} commits", report.total_commits);
    let _ = writeln!(md, "- **Generated:** {date} by local-git-ops\n");

    hotspots(&mut md, report, ctx);
    churn(&mut md, report, ctx);
    bug_clusters(&mut md, report, ctx);
    ownership(&mut md, report);
    velocity(&mut md, report);
    firefight(&mut md, report);
    md
}

fn hotspots(md: &mut String, report: &Report, ctx: &Context) {
    let _ = writeln!(md, "## 🔥 Maintenance Hotspots (churn × size)\n");
    if report.hotspots.is_empty() {
        let _ = writeln!(md, "No hotspots in the analysis window.\n");
        return;
    }
    let _ = writeln!(
        md,
        "| Risk | File | Score | Changes | Lines | Bug fixes | Authors |"
    );
    let _ = writeln!(
        md,
        "|------|------|------:|--------:|------:|----------:|--------:|"
    );
    for &i in report.hotspots.iter().take(ctx.top) {
        let f = &report.files[i];
        let _ = writeln!(
            md,
            "| {} | `{}` | {:.0} | {} | {} | {} | {} |",
            f.risk().label(),
            md_safe(&f.path),
            f.score,
            f.churn,
            f.loc.unwrap_or(0),
            f.bug_fixes,
            f.authors.len()
        );
    }
    let overlap = report.churn_bug_overlap();
    if !overlap.is_empty() {
        let listed: Vec<String> = overlap
            .iter()
            .map(|p| format!("`{}`", md_safe(p)))
            .collect();
        let _ = writeln!(
            md,
            "\n> ⚠️ **High-churn AND high-bug — your single biggest risk:** {}",
            listed.join(", ")
        );
    }
    let _ = writeln!(md);
}

fn churn(md: &mut String, report: &Report, ctx: &Context) {
    let _ = writeln!(md, "## 📈 Code Churn (what changes the most)\n");
    if report.files.is_empty() {
        let _ = writeln!(md, "No file changes in the analysis window.\n");
        return;
    }
    let _ = writeln!(md, "| File | Changes | Authors | Last touched |");
    let _ = writeln!(md, "|------|--------:|--------:|--------------|");
    for f in report.files.iter().take(ctx.top) {
        let _ = writeln!(
            md,
            "| `{}` | {} | {} | {} |",
            md_safe(&f.path),
            f.churn,
            f.authors.len(),
            days_ago(report.now, f.last_touched)
        );
    }
    let _ = writeln!(
        md,
        "\n> Lockfiles, changelogs and generated code are excluded by default.\n"
    );
}

fn bug_clusters(md: &mut String, report: &Report, ctx: &Context) {
    let _ = writeln!(md, "## 🐛 Bug Clusters (files touched by fix commits)\n");
    if report.bug_clusters.is_empty() {
        let _ = writeln!(
            md,
            "No bug-related commits matched — check commit message discipline.\n"
        );
        return;
    }
    let _ = writeln!(md, "| File | Bug fixes | Total changes |");
    let _ = writeln!(md, "|------|----------:|--------------:|");
    for &i in report.bug_clusters.iter().take(ctx.top) {
        let f = &report.files[i];
        let _ = writeln!(
            md,
            "| `{}` | {} | {} |",
            md_safe(&f.path),
            f.bug_fixes,
            f.churn
        );
    }
    let _ = writeln!(
        md,
        "\n> Depends on commit message discipline; a rough map beats no map.\n"
    );
}

fn ownership(md: &mut String, report: &Report) {
    let _ = writeln!(
        md,
        "## 👥 Ownership & Bus Factor (full history, no merges)\n"
    );
    let _ = writeln!(md, "| Author | Commits | Share | Last active |");
    let _ = writeln!(md, "|--------|--------:|------:|-------------|");
    for a in report.authors.iter().take(15) {
        let _ = writeln!(
            md,
            "| {} | {} | {:.0}% | {} |",
            md_safe(&a.name),
            a.commits,
            a.share * 100.0,
            days_ago(report.now, a.last_active)
        );
    }
    if report.authors.len() > 15 {
        let _ = writeln!(md, "\n*… and {} more*", report.authors.len() - 15);
    }
    let _ = writeln!(md);

    let (dominant, inactive) = report.bus_factor_flags();
    if let Some(top) = report.authors.first() {
        if dominant && inactive {
            let _ = writeln!(
                md,
                "> 🚨 **Bus factor crisis:** {} owns {:.0}% of commits and has been inactive for 6+ months.",
                md_safe(&top.name),
                top.share * 100.0
            );
        } else if dominant {
            let _ = writeln!(
                md,
                "> ⚠️ **Bus factor 1:** {} owns {:.0}% of all commits.",
                md_safe(&top.name),
                top.share * 100.0
            );
        } else if inactive {
            let _ = writeln!(
                md,
                "> ⚠️ Top all-time contributor {} has been inactive for 6+ months.",
                md_safe(&top.name)
            );
        }
    }
    let _ = writeln!(
        md,
        "> {} contributors total, {} active in the last year.",
        report.authors.len(),
        report.active_authors_last_year
    );
    let silos = report.silos();
    if !silos.is_empty() {
        let listed: Vec<String> = silos
            .iter()
            .take(5)
            .map(|f| format!("`{}` (only {})", md_safe(&f.path), md_safe(&f.authors[0])))
            .collect();
        let _ = writeln!(
            md,
            "> ⚠️ **Knowledge silos** — single-author hot files: {}.",
            listed.join(", ")
        );
    }
    let _ = writeln!(
        md,
        ">\n> *Caveat: squash-merge workflows compress authorship to whoever merged.*\n"
    );
}

fn velocity(md: &mut String, report: &Report) {
    let _ = writeln!(md, "## 📊 Commit Velocity (entire history)\n");
    let months = &report.velocity.months;
    if months.is_empty() {
        let _ = writeln!(md, "No commits found.\n");
        return;
    }
    let _ = writeln!(md, "| Month | Commits |");
    let _ = writeln!(md, "|-------|--------:|");
    let shown: Vec<(&String, &u32)> = months.iter().collect();
    let start = shown.len().saturating_sub(24);
    for (month, count) in &shown[start..] {
        let _ = writeln!(md, "| {month} | {count} |");
    }
    if start > 0 {
        let _ = writeln!(md, "\n*… {start} earlier months omitted*");
    }
    let _ = writeln!(md, "\n> Trend: {}.\n", report.velocity.trend.describe());
}

fn firefight(md: &mut String, report: &Report) {
    let _ = writeln!(md, "## 🚒 Firefighting (reverts/hotfixes, last 365 days)\n");
    let ff = &report.firefight;
    let _ = writeln!(
        md,
        "{} revert/hotfix/rollback commits ({:.1}/month).\n",
        ff.count_last_year,
        ff.count_last_year as f64 / 12.0
    );
    for summary in &ff.recent {
        let _ = writeln!(md, "- {}", md_safe(summary));
    }
    if !ff.recent.is_empty() {
        let _ = writeln!(md);
    }
    let _ = writeln!(md, "> {}.", report.firefight_verdict());
}
