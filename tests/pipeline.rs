//! End-to-end pipeline test against a real repository built with git2.

use git2::{IndexEntry, IndexTime, Oid, Repository, Signature, Time};
use local_git_ops::filter::PathFilter;
use local_git_ops::{export, history, html, loc, metrics, render};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

const DAY: i64 = 86_400;

fn commit(
    repo: &Repository,
    author: (&str, &str),
    secs: i64,
    msg: &str,
    file: &str,
    content: &str,
) {
    // Stage the blob straight into the index from a buffer rather than writing
    // it to the working directory. The analysis only ever reads the git tree,
    // and this keeps paths like "src/<script>.rs" — legal on Unix, rejected by
    // the Windows filesystem — testable on every platform.
    let mut index = repo.index().unwrap();
    let entry = IndexEntry {
        ctime: IndexTime::new(0, 0),
        mtime: IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode: 0o100_644,
        uid: 0,
        gid: 0,
        file_size: 0,
        id: Oid::ZERO_SHA1,
        flags: 0,
        flags_extended: 0,
        path: file.as_bytes().to_vec(),
    };
    index.add_frombuffer(&entry, content.as_bytes()).unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();

    let sig = Signature::new(author.0, author.1, &Time::new(secs, 0)).unwrap();
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &parents)
        .unwrap();
}

#[test]
fn full_pipeline_on_synthetic_repo() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let alice = ("Alice", "alice@example.com");
    let bob = ("Bob", "bob@example.com");

    commit(
        &repo,
        alice,
        now - 10 * DAY,
        "initial commit",
        "src/core.rs",
        "fn a() {}\n",
    );
    commit(
        &repo,
        alice,
        now - 8 * DAY,
        "fix bug in core",
        "src/core.rs",
        "fn a() {}\nfn b() {}\n",
    );
    commit(
        &repo,
        bob,
        now - 5 * DAY,
        "add api",
        "src/api.rs",
        "fn api() {}\n",
    );
    commit(
        &repo,
        bob,
        now - 2 * DAY,
        "Revert \"add api\"",
        "src/api.rs",
        "// reverted\n",
    );
    commit(
        &repo,
        alice,
        now - DAY,
        "bump lockfile",
        "Cargo.lock",
        "locked\n",
    );

    let window = history::WindowOpts {
        max_commits: 100,
        days: None,
        now,
        authors: Default::default(),
    };
    let hist = history::collect(&repo, &window).unwrap();
    assert_eq!(hist.metas.len(), 5);
    assert_eq!(hist.window_commits, 5);

    let line_counts = loc::head_line_counts(&repo).unwrap();
    assert_eq!(line_counts.get("src/core.rs"), Some(&2));

    let filter = PathFilter::new(None, true);
    let report = metrics::Report::compute(&hist, &line_counts, &filter, now);

    // Lockfile churn is filtered out of file-level metrics.
    assert!(report.files.iter().all(|f| f.path != "Cargo.lock"));

    let core = report
        .files
        .iter()
        .find(|f| f.path == "src/core.rs")
        .expect("core.rs tracked");
    assert_eq!(core.churn, 2);
    assert_eq!(core.bug_fixes, 1);
    assert_eq!(core.authors, vec!["Alice".to_string()]);
    assert!(
        core.score > 0.0,
        "churn ≥ 2 and present at HEAD → hotspot score"
    );

    assert_eq!(report.total_commits, 5);
    assert_eq!(report.authors[0].name, "Alice");
    assert_eq!(report.authors[0].commits, 3);
    assert_eq!(report.firefight.count_last_year, 1);
    assert_eq!(
        report.firefight.recent,
        vec!["Revert \"add api\"".to_string()]
    );
}

#[test]
fn window_cap_and_scope() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let alice = ("Alice", "alice@example.com");

    for i in 0..6 {
        commit(
            &repo,
            alice,
            now - (10 - i) * DAY,
            &format!("change {i}"),
            "app/main.rs",
            &format!("// rev {i}\n"),
        );
        commit(
            &repo,
            alice,
            now - (10 - i) * DAY + 100,
            &format!("docs {i}"),
            "docs/guide.md",
            &format!("rev {i}\n"),
        );
    }

    // Cap the window to the 4 most recent commits.
    let hist = history::collect(
        &repo,
        &history::WindowOpts {
            max_commits: 4,
            days: None,
            now,
            authors: Default::default(),
        },
    )
    .unwrap();
    assert_eq!(hist.window_commits, 4);
    assert_eq!(hist.metas.len(), 12);

    // Scope to app/ — docs churn must disappear.
    let line_counts = loc::head_line_counts(&repo).unwrap();
    let scoped = PathFilter::new(Some("app".into()), true);
    let report = metrics::Report::compute(&hist, &line_counts, &scoped, now);
    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].path, "app/main.rs");
}

