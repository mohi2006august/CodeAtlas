//! Symbol graph (FR-02): resolve raw references into weighted edges.
//!
//! Resolution is name-based with locality heuristics (same class, same file,
//! imported file, same directory). It is a pure function of a file's refs and
//! the symbol table, which is what lets incremental indexing re-resolve only
//! the affected files and still match a full rebuild exactly.
//!
//! Edge types: `calls`, `imports`, `inherits`, `tests`, `references`.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Result;
use rusqlite::params;

use crate::extract::{module_name, strip_test_affixes};
use crate::imports::INDEX_STEMS;
use crate::model::{family, norm, RawRef, RefKind};
use crate::store::Store;

const SELF_WORDS: &[&str] = &["self", "this", "cls", "super", "Self", "me"];
const TEST_DIRS: &[&str] = &["test", "tests", "__tests__", "spec", "specs"];
/// Languages where an import names a directory (package) rather than a file.
const DIR_PACKAGE_LANGS: &[&str] = &["go", "java", "kotlin", "scala", "csharp"];

#[derive(Clone, Debug)]
pub struct Cand {
    pub id: String,
    pub kind: String,
    pub lang: String,
    pub path: String,
    pub qualname: String,
    pub container: String,
    pub is_test: bool,
}

#[derive(Default)]
struct Lookup {
    by_name: HashMap<String, Vec<Cand>>,
    by_norm: HashMap<String, Vec<Cand>>,
    modules_by_norm: HashMap<String, Vec<Cand>>,
    modules_by_pkg: HashMap<String, Vec<Cand>>,
}

const CAND_COLS: &str = "id, kind, lang, path, qualname, container, is_test";

fn row_to_cand(r: &rusqlite::Row) -> rusqlite::Result<Cand> {
    Ok(Cand {
        id: r.get(0)?,
        kind: r.get(1)?,
        lang: r.get(2)?,
        path: r.get(3)?,
        qualname: r.get(4)?,
        container: r.get(5)?,
        is_test: r.get(6)?,
    })
}

