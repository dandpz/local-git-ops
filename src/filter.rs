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
    /// User-supplied exclusion patterns, compiled from globs.
    excludes: Vec<regex::Regex>,
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
            excludes: Vec::new(),
        }
    }

    /// Add user exclusion patterns. Globs: `*` matches any run, `?` one
    /// character; a trailing `/` matches the whole subtree (`docs/`).
    pub fn with_excludes(mut self, patterns: &[String]) -> anyhow::Result<Self> {
        for pattern in patterns {
            let pattern = pattern.trim_start_matches("./");
            self.excludes.push(
                glob_to_regex(pattern)
                    .map_err(|e| anyhow::anyhow!("invalid --exclude-path '{pattern}': {e}"))?,
            );
        }
        Ok(self)
    }

    pub fn included(&self, path: &str) -> bool {
        if let Some(scope) = &self.scope
            && !path.starts_with(scope.as_str())
        {
            return false;
        }
        if self.excludes.iter().any(|re| re.is_match(path)) {
            return false;
        }
        if self.use_default_filters && is_noise(path) {
            return false;
        }
        true
    }

    /// True when the user narrowed the path set (scope or explicit excludes);
    /// ownership is then computed from the included paths only. The default
    /// noise filters alone don't count — they must not change the full-history
    /// bus-factor semantics.
    pub fn is_restrictive(&self) -> bool {
        self.scope.is_some() || !self.excludes.is_empty()
    }
}

fn glob_to_regex(pattern: &str) -> Result<regex::Regex, regex::Error> {
    let mut re = String::from("^");
    for c in pattern.chars() {
        match c {
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            c => re.push_str(&regex::escape(&c.to_string())),
        }
    }
    if !pattern.ends_with('/') {
        re.push('$');
    }
    regex::Regex::new(&re)
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
    fn exclude_patterns_drop_paths() {
        let f = PathFilter::new(None, true)
            .with_excludes(&["docs/".into(), "*.md".into(), "src/generated.rs".into()])
            .unwrap();
        assert!(!f.included("docs/guide.txt"));
        assert!(!f.included("README.md"));
        assert!(!f.included("nested/notes.md"));
        assert!(!f.included("src/generated.rs"));
        assert!(f.included("src/main.rs"));
        // Exact-match pattern must not act as a prefix.
        assert!(f.included("src/generated.rs.bak"));
    }

    #[test]
    fn restrictive_only_with_scope_or_excludes() {
        assert!(!PathFilter::new(None, true).is_restrictive());
        assert!(PathFilter::new(Some("src".into()), true).is_restrictive());
        assert!(
            PathFilter::new(None, true)
                .with_excludes(&["*.md".into()])
                .unwrap()
                .is_restrictive()
        );
    }

    #[test]
    fn invalid_exclude_pattern_is_rejected_gracefully() {
        // `(` is regex-escaped, so weird-but-valid filenames work as literals.
        let f = PathFilter::new(None, true)
            .with_excludes(&["weird(name.rs".into()])
            .unwrap();
        assert!(!f.included("weird(name.rs"));
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
