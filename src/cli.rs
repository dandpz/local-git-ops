use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "local-git-ops",
    version,
    about = "Offline git repository health dashboard: churn, hotspots, bus factor, bug clusters, velocity.",
    long_about = "Scans the local git history (via libgit2, no shell commands, fully offline) and \
                  prints a terminal health dashboard revealing code churn, maintenance hotspots, \
                  knowledge silos, bug clusters, commit velocity and firefighting patterns."
)]
pub struct Cli {
    /// Number of recent non-merge commits to analyze for file-level metrics
    #[arg(short = 'n', long)]
    pub commits: Option<usize>,

    /// Restrict file-level metrics to commits from the last N days (uncapped unless -n is also given)
    #[arg(long)]
    pub days: Option<u32>,

    /// Only count paths under this prefix, relative to the repo root (e.g. "src")
    #[arg(long)]
    pub path: Option<String>,

    /// Don't automatically scope to the current subdirectory when run below the repo root
    #[arg(long)]
    pub no_auto_scope: bool,

    /// Include lockfiles, changelogs, vendored and generated files in file-level metrics
    #[arg(long)]
    pub no_default_filters: bool,

    /// Maximum rows per table
    #[arg(long, default_value_t = 20)]
    pub top: usize,

    /// Also write the report to a file; format chosen by extension
    /// (.html/.htm → HTML, anything else → Markdown)
    #[arg(long, value_name = "FILE")]
    pub export: Option<PathBuf>,

    /// Repository location; the repo root is discovered by walking upward
    #[arg(long, default_value = ".")]
    pub repo: PathBuf,
}

impl Cli {
    /// Effective commit-count cap for the analysis window.
    pub fn max_commits(&self) -> usize {
        match (self.commits, self.days) {
            (Some(n), _) => n,
            (None, Some(_)) => usize::MAX,
            (None, None) => 100,
        }
    }
}
