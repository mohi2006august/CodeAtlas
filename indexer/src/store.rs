//! SQLite store for symbols, raw references and graph edges.
//!
//! The Rust indexer owns `files`, `symbols`, `refs`, `edges`, `tombstones`
//! and `aliases`. The Python side reads them and fills `symbols.tokens`.
//! Every change bumps `symbols.gen` (or writes a tombstone) with the run's
//! generation so downstream indexes can sync incrementally.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::model::{norm, FileIndex, RawRef, RefKind, Symbol};

pub const SCHEMA_VERSION: &str = "1";

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS files (
    path         TEXT PRIMARY KEY,
    lang         TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    size         INTEGER NOT NULL,
    lines        INTEGER NOT NULL,
    fallback     INTEGER NOT NULL,
    is_test      INTEGER NOT NULL,
    shard        TEXT NOT NULL,
    gen          INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS symbols (
    id         TEXT PRIMARY KEY,
    kind       TEXT NOT NULL,
    lang       TEXT NOT NULL,
    path       TEXT NOT NULL,
    name       TEXT NOT NULL,
    norm       TEXT NOT NULL,
    pkg        TEXT NOT NULL,
    qualname   TEXT NOT NULL,
    container  TEXT NOT NULL,
    parent     TEXT,
    start_line INTEGER NOT NULL,
    end_line   INTEGER NOT NULL,
    def_line   INTEGER NOT NULL,
    signature  TEXT NOT NULL,
    doc        TEXT,
    body_hash  TEXT NOT NULL,
    tokens     INTEGER NOT NULL DEFAULT 0,
    is_test    INTEGER NOT NULL,
    fallback   INTEGER NOT NULL,
    shard      TEXT NOT NULL,
    gen        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS symbols_path ON symbols(path);
CREATE INDEX IF NOT EXISTS symbols_name ON symbols(name);
CREATE INDEX IF NOT EXISTS symbols_norm ON symbols(norm);
CREATE INDEX IF NOT EXISTS symbols_pkg ON symbols(pkg) WHERE kind = 'module';
CREATE INDEX IF NOT EXISTS symbols_gen ON symbols(gen);
CREATE INDEX IF NOT EXISTS symbols_parent ON symbols(parent);
CREATE TABLE IF NOT EXISTS refs (
    src       TEXT NOT NULL,
    path      TEXT NOT NULL,
    name      TEXT NOT NULL,
    qualifier TEXT NOT NULL,
    kind      TEXT NOT NULL,
    line      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS refs_path ON refs(path);
CREATE INDEX IF NOT EXISTS refs_name ON refs(name);
CREATE TABLE IF NOT EXISTS edges (
    src      TEXT NOT NULL,
    dst      TEXT NOT NULL,
    type     TEXT NOT NULL,
    weight   REAL NOT NULL,
    src_path TEXT NOT NULL,
    PRIMARY KEY (src, dst, type)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS edges_dst ON edges(dst, type);
CREATE INDEX IF NOT EXISTS edges_src_path ON edges(src_path);
CREATE TABLE IF NOT EXISTS tombstones (
    id  TEXT PRIMARY KEY,
    gen INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS aliases (
    old_id TEXT PRIMARY KEY,
    new_id TEXT NOT NULL,
    gen    INTEGER NOT NULL
);
"#;

/// What changed in one indexing run; drives graph re-resolution.
#[derive(Default, Debug)]
pub struct ChangeSet {
    /// Lookup keys (names, normalised names, package dirs) whose candidate sets changed.
    pub keys: HashSet<String>,
    /// Paths that were re-parsed or deleted.
    pub files: HashSet<String>,
    pub added: Vec<SymRef>,
    pub removed: Vec<SymRef>,
    pub changed: usize,
    pub moved: usize,
}

#[derive(Debug, Clone)]
pub struct SymRef {
    pub id: String,
    pub kind: String,
    pub body_hash: String,
    pub lines: u32,
}

struct OldSym {
    kind: String,
    name: String,
    pkg: String,
    container: String,
    signature: String,
    doc: Option<String>,
    body_hash: String,
    is_test: bool,
    fallback: bool,
    start_line: u32,
    end_line: u32,
    def_line: u32,
    parent: Option<String>,
}

pub struct Store {
    pub conn: Connection,
}

/// Lowercased name of the directory that contains `path` (package key for dir-imports).
pub fn pkg_of(path: &str) -> String {
    let mut parts = path.rsplit('/');
    parts.next();
    parts.next().map(norm).unwrap_or_default()
}

fn add_keys(keys: &mut HashSet<String>, name: &str, kind: &str, pkg: &str) {
    keys.insert(name.to_string());
    keys.insert(norm(name));
    if kind == "module" && !pkg.is_empty() {
        keys.insert(pkg.to_string());
    }
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(30))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        let store = Store { conn };
        if store.meta_get("schema_version").ok().flatten().as_deref() != Some(SCHEMA_VERSION) {
            store.reset()?;
        }
        Ok(store)
    }

    /// Drop everything and recreate an empty index.
    pub fn reset(&self) -> Result<()> {
        self.conn.execute_batch(
            "DROP TABLE IF EXISTS meta; DROP TABLE IF EXISTS files; DROP TABLE IF EXISTS symbols;
             DROP TABLE IF EXISTS refs; DROP TABLE IF EXISTS edges; DROP TABLE IF EXISTS tombstones;
             DROP TABLE IF EXISTS aliases;",
        )?;
        self.conn.execute_batch(SCHEMA)?;
        self.meta_set("schema_version", SCHEMA_VERSION)?;
        self.meta_set("generation", "0")?;
        Ok(())
    }

    pub fn meta_get(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn meta_set(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn generation(&self) -> Result<i64> {
        Ok(self.meta_get("generation")?.and_then(|g| g.parse().ok()).unwrap_or(0))
    }

    pub fn file_hashes(&self) -> Result<HashMap<String, String>> {
        let mut stmt = self.conn.prepare("SELECT path, content_hash FROM files")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn old_symbols(&self, path: &str) -> Result<HashMap<String, OldSym>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, kind, name, pkg, container, signature, doc, body_hash, is_test, fallback,
                    start_line, end_line, def_line, parent
             FROM symbols WHERE path = ?1",
        )?;
        let rows = stmt.query_map([path], |r| {
            Ok((
                r.get::<_, String>(0)?,
                OldSym {
                    kind: r.get(1)?,
                    name: r.get(2)?,
                    pkg: r.get(3)?,
                    container: r.get(4)?,
                    signature: r.get(5)?,
                    doc: r.get(6)?,
                    body_hash: r.get(7)?,
                    is_test: r.get(8)?,
                    fallback: r.get(9)?,
                    start_line: r.get(10)?,
                    end_line: r.get(11)?,
                    def_line: r.get(12)?,
                    parent: r.get(13)?,
                },
            ))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn insert_symbol(&self, s: &Symbol, shard: &str, gen: i64) -> Result<()> {
        let pkg = if s.kind == "module" { pkg_of(&s.path) } else { String::new() };
        self.conn
            .prepare_cached(
                "INSERT OR REPLACE INTO symbols (id, kind, lang, path, name, norm, pkg, qualname, container,
                    parent, start_line, end_line, def_line, signature, doc, body_hash, tokens, is_test,
                    fallback, shard, gen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, 0, ?17,
                    ?18, ?19, ?20)",
            )?
            .execute(params![
                s.id, s.kind, s.lang, s.path, s.name, norm(&s.name), pkg, s.qualname, s.container,
                s.parent, s.start_line, s.end_line, s.def_line, s.signature, s.doc, s.body_hash,
                s.is_test, s.fallback, shard, gen
            ])?;
        self.conn.prepare_cached("DELETE FROM tombstones WHERE id = ?1")?.execute([&s.id])?;
        Ok(())
    }

    fn remove_symbol(&self, id: &str, gen: i64) -> Result<()> {
        self.conn.prepare_cached("DELETE FROM symbols WHERE id = ?1")?.execute([id])?;
        self.conn
            .prepare_cached("INSERT OR REPLACE INTO tombstones(id, gen) VALUES (?1, ?2)")?
            .execute(params![id, gen])?;
        Ok(())
    }

    /// Replace one file's symbols and refs, recording what changed.
    pub fn apply_file(&self, gen: i64, fi: &FileIndex, changes: &mut ChangeSet) -> Result<()> {
        let mut old = self.old_symbols(&fi.path)?;
        for s in &fi.symbols {
            let pkg = if s.kind == "module" { pkg_of(&s.path) } else { String::new() };
            let lines = s.end_line.saturating_sub(s.start_line) + 1;
            match old.remove(&s.id) {
                None => {
                    self.insert_symbol(s, &fi.shard, gen)?;
                    add_keys(&mut changes.keys, &s.name, &s.kind, &pkg);
                    changes.added.push(SymRef {
                        id: s.id.clone(),
                        kind: s.kind.clone(),
                        body_hash: s.body_hash.clone(),
                        lines,
                    });
                }
                Some(o) => {
                    let resolution_inputs_changed =
                        o.kind != s.kind || o.is_test != s.is_test || o.container != s.container;
                    let content_changed = resolution_inputs_changed
                        || o.body_hash != s.body_hash
                        || o.signature != s.signature
                        || o.doc != s.doc
                        || o.fallback != s.fallback;
                    if content_changed {
                        self.insert_symbol(s, &fi.shard, gen)?;
                        changes.changed += 1;
                        if resolution_inputs_changed {
                            add_keys(&mut changes.keys, &s.name, &s.kind, &pkg);
                            add_keys(&mut changes.keys, &o.name, &o.kind, &o.pkg);
                        }
                    } else if o.start_line != s.start_line
                        || o.end_line != s.end_line
                        || o.def_line != s.def_line
                        || o.parent != s.parent
                    {
                        self.conn
                            .prepare_cached(
                                "UPDATE symbols SET start_line = ?2, end_line = ?3, def_line = ?4, parent = ?5
                                 WHERE id = ?1",
                            )?
                            .execute(params![s.id, s.start_line, s.end_line, s.def_line, s.parent])?;
                        changes.moved += 1;
                    }
                }
            }
        }
        for (id, o) in old {
            self.remove_symbol(&id, gen)?;
            add_keys(&mut changes.keys, &o.name, &o.kind, &o.pkg);
            changes.removed.push(SymRef {
                id,
                kind: o.kind,
                body_hash: o.body_hash,
                lines: o.end_line.saturating_sub(o.start_line) + 1,
            });
        }
        self.conn
            .prepare_cached(
                "INSERT OR REPLACE INTO files (path, lang, content_hash, size, lines, fallback, is_test, shard, gen)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?
            .execute(params![
                fi.path, fi.lang, fi.content_hash, fi.size, fi.lines, fi.fallback, fi.is_test, fi.shard, gen
            ])?;
        self.conn.prepare_cached("DELETE FROM refs WHERE path = ?1")?.execute([&fi.path])?;
        let mut insert = self.conn.prepare_cached(
            "INSERT INTO refs (src, path, name, qualifier, kind, line) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for r in &fi.refs {
            insert.execute(params![r.src, fi.path, r.name, r.qualifier, r.kind.as_str(), r.line])?;
        }
        changes.files.insert(fi.path.clone());
        Ok(())
    }

    pub fn delete_file(&self, gen: i64, path: &str, changes: &mut ChangeSet) -> Result<()> {
        for (id, o) in self.old_symbols(path)? {
            self.remove_symbol(&id, gen)?;
            add_keys(&mut changes.keys, &o.name, &o.kind, &o.pkg);
            changes.removed.push(SymRef {
                id,
                kind: o.kind,
                body_hash: o.body_hash,
                lines: o.end_line.saturating_sub(o.start_line) + 1,
            });
        }
        self.conn.prepare_cached("DELETE FROM refs WHERE path = ?1")?.execute([path])?;
        self.conn.prepare_cached("DELETE FROM files WHERE path = ?1")?.execute([path])?;
        self.conn.prepare_cached("DELETE FROM edges WHERE src_path = ?1")?.execute([path])?;
        changes.files.insert(path.to_string());
        Ok(())
    }

    /// Rename detection: a symbol that disappeared and one that appeared with
    /// the same body and kind are the same code moved or renamed. Old ids keep
    /// resolving through `aliases`.
    pub fn record_aliases(&self, gen: i64, changes: &ChangeSet) -> Result<usize> {
        let group = |syms: &[SymRef]| {
            let mut map: HashMap<(String, String), Vec<String>> = HashMap::new();
            for s in syms.iter().filter(|s| s.kind != "module" && s.kind != "chunk" && s.lines >= 2) {
                map.entry((s.body_hash.clone(), s.kind.clone())).or_default().push(s.id.clone());
            }
            map
        };
        let removed = group(&changes.removed);
        let added = group(&changes.added);
        let mut count = 0;
        for (key, olds) in &removed {
            let (Some(news), [old]) = (added.get(key), olds.as_slice()) else { continue };
            let [new] = news.as_slice() else { continue };
            self.conn.execute("UPDATE aliases SET new_id = ?1, gen = ?2 WHERE new_id = ?3", params![new, gen, old])?;
            self.conn.execute(
                "INSERT OR REPLACE INTO aliases(old_id, new_id, gen) VALUES (?1, ?2, ?3)",
                params![old, new, gen],
            )?;
            count += 1;
        }
        self.conn.execute_batch(
            "DELETE FROM aliases WHERE old_id IN (SELECT id FROM symbols);
             DELETE FROM aliases WHERE new_id NOT IN (SELECT id FROM symbols);",
        )?;
        Ok(count)
    }

    /// Paths of files holding refs whose lookup key is in `keys`.
    pub fn files_referencing(&self, keys: &HashSet<String>) -> Result<HashSet<String>> {
        let mut out = HashSet::new();
        let mut stmt = self.conn.prepare_cached("SELECT DISTINCT path FROM refs WHERE name = ?1")?;
        for key in keys {
            let rows = stmt.query_map([key], |r| r.get::<_, String>(0))?;
            for row in rows {
                out.insert(row?);
            }
        }
        Ok(out)
    }

    pub fn refs_for_file(&self, path: &str) -> Result<Vec<RawRef>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT src, name, qualifier, kind, line FROM refs WHERE path = ?1 ORDER BY rowid")?;
        let rows = stmt.query_map([path], |r| {
            let kind: String = r.get(3)?;
            Ok(RawRef {
                src: r.get(0)?,
                name: r.get(1)?,
                qualifier: r.get(2)?,
                kind: RefKind::parse(&kind).unwrap_or(RefKind::Call),
                line: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn all_paths(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT path FROM files ORDER BY path")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn count(&self, table: &str) -> Result<i64> {
        Ok(self.conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?)
    }
}
