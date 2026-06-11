//! Current line counts for every text file in the HEAD tree.
//!
//! The tree walk is sequential (cheap); blob loading and line counting run in
//! parallel with one repository handle per rayon worker.

use anyhow::{Context, Result};
use git2::{ObjectType, Oid, Repository, TreeWalkMode, TreeWalkResult};
use rayon::prelude::*;
use std::collections::HashMap;

/// Blobs larger than this are skipped: they are never meaningful hotspot
/// candidates, and loading them fully would let a hostile repository exhaust
/// memory with a single huge text blob.
const MAX_BLOB_BYTES: u64 = 10 * 1024 * 1024;

pub fn head_line_counts(repo: &Repository) -> Result<HashMap<String, usize>> {
    let head_tree = repo
        .head()
        .context("failed to resolve HEAD")?
        .peel_to_tree()
        .context("HEAD does not point to a tree")?;

    let mut blobs: Vec<(String, Oid)> = Vec::new();
    head_tree.walk(TreeWalkMode::PreOrder, |dir, entry| {
        if entry.kind() == Some(ObjectType::Blob)
            && let Ok(name) = entry.name()
        {
            // Same control-character stripping as history::FileChange paths,
            // so the churn↔line-count join stays consistent.
            blobs.push((
                crate::sanitize::strip_controls(&format!("{dir}{name}")),
                entry.id(),
            ));
        }
        TreeWalkResult::Ok
    })?;

    let gitdir = repo.path().to_path_buf();
    let counts: Vec<Option<(String, usize)>> = blobs
        .par_iter()
        .map_init(
            || Repository::open(&gitdir),
            |repo, (path, oid)| {
                let repo = repo.as_ref().ok()?;
                let blob = repo.find_blob(*oid).ok()?;
                if blob.is_binary() || blob.size() as u64 > MAX_BLOB_BYTES {
                    return None;
                }
                Some((path.clone(), count_lines(blob.content())))
            },
        )
        .collect();

    Ok(counts.into_iter().flatten().collect())
}

fn count_lines(content: &[u8]) -> usize {
    let newlines = bytecount(content);
    if content.is_empty() {
        0
    } else if content.ends_with(b"\n") {
        newlines
    } else {
        newlines + 1
    }
}

fn bytecount(content: &[u8]) -> usize {
    content.iter().filter(|&&b| b == b'\n').count()
}

#[cfg(test)]
mod tests {
    use super::count_lines;

    #[test]
    fn counts_lines() {
        assert_eq!(count_lines(b""), 0);
        assert_eq!(count_lines(b"one\ntwo\n"), 2);
        assert_eq!(count_lines(b"one\ntwo"), 2);
    }
}
