use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use local_git_ops::{cli, export, filter, history, html, loc, metrics, render};
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    if let Err(err) = run() {
        eprintln!("{} {err:#}", "error:".red().bold());
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = cli::Cli::parse();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let repo = history::open_repo(&args.repo)?;
    let workdir = history::workdir(&repo)?;
    let branch = repo
        .head()
        .ok()
        .and_then(|h| h.shorthand().ok().map(str::to_string))
        .unwrap_or_else(|| "HEAD".to_string());

    let scope = match (&args.path, args.no_auto_scope) {
        (Some(p), _) => Some(p.trim_matches('/').to_string()),
        (None, false) => history::cwd_scope(&repo),
        (None, true) => None,
    }
    .filter(|s| !s.is_empty());
    let path_filter = filter::PathFilter::new(scope.clone(), !args.no_default_filters);

    let author_filter = filter::AuthorFilter::new(&args.exclude_author, args.exclude_bots);
    let excluded_desc = author_filter.describe();
    let window = history::WindowOpts {
        max_commits: args.max_commits(),
        days: args.days,
        now,
        authors: author_filter,
    };
    let hist = history::collect(&repo, &window)?;
    let line_counts = loc::head_line_counts(&repo)?;
    let report = metrics::Report::compute(&hist, &line_counts, &path_filter, now);

    let window_desc = match (args.days, args.commits) {
        (Some(d), Some(n)) => format!("last {d} days (max {n} commits)"),
        (Some(d), None) => format!("last {d} days ({} commits)", hist.window_commits),
        (None, _) => format!("last {} non-merge commits", hist.window_commits),
    };
    let ctx = render::Context {
        repo_root: &workdir.display().to_string(),
        branch: &branch,
        scope: scope.as_deref(),
        window_desc: &window_desc,
        excluded_authors: excluded_desc.as_deref(),
        top: args.top,
    };

    render::dashboard(&report, &ctx);

    if let Some(dest) = &args.export {
        let ext = dest
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        let kind = match ext.as_deref() {
            Some("html" | "htm") => {
                html::write_html(&report, &ctx, dest)?;
                "html"
            }
            _ => {
                export::write_markdown(&report, &ctx, dest)?;
                "markdown"
            }
        };
        println!(
            "{} {}",
            format!("{kind} report written to").dimmed(),
            dest.display()
        );
    }
    Ok(())
}