impl Lookup {
    fn query(store: &Store, sql: &str, key: &str) -> Result<Vec<Cand>> {
        let mut stmt = store.conn.prepare_cached(sql)?;
        let rows = stmt.query_map([key], row_to_cand)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn by_name(&mut self, store: &Store, key: &str) -> Result<&[Cand]> {
        if !self.by_name.contains_key(key) {
            let sql = format!(
                "SELECT {CAND_COLS} FROM symbols WHERE name = ?1 AND kind IN ('function','method','class') ORDER BY id"
            );
            let v = Self::query(store, &sql, key)?;
            self.by_name.insert(key.to_string(), v);
        }
        Ok(&self.by_name[key])
    }

    fn by_norm(&mut self, store: &Store, key: &str) -> Result<&[Cand]> {
        if !self.by_norm.contains_key(key) {
            let sql = format!(
                "SELECT {CAND_COLS} FROM symbols WHERE norm = ?1 AND kind IN ('function','method','class') ORDER BY id"
            );
            let v = Self::query(store, &sql, key)?;
            self.by_norm.insert(key.to_string(), v);
        }
        Ok(&self.by_norm[key])
    }

    fn modules_by_norm(&mut self, store: &Store, key: &str) -> Result<&[Cand]> {
        if !self.modules_by_norm.contains_key(key) {
            let sql = format!("SELECT {CAND_COLS} FROM symbols WHERE norm = ?1 AND kind = 'module' ORDER BY id");
            let v = Self::query(store, &sql, key)?;
            self.modules_by_norm.insert(key.to_string(), v);
        }
        Ok(&self.modules_by_norm[key])
    }

    fn modules_by_pkg(&mut self, store: &Store, key: &str) -> Result<&[Cand]> {
        if !self.modules_by_pkg.contains_key(key) {
            let sql = format!("SELECT {CAND_COLS} FROM symbols WHERE pkg = ?1 AND kind = 'module' ORDER BY id");
            let v = Self::query(store, &sql, key)?;
            self.modules_by_pkg.insert(key.to_string(), v);
        }
        Ok(&self.modules_by_pkg[key])
    }
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

fn dir_segs(path: &str) -> Vec<String> {
    dir_of(path).split('/').filter(|s| !s.is_empty()).map(norm).collect()
}

/// Normalised path segments a module answers to (`a/b/__init__.py` -> `a/b`).
fn module_segments(path: &str, as_package: bool) -> Vec<String> {
    let mut segs = dir_segs(path);
    if !as_package {
        let file = path.rsplit('/').next().unwrap_or(path);
        let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file);
        if !INDEX_STEMS.contains(&stem) {
            segs.push(norm(stem));
        }
    }
    segs
}

fn common_suffix(a: &[String], b: &[String]) -> usize {
    a.iter().rev().zip(b.iter().rev()).take_while(|(x, y)| x == y).count()
}

fn common_prefix(a: &[String], b: &[String]) -> usize {
    a.iter().zip(b.iter()).take_while(|(x, y)| x == y).count()
}

#[derive(Clone)]
struct SrcInfo {
    lang: String,
    path: String,
    qualname: String,
    is_test: bool,
}

type EdgeMap = BTreeMap<(String, String, &'static str), f64>;

fn add_edge(edges: &mut EdgeMap, src: &str, dst: &str, kind: &'static str, weight: f64) {
    if src == dst {
        return;
    }
    let w = edges.entry((src.to_string(), dst.to_string(), kind)).or_insert(0.0);
    *w = w.max(weight);
}

fn resolve_import(
    r: &RawRef,
    src: &SrcInfo,
    lookup: &mut Lookup,
    store: &Store,
) -> Result<Vec<(Cand, f64)>> {
    let exact = r.qualifier.starts_with('/');
    let target: Vec<String> = r.qualifier.trim_start_matches('/').split('/').map(norm).collect();
    let fam = family(&src.lang);
    let mut cands: Vec<(Cand, bool)> = lookup
        .modules_by_norm(store, &r.name)?
        .iter()
        .map(|c| (c.clone(), false))
        .collect();
    if DIR_PACKAGE_LANGS.contains(&src.lang.as_str()) && !exact {
        cands.extend(lookup.modules_by_pkg(store, &r.name)?.iter().map(|c| (c.clone(), true)));
    }
    cands.retain(|(c, _)| family(&c.lang) == fam && c.path != src.path);
    if exact {
        return Ok(cands
            .into_iter()
            .filter(|(c, pkg)| module_segments(&c.path, *pkg) == target)
            .map(|(c, _)| (c, 1.0))
            .collect());
    }
    let total = cands.len();
    let src_dir = dir_segs(&src.path);
    let scored: Vec<(Cand, usize, usize, bool)> = cands
        .into_iter()
        .map(|(c, pkg)| {
            let segs = module_segments(&c.path, pkg);
            let m = common_suffix(&segs, &target);
            let near = common_prefix(&dir_segs(&c.path), &src_dir);
            (c, m, near, pkg)
        })
        .filter(|(_, m, _, _)| *m >= 1)
        .collect();
    let Some(best) = scored.iter().map(|s| s.1).max() else { return Ok(Vec::new()) };
    let mut top: Vec<_> = scored.into_iter().filter(|s| s.1 == best).collect();
    let nearest = top.iter().map(|s| s.2).max().unwrap_or(0);
    let all_pkg = top.iter().all(|s| s.3);
    if !all_pkg {
        top.retain(|s| s.2 == nearest);
    }
    let weight = if best == target.len() && target.len() >= 2 {
        1.0
    } else if best >= 2 {
        0.8
    } else {
        // A one-segment match (`import utils`) is trusted when the module sits
        // next to the importer or in an ancestor directory, or is unique.
        let near_ancestor = top.iter().all(|s| {
            let d = dir_segs(&s.0.path);
            d.len() <= src_dir.len() && common_prefix(&d, &src_dir) == d.len()
        });
        if near_ancestor {
            0.8
        } else if total == 1 {
            0.5
        } else {
            return Ok(Vec::new());
        }
    };
    let cap = if all_pkg { 20 } else { 3 };
    Ok(top.into_iter().take(cap).map(|s| (s.0, weight)).collect())
}

fn score(r: &RawRef, src: &SrcInfo, c: &Cand, imported: &HashSet<String>) -> i32 {
    let same_file = c.path == src.path;
    let locality = if same_file {
        50
    } else if imported.contains(&c.path) {
        45
    } else if dir_of(&c.path) == dir_of(&src.path) {
        20
    } else {
        0
    };
    let in_same_scope =
        !c.container.is_empty() && (src.qualname == c.container || src.qualname.starts_with(&format!("{}.", c.container)));
    let owner = c.container.rsplit('.').next().unwrap_or("");
    let q = r.qualifier.as_str();
    let qtail = q.rsplit('.').next().unwrap_or("");
    let base = match r.kind {
        RefKind::Inherit => 40,
        RefKind::Ref => 30,
        _ if q.is_empty() => {
            if in_same_scope && (same_file || dir_of(&c.path) == dir_of(&src.path)) {
                95
            } else if c.kind == "method" {
                5
            } else {
                40
            }
        }
        _ if SELF_WORDS.contains(&qtail) => {
            if in_same_scope {
                100
            } else if c.kind == "method" {
                20
            } else {
                5
            }
        }
        _ if qtail != "?" && !owner.is_empty() && norm(qtail) == norm(owner) => 90,
        _ if qtail != "?" && c.container.is_empty() && norm(qtail) == norm(&module_name(&c.path)) => 85,
        _ if c.kind == "method" => 25,
        _ => 10,
    };
    base + locality
}

fn distinctive(name: &str) -> bool {
    name.len() >= 8
        && (name.contains('_') || name.chars().skip(1).any(|c| c.is_uppercase()))
}

fn resolve_symbol_ref(
    r: &RawRef,
    src: &SrcInfo,
    imported: &HashSet<String>,
    lookup: &mut Lookup,
    store: &Store,
) -> Result<Vec<(Cand, f64)>> {
    let fam = family(&src.lang);
    let src_id = &r.src;
    let all = lookup.by_name(store, &r.name)?;
    let eligible = |c: &&Cand| match r.kind {
        RefKind::Call => true,
        _ => c.kind == "class",
    };
    let same: Vec<Cand> =
        all.iter().filter(eligible).filter(|c| family(&c.lang) == fam && &c.id != src_id).cloned().collect();
    if same.is_empty() {
        if r.kind == RefKind::Call && distinctive(&r.name) {
            let other: Vec<&Cand> = all.iter().filter(|c| family(&c.lang) != fam).collect();
            if let [only] = other.as_slice() {
                return Ok(vec![((*only).clone(), 0.3)]);
            }
        }
        return Ok(Vec::new());
    }
    let mut scored: Vec<(i32, Cand)> = same.into_iter().map(|c| (score(r, src, &c, imported), c)).collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    let best = scored[0].0;
    let top: Vec<Cand> = scored.iter().filter(|s| s.0 == best).map(|s| s.1.clone()).collect();
    if best >= 60 {
        let w = if best >= 90 { 1.0 } else { 0.8 };
        if top.len() == 1 {
            return Ok(vec![(top[0].clone(), w)]);
        }
        return Ok(top.into_iter().take(3).map(|c| (c, w * 0.7)).collect());
    }
    if scored.len() == 1 && r.name.chars().count() >= 3 {
        return Ok(vec![(scored[0].1.clone(), 0.5)]);
    }
    if best >= 25 && top.len() <= 3 {
        return Ok(top.into_iter().map(|c| (c, 0.3)).collect());
    }
    Ok(Vec::new())
}

fn test_target_norm(path: &str) -> Option<String> {
    let stem = module_name(path);
    let stem = stem.split('.').next().unwrap_or(&stem).to_string();
    strip_test_affixes(&stem).map(|s| norm(&s))
}

fn resolve_test_file(r: &RawRef, src: &SrcInfo, lookup: &mut Lookup, store: &Store) -> Result<Vec<(Cand, f64)>> {
    let fam = family(&src.lang);
    let cands: Vec<Cand> = lookup
        .modules_by_norm(store, &r.name)?
        .iter()
        .filter(|c| !c.is_test && family(&c.lang) == fam && c.path != src.path)
        .cloned()
        .collect();
    let src_dir: Vec<String> =
        dir_segs(&src.path).into_iter().filter(|d| !TEST_DIRS.contains(&d.as_str())).collect();
    let mut scored: Vec<(usize, usize, Cand)> = cands
        .into_iter()
        .map(|c| {
            let d = dir_segs(&c.path);
            (common_suffix(&d, &src_dir), common_prefix(&d, &src_dir), c)
        })
        .collect();
    scored.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)).then_with(|| a.2.id.cmp(&b.2.id)));
    let Some((s0, p0)) = scored.first().map(|s| (s.0, s.1)) else { return Ok(Vec::new()) };
    let top: Vec<Cand> = scored.into_iter().filter(|s| s.0 == s0 && s.1 == p0).map(|s| s.2).collect();
    let w = if top.len() == 1 { 0.9 } else { 0.5 };
    Ok(top.into_iter().take(3).map(|c| (c, w)).collect())
}

