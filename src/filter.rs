//! Noise filtering and path scoping for file-level metrics.
//!
//! Lockfiles, changelogs and generated code dominate raw churn lists and bury
//! the signal, so they are excluded by default (`--no-default-filters` opts out).

const LOCKFILES: &[&str] = &[
    "Cargo.lock",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lockb",
    "bun.lock",
    "poetry.lock",
    "uv.lock",
    "Pipfile.lock",
    "Gemfile.lock",
    "composer.lock",
    "go.sum",
    "flake.lock",
    "packages.lock.json",
    "mix.lock",
];

const NOISY_DIRS: &[&str] = &[
    "node_modules/",
    "vendor/",
    "dist/",
    "build/",
    "target/",
    "out/",
    ".idea/",
    ".vscode/",
    "__snapshots__/",
];

const NOISY_SUFFIXES: &[&str] = &[
    ".min.js",
    ".min.css",
    ".map",
    ".snap",
    ".pb.go",
    ".pb.cc",
    ".pb.h",
    "_pb2.py",
    ".generated.ts",
    ".generated.cs",
    ".g.dart",
];

/// Excludes commits by author — bot accounts and explicitly named users
/// (e.g. dependabot) otherwise dominate churn, ownership and velocity.
#[derive(Default)]
pub struct AuthorFilter {
    /// Lowercased author names to exclude.
    names: std::collections::HashSet<String>,
    exclude_bots: bool,
}

impl AuthorFilter {
    pub fn new(names: &[String], exclude_bots: bool) -> Self {
        Self {
            names: names.iter().map(|n| n.trim().to_lowercase()).collect(),
            exclude_bots,
        }
    }

    pub fn excluded(&self, author: &str) -> bool {
        let lower = author.to_lowercase();
        self.names.contains(&lower) || (self.exclude_bots && lower.ends_with("[bot]"))
    }

    /// Human-readable summary for report headers; None when inactive.
    pub fn describe(&self) -> Option<String> {
        let mut parts: Vec<String> = self.names.iter().cloned().collect();
        parts.sort();
        if self.exclude_bots {
            parts.push("*[bot]".to_string());
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        }
    }
}

pub struct PathFilter {
    /// Prefix relative to the repo root, always ending in '/', e.g. "src/".
    pub scope: Option<String>,
    pub use_default_filters: bool,
}

impl PathFilter {
    pub fn new(scope: Option<String>, use_default_filters: bool) -> Self {
        let scope = scope
            .map(|s| s.trim_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .map(|s| format!("{s}/"));
        Self {
            scope,
            use_default_filters,
        }
    }

    pub fn included(&self, path: &str) -> bool {
        if let Some(scope) = &self.scope
            && !path.starts_with(scope.as_str())
        {
            return false;
        }
        if self.use_default_filters && is_noise(path) {
            return false;
        }
        true
    }
}

fn is_noise(path: &str) -> bool {
    let basename = path.rsplit('/').next().unwrap_or(path);
    if LOCKFILES.contains(&basename) {
        return true;
    }
    if basename.to_ascii_uppercase().starts_with("CHANGELOG") {
        return true;
    }
    if NOISY_DIRS
        .iter()
        .any(|d| path.starts_with(d) || path.contains(&format!("/{d}")))
    {
        return true;
    }
    NOISY_SUFFIXES.iter().any(|s| path.ends_with(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_noise_by_default() {
        let f = PathFilter::new(None, true);
        assert!(!f.included("Cargo.lock"));
        assert!(!f.included("app/package-lock.json"));
        assert!(!f.included("CHANGELOG.md"));
        assert!(!f.included("web/dist/bundle.min.js"));
        assert!(!f.included("node_modules/lodash/index.js"));
        assert!(f.included("src/main.rs"));
    }

    #[test]
    fn scope_restricts_prefix() {
        let f = PathFilter::new(Some("src".into()), true);
        assert!(f.included("src/main.rs"));
        assert!(!f.included("docs/readme.md"));
        assert!(!f.included("srcs/decoy.rs"));
    }

    #[test]
    fn no_default_filters_keeps_lockfiles() {
        let f = PathFilter::new(None, false);
        assert!(f.included("Cargo.lock"));
    }

    #[test]
    fn author_filter_matches_names_case_insensitively() {
        let f = AuthorFilter::new(&["Alice".to_string()], false);
        assert!(f.excluded("alice"));
        assert!(f.excluded("ALICE"));
        assert!(!f.excluded("Bob"));
        assert!(!f.excluded("dependabot[bot]"));
    }

    #[test]
    fn author_filter_excludes_bots() {
        let f = AuthorFilter::new(&[], true);
        assert!(f.excluded("dependabot[bot]"));
        assert!(f.excluded("Renovate[Bot]"));
        assert!(!f.excluded("Bob"));
        assert!(!f.excluded("robotics-team"));
    }

    #[test]
    fn author_filter_default_excludes_nothing() {
        let f = AuthorFilter::default();
        assert!(!f.excluded("dependabot[bot]"));
        assert!(f.describe().is_none());
        assert_eq!(
            AuthorFilter::new(&["Bob".into()], true)
                .describe()
                .as_deref(),
            Some("bob, *[bot]")
        );
    }
}
