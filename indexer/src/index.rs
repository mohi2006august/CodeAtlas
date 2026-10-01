//! Indexing runs: full scans and git-diff-driven incremental updates (FR-07).
//!
//! In `auto` mode, if the index already covers a commit that still exists,
//! only files touched since then are examined: `git diff <indexed>`, untracked
//! files, and files that were dirty at the previous run. Content hashes then
//! decide what really needs re-parsing. Otherwise every file is scanned.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rayon::prelude::*;
use serde::Serialize;

use crate::extract::extract_file;
use crate::graph;
use crate::model::{hash_hex, FileIndex};
use crate::store::{ChangeSet, Store};
use crate::walk::{self, Filter, SkipReason};
use crate::git;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Mode {
    /// Incremental from the indexed commit when possible, otherwise a scan.
    Auto,
    /// Hash every file and update what changed.
    Scan,
    /// Drop the index and rebuild from scratch.
    Rebuild,
}

pub struct IndexOptions {
    pub root: PathBuf,
    pub db: PathBuf,
    pub mode: Mode,
    pub max_file_bytes: u64,
    pub excludes: Vec<String>,
    pub shard_depth: usize,
}

#[derive(Serialize, Debug, Default)]
pub struct IndexReport {
    pub mode: String,
    pub candidates: usize,
    pub parsed: usize,
    pub unchanged: usize,
    pub deleted: usize,
    pub fallback_files: usize,
    pub skipped: BTreeMap<String, usize>,
    pub symbols_added: usize,
    pub symbols_removed: usize,
    pub symbols_changed: usize,
    pub symbols_moved: usize,
    pub aliases: usize,
    pub files_resolved: usize,
    pub edges_written: usize,
    pub generation: i64,
    pub indexed_commit: Option<String>,
    pub dirty: bool,
    pub totals: BTreeMap<String, i64>,
    pub languages: BTreeMap<String, usize>,
    pub elapsed_ms: u128,
}

enum Outcome {
    Skip(SkipReason),
    Gone,
    Unchanged,
    Parsed(FileIndex),
}

/// Top `depth` directories of a path: the shard key for monorepos.
pub fn shard_of(path: &str, depth: usize) -> String {
    let dirs: Vec<&str> = path.split('/').collect();
    let dirs = &dirs[..dirs.len().saturating_sub(1)];
    dirs.iter().take(depth).copied().collect::<Vec<_>>().join("/")
}

fn examine(root: &Path, rel: &str, filter: &Filter, known_hash: Option<&String>, shard_depth: usize) -> Outcome {
    if let Some(reason) = filter.path_reason(rel) {
        return Outcome::Skip(reason);
    }
    let full = root.join(rel);
    let Ok(meta) = std::fs::symlink_metadata(&full) else { return Outcome::Gone };
    if !meta.is_file() {
        return Outcome::Gone;
    }
    let Ok(bytes) = std::fs::read(&full) else { return Outcome::Gone };
    if let Some(reason) = filter.content_reason(&bytes) {
        return Outcome::Skip(reason);
    }
    if known_hash == Some(&hash_hex(&bytes)) {
        return Outcome::Unchanged;
    }
    match extract_file(rel, &bytes) {
        Some(mut fi) => {
            fi.shard = shard_of(rel, shard_depth);
            Outcome::Parsed(fi)
        }
        None => Outcome::Skip(SkipReason::Unsupported),
    }
}

