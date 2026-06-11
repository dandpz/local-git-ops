//! Terminal dashboard rendering.

use crate::metrics::{Report, Risk, days_ago};
use colored::Colorize;
use comfy_table::{Cell, Color, ContentArrangement, Table, presets::UTF8_FULL_CONDENSED};

pub struct Context<'a> {
    pub repo_root: &'a str,
    pub branch: &'a str,
    pub scope: Option<&'a str>,
    pub window_desc: &'a str,
    /// Summary of author exclusions (e.g. "dependabot[bot], *[bot]"), if any.
    pub excluded_authors: Option<&'a str>,
    pub top: usize,
}

pub fn dashboard(report: &Report, ctx: &Context) {
    header(report, ctx);
    hotspots(report, ctx);
    churn(report, ctx);
    bug_clusters(report, ctx);
    ownership(report, ctx);
    velocity(report);
    firefight(report);
}

fn new_table() -> Table {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL_CONDENSED)
        .set_content_arrangement(ContentArrangement::Dynamic);
    table
}

fn section(title: &str) {
    println!("\n{}", title.bold().underline());
}

fn verdict(text: &str, severity: Risk) {
    let bullet = match severity {
        Risk::Critical => format!("  ▲ {text}").red().bold(),
        Risk::High => format!("  ▲ {text}").yellow(),
        Risk::Watch => format!("  ● {text}").green(),
    };
    println!("{bullet}");
}

fn risk_cell(risk: Risk) -> Cell {
    let color = match risk {
        Risk::Critical => Color::Red,
        Risk::High => Color::Yellow,
        Risk::Watch => Color::Green,
    };
    Cell::new(risk.label()).fg(color)
}

fn header(report: &Report, ctx: &Context) {
    println!(
        "{} {}",
        "local-git-ops".bold().cyan(),
        format!("· {} @ {}", ctx.repo_root, ctx.branch).dimmed()
    );
    let scope = match ctx.scope {
        Some(s) => format!("  scope: {s}/"),
        None => String::new(),
    };
    let excluded = match ctx.excluded_authors {
        Some(e) => format!("  excluding authors: {e}"),
        None => String::new(),
    };
    println!(
        "{}",
        format!(
            "window: {} · history: {} commits{}{}",
            ctx.window_desc, report.total_commits, scope, excluded
        )
        .dimmed()
    );
}

fn hotspots(report: &Report, ctx: &Context) {
    section("🔥 Maintenance Hotspots (churn × size)");
    if report.hotspots.is_empty() {
        println!("{}", "  no hotspots in the analysis window".dimmed());
        return;
    }
    let mut table = new_table();
    table.set_header(vec![
        "risk",
        "file",
        "score",
        "changes",
        "lines",
        "bug fixes",
        "authors",
    ]);
    for &i in report.hotspots.iter().take(ctx.top) {
        let f = &report.files[i];
        table.add_row(vec![
            risk_cell(f.risk()),
            Cell::new(&f.path),
            Cell::new(format!("{:.0}", f.score)),
            Cell::new(f.churn),
            Cell::new(f.loc.unwrap_or(0)),
            Cell::new(f.bug_fixes),
            Cell::new(f.authors.len()),
        ]);
    }
    println!("{table}");

    let overlap = report.churn_bug_overlap();
    if !overlap.is_empty() {
        verdict(
            &format!(
                "high-churn AND high-bug — your single biggest risk: {}",
                overlap.join(", ")
            ),
            Risk::Critical,
        );
    }
}

fn churn(report: &Report, ctx: &Context) {
    section("📈 Code Churn (what changes the most)");
    if report.files.is_empty() {
        println!("{}", "  no file changes in the analysis window".dimmed());
        return;
    }
    let mut table = new_table();
    table.set_header(vec!["file", "changes", "authors", "last touched"]);
    for f in report.files.iter().take(ctx.top) {
        table.add_row(vec![
            Cell::new(&f.path),
            Cell::new(f.churn),
            Cell::new(f.authors.len()),
            Cell::new(days_ago(report.now, f.last_touched)),
        ]);
    }
    println!("{table}");
    println!(
        "{}",
        "  lockfiles/changelogs/generated code excluded (--no-default-filters to include)".dimmed()
    );
}