fn resolve_test_name(
    r: &RawRef,
    src: &SrcInfo,
    imported: &HashSet<String>,
    lookup: &mut Lookup,
    store: &Store,
) -> Result<Vec<(Cand, f64)>> {
    let fam = family(&src.lang);
    let target_file = test_target_norm(&src.path);
    let cands: Vec<Cand> = lookup
        .by_norm(store, &r.name)?
        .iter()
        .filter(|c| !c.is_test && family(&c.lang) == fam)
        .cloned()
        .collect();
    let mut scored: Vec<(i32, Cand)> = cands
        .into_iter()
        .map(|c| {
            let s = if target_file.as_deref() == Some(norm(&module_name(&c.path)).as_str()) {
                90
            } else if imported.contains(&c.path) {
                70
            } else {
                0
            };
            (s, c)
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    let Some(best) = scored.first().map(|s| s.0) else { return Ok(Vec::new()) };
    if best >= 70 {
        return Ok(scored.into_iter().filter(|s| s.0 == best).take(3).map(|s| (s.1, 0.9)).collect());
    }
    if scored.len() == 1 {
        return Ok(vec![(scored.remove(0).1, 0.5)]);
    }
    Ok(Vec::new())
}

#[derive(Default, Debug, Clone, Copy)]
pub struct ResolveStats {
    pub files: usize,
    pub refs: usize,
    pub edges: usize,
}

fn src_infos(store: &Store, path: &str) -> Result<HashMap<String, SrcInfo>> {
    let mut stmt =
        store.conn.prepare_cached("SELECT id, lang, path, qualname, is_test FROM symbols WHERE path = ?1")?;
    let rows = stmt.query_map([path], |r| {
        Ok((
            r.get::<_, String>(0)?,
            SrcInfo { lang: r.get(1)?, path: r.get(2)?, qualname: r.get(3)?, is_test: r.get(4)? },
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Recompute every outgoing edge of the given files.
pub fn resolve_files(store: &Store, paths: &[String]) -> Result<ResolveStats> {
    let mut lookup = Lookup::default();
    let mut stats = ResolveStats::default();
    let mut delete = store.conn.prepare_cached("DELETE FROM edges WHERE src_path = ?1")?;
    let mut insert = store.conn.prepare_cached(
        "INSERT OR REPLACE INTO edges (src, dst, type, weight, src_path) VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    for path in paths {
        delete.execute([path])?;
        let infos = src_infos(store, path)?;
        if infos.is_empty() {
            continue;
        }
        let refs = store.refs_for_file(path)?;
        stats.files += 1;
        stats.refs += refs.len();
        let mut edges = EdgeMap::new();
        let mut imported: HashSet<String> = HashSet::new();

        // Imports first: they feed the locality signal for everything else.
        for r in refs.iter().filter(|r| r.kind == RefKind::Import) {
            let Some(src) = infos.get(&r.src) else { continue };
            for (c, w) in resolve_import(r, src, &mut lookup, store)? {
                imported.insert(c.path.clone());
                add_edge(&mut edges, &r.src, &c.id, "imports", w);
            }
        }
        for r in refs.iter().filter(|r| r.kind != RefKind::Import) {
            let Some(src) = infos.get(&r.src) else { continue };
            let resolved = match r.kind {
                RefKind::TestFile => resolve_test_file(r, src, &mut lookup, store)?,
                RefKind::TestName => resolve_test_name(r, src, &imported, &mut lookup, store)?,
                _ => resolve_symbol_ref(r, src, &imported, &mut lookup, store)?,
            };
            for (c, w) in resolved {
                let kind = match r.kind {
                    RefKind::Inherit => "inherits",
                    RefKind::Ref => "references",
                    RefKind::TestFile | RefKind::TestName => "tests",
                    _ if src.is_test && !c.is_test => "tests",
                    _ => "calls",
                };
                add_edge(&mut edges, &r.src, &c.id, kind, w);
            }
        }
        // A type mention adds nothing when a stronger edge already links the pair.
        let strong: HashSet<(String, String)> = edges
            .keys()
            .filter(|(_, _, k)| *k != "references")
            .map(|(s, d, _)| (s.clone(), d.clone()))
            .collect();
        edges.retain(|(s, d, k), _| *k != "references" || !strong.contains(&(s.clone(), d.clone())));
        for ((src, dst, kind), weight) in &edges {
            insert.execute(params![src, dst, kind, weight, path])?;
        }
        stats.edges += edges.len();
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_segments_handle_index_files() {
        assert_eq!(module_segments("app/billing/__init__.py", false), ["app", "billing"]);
        assert_eq!(module_segments("app/billing/retry.py", false), ["app", "billing", "retry"]);
        assert_eq!(module_segments("pkg/metrics/record.go", true), ["pkg", "metrics"]);
    }

    #[test]
    fn suffix_and_prefix() {
        let a: Vec<String> = ["x", "billing", "retry"].iter().map(|s| s.to_string()).collect();
        let b: Vec<String> = ["billing", "retry"].iter().map(|s| s.to_string()).collect();
        assert_eq!(common_suffix(&a, &b), 2);
        assert_eq!(common_prefix(&a, &b), 0);
    }
}
