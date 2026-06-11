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
}