pub fn run(opts: &IndexOptions) -> Result<IndexReport> {
    let started = Instant::now();
    let root = opts.root.canonicalize().with_context(|| format!("repo {}", opts.root.display()))?;
    let store = Store::open(&opts.db)?;
    if opts.mode == Mode::Rebuild {
        store.reset()?;
    }
    let filter = Filter::new(&root, &opts.excludes, opts.max_file_bytes)?;
    let is_git = git::is_repo(&root);
    let head = if is_git { git::head(&root) } else { None };
    let known = store.file_hashes()?;
    let prev_commit = store.meta_get("indexed_commit")?.filter(|c| !c.is_empty());
    let prev_dirty: Vec<String> = store
        .meta_get("dirty_paths")?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let mut report = IndexReport::default();
    let incremental = opts.mode == Mode::Auto
        && is_git
        && !known.is_empty()
        && prev_commit.as_deref().is_some_and(|c| git::commit_exists(&root, c));
    let mut candidates: BTreeSet<String> = BTreeSet::new();
    if incremental {
        report.mode = "git-diff".to_string();
        candidates.extend(git::changed_since(&root, prev_commit.as_deref().unwrap_or_default())?);
        candidates.extend(prev_dirty);
    } else {
        report.mode = if opts.mode == Mode::Rebuild { "rebuild" } else { "scan" }.to_string();
        candidates.extend(walk::list_files(&root)?);
        candidates.extend(known.keys().cloned());
    }
    report.candidates = candidates.len();

    let candidates: Vec<String> = candidates.into_iter().collect();
    let outcomes: Vec<(String, Outcome)> = candidates
        .par_iter()
        .map(|rel| (rel.clone(), examine(&root, rel, &filter, known.get(rel), opts.shard_depth)))
        .collect();

    let gen = store.generation()? + 1;
    let mut changes = ChangeSet::default();
    store.conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| -> Result<()> {
        for (rel, outcome) in &outcomes {
            match outcome {
                Outcome::Unchanged => report.unchanged += 1,
                Outcome::Parsed(fi) => {
                    store.apply_file(gen, fi, &mut changes)?;
                    report.parsed += 1;
                    report.fallback_files += fi.fallback as usize;
                    *report.languages.entry(fi.lang.clone()).or_default() += 1;
                }
                Outcome::Skip(reason) => {
                    *report.skipped.entry(reason.as_str().to_string()).or_default() += 1;
                    if known.contains_key(rel) {
                        store.delete_file(gen, rel, &mut changes)?;
                        report.deleted += 1;
                    }
                }
                Outcome::Gone => {
                    if known.contains_key(rel) {
                        store.delete_file(gen, rel, &mut changes)?;
                        report.deleted += 1;
                    }
                }
            }
        }
        report.aliases = store.record_aliases(gen, &changes)?;

        // Re-resolve the changed files and every file whose refs could now
        // resolve differently because a symbol with that name came or went.
        let mut affected: HashSet<String> = changes.files.clone();
        affected.extend(store.files_referencing(&changes.keys)?);
        let mut affected: Vec<String> = affected.into_iter().collect();
        affected.sort();
        let stats = graph::resolve_files(&store, &affected)?;
        store.conn.execute("DELETE FROM edges WHERE dst NOT IN (SELECT id FROM symbols)", [])?;
        report.files_resolved = stats.files;
        report.edges_written = stats.edges;

        let changed = report.parsed > 0 || report.deleted > 0;
        let generation = if changed { gen } else { gen - 1 };
        store.meta_set("generation", &generation.to_string())?;
        report.generation = generation;
        let dirty_paths = if is_git { git::dirty(&root)? } else { Vec::new() };
        report.dirty = !dirty_paths.is_empty();
        store.meta_set("indexed_commit", head.as_deref().unwrap_or(""))?;
        store.meta_set("dirty", if report.dirty { "1" } else { "0" })?;
        store.meta_set("dirty_paths", &serde_json::to_string(&dirty_paths)?)?;
        store.meta_set("repo_root", &root.to_string_lossy())?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        store.meta_set("indexed_at", &now.to_string())?;
        Ok(())
    })();
    match result {
        Ok(()) => store.conn.execute_batch("COMMIT")?,
        Err(e) => {
            store.conn.execute_batch("ROLLBACK")?;
            return Err(e);
        }
    }

    report.symbols_added = changes.added.len();
    report.symbols_removed = changes.removed.len();
    report.symbols_changed = changes.changed;
    report.symbols_moved = changes.moved;
    report.indexed_commit = head;
    for table in ["files", "symbols", "edges", "refs", "aliases"] {
        report.totals.insert(table.to_string(), store.count(table)?);
    }
    report.elapsed_ms = started.elapsed().as_millis();
    Ok(report)
}