#[test]
fn markdown_export_writes_report() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    commit(
        &repo,
        ("Alice", "alice@example.com"),
        now - DAY,
        "initial commit",
        "src/main.rs",
        "fn main() {}\n",
    );

    let hist = history::collect(
        &repo,
        &history::WindowOpts {
            max_commits: 100,
            days: None,
            now,
            authors: Default::default(),
        },
    )
    .unwrap();
    let line_counts = loc::head_line_counts(&repo).unwrap();
    let report = metrics::Report::compute(&hist, &line_counts, &PathFilter::new(None, true), now);

    let ctx = render::Context {
        repo_root: "/tmp/example",
        branch: "main",
        scope: None,
        excluded_authors: None,
        window_desc: "last 1 non-merge commits",
        top: 20,
    };
    let dest = dir.path().join("report.md");
    export::write_markdown(&report, &ctx, &dest).unwrap();

    let md = fs::read_to_string(&dest).unwrap();
    assert!(md.starts_with("# Repository Health Report"));
    assert!(md.contains("## 👥 Ownership & Bus Factor"));
    assert!(md.contains("| Alice | 1 | 100% |"));
}

#[test]
fn hostile_metadata_is_sanitized() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    // ANSI color + OSC title escape in the message, table-breaking pipe and
    // code-span-breaking backtick in the author name (angle brackets are
    // already rejected by git signature parsing itself).
    let evil = ("Mallory|`tick", "m@example.com");
    commit(
        &repo,
        evil,
        now - 2 * DAY,
        "initial commit",
        "src/a.rs",
        "fn a() {}\n",
    );
    commit(
        &repo,
        evil,
        now - DAY,
        "Revert \"\u{1b}[31mevil\u{1b}]0;pwned\u{7}\"",
        "src/a.rs",
        "// reverted\n",
    );

    let hist = history::collect(
        &repo,
        &history::WindowOpts {
            max_commits: 100,
            days: None,
            now,
            authors: Default::default(),
        },
    )
    .unwrap();

    // Control characters never survive ingestion.
    for meta in &hist.metas {
        assert!(
            !meta.summary.chars().any(char::is_control),
            "summary: {:?}",
            meta.summary
        );
        assert!(!meta.author.chars().any(char::is_control));
    }
    assert_eq!(hist.metas[0].summary, "Revert \"[31mevil]0;pwned\"");

    // Markdown export escapes table/HTML metacharacters.
    let line_counts = loc::head_line_counts(&repo).unwrap();
    let report = metrics::Report::compute(&hist, &line_counts, &PathFilter::new(None, true), now);
    let ctx = render::Context {
        repo_root: "/tmp/example",
        branch: "main",
        scope: None,
        excluded_authors: None,
        window_desc: "last 2 non-merge commits",
        top: 20,
    };
    let dest = dir.path().join("report.md");
    export::write_markdown(&report, &ctx, &dest).unwrap();
    let md = fs::read_to_string(&dest).unwrap();
    assert!(md.contains("Mallory\\|'tick"), "author escaped in export");
    assert!(!md.contains('\u{1b}'));
}

#[test]
fn html_export_writes_escaped_report() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    // Unix filenames may legally contain HTML metacharacters.
    let hostile_file = "src/<script>alert(1)<.rs";
    let alice = ("Alice & Co", "alice@example.com");
    commit(
        &repo,
        alice,
        now - 3 * DAY,
        "initial commit",
        hostile_file,
        "fn a() {}\n",
    );
    commit(
        &repo,
        alice,
        now - 2 * DAY,
        "fix bug here",
        hostile_file,
        "fn a() {}\nfn b() {}\n",
    );
    commit(
        &repo,
        alice,
        now - DAY,
        "more work",
        hostile_file,
        "fn a() {}\nfn b() {}\nfn c() {}\n",
    );

    let hist = history::collect(
        &repo,
        &history::WindowOpts {
            max_commits: 100,
            days: None,
            now,
            authors: Default::default(),
        },
    )
    .unwrap();
    let line_counts = loc::head_line_counts(&repo).unwrap();
    let report = metrics::Report::compute(&hist, &line_counts, &PathFilter::new(None, true), now);

    let ctx = render::Context {
        repo_root: "/tmp/example",
        branch: "main",
        scope: None,
        excluded_authors: None,
        window_desc: "last 3 non-merge commits",
        top: 20,
    };
    let dest = dir.path().join("report.html");
    html::write_html(&report, &ctx, &dest).unwrap();

    let out = fs::read_to_string(&dest).unwrap();
    assert!(out.starts_with("<!DOCTYPE html>"));
    assert!(out.contains("<h2>👥 Ownership &amp; Bus Factor"));
    // Hostile path and author are escaped, never raw.
    assert!(!out.contains("<script>"));
    assert!(out.contains("src/&lt;script&gt;alert(1)&lt;.rs"));
    assert!(out.contains("Alice &amp; Co"));
    // Velocity bar rendered.
    assert!(out.contains("class=\"bar\""));
}