fn bug_clusters(report: &Report, ctx: &Context) {
    section("🐛 Bug Clusters (files touched by fix commits)");
    if report.bug_clusters.is_empty() {
        println!(
            "{}",
            "  no bug-related commits matched — check commit message discipline".dimmed()
        );
        return;
    }
    let mut table = new_table();
    table.set_header(vec!["file", "bug fixes", "total changes"]);
    for &i in report.bug_clusters.iter().take(ctx.top) {
        let f = &report.files[i];
        table.add_row(vec![
            Cell::new(&f.path),
            Cell::new(f.bug_fixes),
            Cell::new(f.churn),
        ]);
    }
    println!("{table}");
    println!(
        "{}",
        "  depends on commit message discipline; a rough map beats no map".dimmed()
    );
}

fn ownership(report: &Report, ctx: &Context) {
    section("👥 Ownership & Bus Factor (full history, no merges)");
    let mut table = new_table();
    table.set_header(vec!["author", "commits", "share", "last active"]);
    for a in report.authors.iter().take(15) {
        table.add_row(vec![
            Cell::new(&a.name),
            Cell::new(a.commits),
            Cell::new(format!("{:.0}%", a.share * 100.0)),
            Cell::new(days_ago(report.now, a.last_active)),
        ]);
    }
    println!("{table}");
    if report.authors.len() > 15 {
        println!(
            "{}",
            format!("  … and {} more", report.authors.len() - 15).dimmed()
        );
    }

    let (dominant, inactive) = report.bus_factor_flags();
    if let Some(top) = report.authors.first() {
        if dominant && inactive {
            verdict(
                &format!(
                    "bus factor crisis: {} owns {:.0}% of commits and has been inactive 6+ months",
                    top.name,
                    top.share * 100.0
                ),
                Risk::Critical,
            );
        } else if dominant {
            verdict(
                &format!(
                    "bus factor 1: {} owns {:.0}% of all commits",
                    top.name,
                    top.share * 100.0
                ),
                Risk::High,
            );
        } else if inactive {
            verdict(
                &format!(
                    "top all-time contributor {} inactive for 6+ months",
                    top.name
                ),
                Risk::High,
            );
        }
    }
    verdict(
        &format!(
            "{} contributors total, {} active in the last year",
            report.authors.len(),
            report.active_authors_last_year
        ),
        if report.active_authors_last_year * 3 < report.authors.len() {
            Risk::High
        } else {
            Risk::Watch
        },
    );

    let silos = report.silos();
    if !silos.is_empty() {
        let listed: Vec<String> = silos
            .iter()
            .take(5)
            .map(|f| format!("{} (only {})", f.path, f.authors[0]))
            .collect();
        verdict(
            &format!(
                "knowledge silos — single-author hot files: {}",
                listed.join(", ")
            ),
            Risk::High,
        );
    }
    println!(
        "{}",
        "  caveat: squash-merge workflows compress authorship to whoever merged".dimmed()
    );
    let _ = ctx;
}

fn velocity(report: &Report) {
    section("📊 Commit Velocity (entire history)");
    let months = &report.velocity.months;
    if months.is_empty() {
        println!("{}", "  no commits found".dimmed());
        return;
    }
    let max = months.values().copied().max().unwrap_or(1).max(1);
    let shown: Vec<(&String, &u32)> = months.iter().collect();
    let start = shown.len().saturating_sub(24);
    for (month, count) in &shown[start..] {
        let width = (**count as usize * 40).div_ceil(max as usize);
        println!("  {month}  {:>5}  {}", count, "█".repeat(width).cyan());
    }
    if start > 0 {
        println!("{}", format!("  … {start} earlier months omitted").dimmed());
    }
    let severity = match report.velocity.trend {
        crate::metrics::Trend::Steady | crate::metrics::Trend::Sparse => Risk::Watch,
        crate::metrics::Trend::Spiky => Risk::High,
        _ => Risk::Critical,
    };
    verdict(report.velocity.trend.describe(), severity);
}

fn firefight(report: &Report) {
    section("🚒 Firefighting (reverts/hotfixes, last 365 days)");
    let ff = &report.firefight;
    println!(
        "  {} revert/hotfix/rollback commits ({:.1}/month)",
        ff.count_last_year,
        ff.count_last_year as f64 / 12.0
    );
    for summary in &ff.recent {
        println!("{}", format!("    · {summary}").dimmed());
    }
    let severity = match ff.count_last_year {
        0..=6 => Risk::Watch,
        7..=24 => Risk::High,
        _ => Risk::Critical,
    };
    verdict(report.firefight_verdict(), severity);
    println!();
}
