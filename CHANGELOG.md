# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1](https://github.com/dandpz/local-git-ops/compare/v0.1.0...v0.1.1) - 2026-08-24

### Other

- *(deps)* update lockfile to fix RUSTSEC-2026-0204 ([#23](https://github.com/dandpz/local-git-ops/pull/23))
- credit the blog post that inspired the project ([#11](https://github.com/dandpz/local-git-ops/pull/11))

## [0.1.0]

### Added

- Offline git repository health dashboard built on libgit2 (`git2`) — no
  network, no shell commands. Reports code churn, churn×size maintenance
  hotspots, bus factor, bug clusters, commit velocity and firefighting
  frequency for a local repository.
- Analysis window controls (`-n`/`--commits`, `--days`) and path scoping
  (`--path`, auto-scope to the current subdirectory, `--exclude-path` globs).
- Author filtering (`--exclude-author`, `--exclude-bots`).
- Report export to Markdown or self-contained HTML (`--export`, format chosen
  by file extension).
