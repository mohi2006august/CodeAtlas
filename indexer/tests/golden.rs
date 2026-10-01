//! Parser golden tests: one fixture per language, rendered to a reviewable
//! text snapshot. Regenerate with `UPDATE_GOLDEN=1 cargo test -p indexer`.

use std::fs;
use std::path::Path;

use indexer::extract::extract_file;
use indexer::model::FileIndex;

fn render(fi: &FileIndex) -> String {
    let mut out = format!("lang={} lines={} fallback={} test={}\n", fi.lang, fi.lines, fi.fallback, fi.is_test);
    for s in &fi.symbols {
        let test = if s.is_test { " test" } else { "" };
        out.push_str(&format!("{:<8} {}  [{}-{} def {}]{}\n", s.kind, s.id, s.start_line, s.end_line, s.def_line, test));
        out.push_str(&format!("         sig: {}\n", s.signature));
        if let Some(doc) = &s.doc {
            out.push_str(&format!("         doc: {}\n", doc.replace('\n', " | ")));
        }
        if let Some(parent) = &s.parent {
            out.push_str(&format!("         parent: {parent}\n"));
        }
    }
    for r in &fi.refs {
        let src = r.src.split_once("::").map(|(_, q)| q).unwrap_or("<module>");
        let qual = if r.qualifier.is_empty() { String::new() } else { format!(" via {}", r.qualifier) };
        out.push_str(&format!("ref {:<9} L{:<3} {} -> {}{}\n", r.kind.as_str(), r.line, src, r.name, qual));
    }
    out
}

#[test]
fn golden_per_language() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let mut failures = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(root.join("fixtures")).unwrap().map(|e| e.unwrap().path()).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let src = fs::read(&path).unwrap();
        let fi = extract_file(&name, &src).unwrap_or_else(|| panic!("{name}: not indexed"));
        assert!(!fi.fallback, "{name}: fell back to chunks");
        assert!(fi.symbols.len() > 2, "{name}: found no definitions");
        let actual = render(&fi);
        let golden = root.join("golden").join(format!("{name}.golden"));
        if update {
            fs::write(&golden, &actual).unwrap();
            continue;
        }
        match fs::read_to_string(&golden) {
            Ok(expected) if expected.replace("\r\n", "\n") == actual => {}
            Ok(_) => failures.push(format!("{name}: output differs from {}", golden.display())),
            Err(_) => failures.push(format!("{name}: missing {}", golden.display())),
        }
    }
    assert!(failures.is_empty(), "golden mismatches (UPDATE_GOLDEN=1 to accept):\n{}", failures.join("\n"));
}
