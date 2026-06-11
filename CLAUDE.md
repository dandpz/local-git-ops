# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`local-git-ops` — offline Rust CLI that scans a local git repository via libgit2 (never shell commands, never network) and prints a terminal health dashboard: code churn, churn×size maintenance hotspots, bus factor, bug clusters, commit velocity, firefighting frequency. Optionally exports the same report as Markdown or self-contained HTML (`--export`, format by file extension).

## Commands

```sh
make check            # full CI gate: cargo fmt --check + clippy -D warnings + all tests
make test             # unit + integration tests
make lint             # clippy --all-targets -- -D warnings
make run ARGS="..."   # run against the current directory
cargo test --test pipeline                      # integration tests only
cargo test --test pipeline hostile              # single integration test by substring
cargo test metrics::tests::trend_sparse         # single unit test
```

CI (`.github/workflows/ci.yml`) has three jobs: lint (fmt + clippy, ubuntu only), test matrix (Linux + macOS, plus a release-profile build on ubuntu to catch LTO breakage), and MSRV (`rust-version` in Cargo.toml, currently 1.88 — floor set by let-chains in our code and comfy-table). Everything runs `--locked` — keep `Cargo.lock` committed. The release workflow builds 6 targets, tests each, then a single `publish` job uploads all artifacts plus a combined `SHA256SUMS` atomically (never let per-matrix-job release uploads back in — partial releases). Actions are pinned to commit SHAs (dependabot refreshes them); keep the `# vN` comment when bumping.

## Architecture

Lib + thin bin split: `src/lib.rs` exposes all modules; `src/main.rs` only parses CLI args and orchestrates. The split exists so `tests/pipeline.rs` can drive the pipeline against real temp repositories built with git2 (no shelling out in tests either).

Data flow is two-pass, dictated by git2's threading model (`git2::Repository` is not `Sync`):

1. `history.rs` — sequential `Revwalk` over the **entire** history collecting lightweight `CommitMeta` (author, time, summary, bugfix/firefight regex flags). Full history is required for velocity and bus-factor fidelity.
2. `history.rs` — rayon-parallel diff pass (`diff_tree_to_tree` vs first parent) restricted to the analysis window (default: last 100 non-merge commits). Each rayon worker opens its own `Repository` handle via `map_init` — this is the pattern for any new parallel git work; never share a handle across threads.
3. `loc.rs` — parallel line counts of HEAD-tree blobs (same `map_init` pattern; skips binary and >10 MB blobs).
4. `metrics.rs` — aggregates everything into a single plain `Report` struct. All three presenters (`render.rs` terminal, `export.rs` Markdown, `html.rs` HTML) consume `Report` and must stay feature-equivalent: a new metric lands in `metrics.rs` first, then every presenter. Export format is chosen in `main.rs` by file extension.

All analysis thresholds (bus-factor share, silo churn, hotspot score formula, firefight rates, trend classification) live as constants/functions in `metrics.rs` with the verdict logic.

## Security invariants

Commit messages, author names, and file paths are attacker-controlled (untrusted repos). Two rules:

- `sanitize::strip_controls` is applied **at ingestion** (`history.rs` for authors/summaries/diff paths, `loc.rs` for tree paths). Paths must be sanitized identically in both places or the churn↔line-count join silently breaks.
- `sanitize::markdown` / `sanitize::html` are applied at the export layer for every untrusted interpolation. Any new repo-derived string reaching output goes through these. (Unix filenames may legally contain `<`, `&`, quotes — HTML escaping of paths is not optional.)

`tests/pipeline.rs::hostile_metadata_is_sanitized` is the regression test for this.

## Gotchas

- git2 0.21 string accessors return `Result`, not `Option` (`commit.summary()` returns `Result<Option<&str>>`, `Signature::name()`/`shorthand()`/tree-entry `name()` return `Result<&str>`).
- git2 is built with `default-features = false` (no ssh/https transports) — intentional, keeps the tool offline and avoids the OpenSSL build. Don't re-enable.
- git itself rejects angle brackets in signature names, so test fixtures can't put `<` in an author name (`Signature::new` errors); the sanitizer still handles it for hand-crafted commits.
- File-level metrics exclude lockfiles/changelogs/vendored/generated paths by default (`filter.rs`) and auto-scope to the cwd when run below the repo root — keep this in mind when interpreting test expectations.
- Author exclusion (`--exclude-author`, `--exclude-bots`) is enforced inside the history walk (`WindowOpts::authors`), dropping commits before metas exist — excluded users vanish from every metric (churn, ownership, velocity, firefighting), not just the author table.