#[test]
fn excluded_authors_vanish_from_all_metrics() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let alice = ("Alice", "alice@example.com");
    let bot = ("dependabot[bot]", "support@github.com");
    let mallory = ("Mallory", "m@example.com");

    commit(
        &repo,
        alice,
        now - 6 * DAY,
        "initial commit",
        "src/core.rs",
        "fn a() {}\n",
    );
    commit(
        &repo,
        alice,
        now - 5 * DAY,
        "more core",
        "src/core.rs",
        "fn a() {}\nfn b() {}\n",
    );
    // Bot churns a non-lockfile path so only the author filter can drop it.
    commit(
        &repo,
        bot,
        now - 4 * DAY,
        "bump deps",
        "deps/manifest.txt",
        "v1\n",
    );
    commit(
        &repo,
        bot,
        now - 3 * DAY,
        "bump deps again",
        "deps/manifest.txt",
        "v2\n",
    );
    commit(
        &repo,
        mallory,
        now - 2 * DAY,
        "noise",
        "src/extra.rs",
        "fn x() {}\n",
    );

    let hist = history::collect(
        &repo,
        &history::WindowOpts {
            max_commits: 100,
            days: None,
            now,
            authors: local_git_ops::filter::AuthorFilter::new(&["mallory".to_string()], true),
        },
    )
    .unwrap();

    // Bot + Mallory commits dropped from history entirely.
    assert_eq!(hist.metas.len(), 2);
    assert!(hist.metas.iter().all(|m| m.author == "Alice"));

    let line_counts = loc::head_line_counts(&repo).unwrap();
    let report = metrics::Report::compute(&hist, &line_counts, &PathFilter::new(None, true), now);

    assert_eq!(report.authors.len(), 1);
    assert_eq!(report.authors[0].name, "Alice");
    assert!(report.files.iter().all(|f| !f.path.starts_with("deps/")));
    assert!(report.files.iter().all(|f| f.path != "src/extra.rs"));
    // Velocity counts only the remaining commits.
    assert_eq!(report.velocity.months.values().sum::<u32>(), 2);
}

#[test]
fn exclude_paths_and_scoped_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let alice = ("Alice", "alice@example.com");
    let bob = ("Bob", "bob@example.com");

    // Alice owns src/, Bob only ever touches docs and markdown.
    commit(
        &repo,
        alice,
        now - 6 * DAY,
        "core work",
        "src/core.rs",
        "fn a() {}\n",
    );
    commit(
        &repo,
        alice,
        now - 5 * DAY,
        "more core",
        "src/core.rs",
        "fn a() {}\nfn b() {}\n",
    );
    commit(
        &repo,
        bob,
        now - 4 * DAY,
        "write docs",
        "docs/guide.txt",
        "guide\n",
    );
    commit(&repo, bob, now - 3 * DAY, "notes", "NOTES.md", "notes\n");

    let hist = history::collect(
        &repo,
        &history::WindowOpts {
            max_commits: 100,
            days: None,
            now,
            authors: Default::default(),
        },
    )
    .unwrap();
    let line_counts = loc::head_line_counts(&repo).unwrap();

    // --exclude-path docs/ + *.md: Bob's files drop out of file metrics AND
    // Bob drops out of ownership, because the path set is user-narrowed.
    let filter = PathFilter::new(None, true)
        .with_excludes(&["docs/".to_string(), "*.md".to_string()])
        .unwrap();
    let report = metrics::Report::compute(&hist, &line_counts, &filter, now);

    assert!(report.files.iter().all(|f| f.path.starts_with("src/")));
    assert!(report.ownership_scoped);
    assert_eq!(report.authors.len(), 1);
    assert_eq!(report.authors[0].name, "Alice");
    assert_eq!(report.authors[0].commits, 2);

    // Without user narrowing, ownership keeps full-history semantics.
    let report = metrics::Report::compute(&hist, &line_counts, &PathFilter::new(None, true), now);
    assert!(!report.ownership_scoped);
    assert_eq!(report.authors.len(), 2);
}
