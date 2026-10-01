//! Turn import statements into lookup targets.
//!
//! Every target becomes a `RawRef` of kind `import` whose `name` is the
//! normalised last path segment (the lookup key) and whose `qualifier` is the
//! slash-separated path. A leading `/` marks a path already resolved relative
//! to the importing file, which must match exactly; other paths are matched by
//! their longest common suffix with candidate module paths.

use crate::lang;
use crate::model::norm;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub key: String,
    pub qualifier: String,
}

enum Raw {
    /// Dotted or slashed module path matched by suffix.
    Path(String),
    /// `./x` style path relative to the importing file.
    Rel(String),
    /// Python relative import: number of leading dots and the rest.
    PyRel(usize, String),
}

/// File stems that stand for their directory (`pkg/__init__.py` is `pkg`).
pub const INDEX_STEMS: &[&str] = &["__init__", "index", "mod"];

fn quoted(text: &str) -> Option<String> {
    let start = text.find(['"', '\'', '`'])?;
    let quote = text[start..].chars().next()?;
    let rest = &text[start + 1..];
    let end = rest.find(quote)?;
    Some(rest[..end].to_string())
}

fn between(text: &str, open: char, close: char) -> Option<String> {
    let start = text.find(open)?;
    let end = text[start + 1..].find(close)? + start + 1;
    Some(text[start + 1..end].to_string())
}

fn matching_brace(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + i);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts.into_iter().map(str::trim).filter(|p| !p.is_empty()).collect()
}

/// `a::{b, c::{d, e}}` -> `a::b`, `a::c::d`, `a::c::e`.
fn expand_braces(s: &str) -> Vec<String> {
    let s = s.trim();
    let Some(open) = s.find('{') else {
        return vec![s.to_string()];
    };
    let Some(close) = matching_brace(s, open) else {
        return vec![s.replace(['{', '}'], "")];
    };
    let prefix = &s[..open];
    split_top_level(&s[open + 1..close])
        .into_iter()
        .flat_map(expand_braces)
        .map(|p| format!("{prefix}{p}"))
        .collect()
}

fn strip_alias(s: &str) -> &str {
    let s = s.split(" as ").next().unwrap_or(s);
    s.split("=>").next().unwrap_or(s).trim()
}

fn dotted(s: &str) -> String {
    s.trim().trim_end_matches(".*").trim_end_matches("._").replace('.', "/")
}

/// Push `path` and, when it has two or more segments, its parent: the last
/// segment of `import a.b.C` is often an item rather than a module.
fn with_parent(out: &mut Vec<Raw>, path: String) {
    if let Some((parent, _)) = path.rsplit_once('/') {
        if !parent.is_empty() {
            out.push(Raw::Path(parent.to_string()));
        }
    }
    out.push(Raw::Path(path));
}

fn python(text: &str, out: &mut Vec<Raw>) {
    let t: String = text.replace(['(', ')', '\\'], " ").split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some(rest) = t.strip_prefix("from ") {
        let Some((module, names)) = rest.split_once(" import ") else { return };
        let dots = module.chars().take_while(|c| *c == '.').count();
        let module_path = module[dots..].replace('.', "/");
        let push = |out: &mut Vec<Raw>, p: String| {
            if dots > 0 {
                out.push(Raw::PyRel(dots, p));
            } else if !p.is_empty() {
                out.push(Raw::Path(p));
            }
        };
        if !module_path.is_empty() {
            push(out, module_path.clone());
        }
        for name in names.split(',') {
            let name = strip_alias(name.trim());
            if name.is_empty() || name == "*" {
                continue;
            }
            let joined =
                if module_path.is_empty() { name.to_string() } else { format!("{module_path}/{name}") };
            push(out, joined);
        }
    } else if let Some(rest) = t.strip_prefix("import ") {
        for part in rest.split(',') {
            let module = strip_alias(part.trim());
            if !module.is_empty() {
                out.push(Raw::Path(module.replace('.', "/")));
            }
        }
    }
}

fn rust(text: &str, out: &mut Vec<Raw>) {
    let t = text.trim().trim_end_matches(';');
    if let Some(i) = t.find("mod ") {
        if !t.contains("use ") {
            out.push(Raw::Path(t[i + 4..].trim().to_string()));
            return;
        }
    }
    let Some(i) = t.find("use ") else { return };
    for path in expand_braces(&t[i + 4..]) {
        let mut p = strip_alias(&path).trim_end_matches("::*").trim_end_matches("::self").trim();
        loop {
            let before = p;
            for prefix in ["::", "crate::", "self::", "super::"] {
                p = p.strip_prefix(prefix).unwrap_or(p);
            }
            if p == before {
                break;
            }
        }
        if !p.is_empty() && !matches!(p, "self" | "crate" | "super") {
            with_parent(out, p.replace("::", "/"));
        }
    }
}

fn jvm(text: &str, out: &mut Vec<Raw>) {
    let mut t = text.trim().trim_end_matches(';').trim();
    for kw in ["import ", "static "] {
        t = t.strip_prefix(kw).unwrap_or(t).trim();
    }
    for path in expand_braces(t) {
        let p = strip_alias(&path);
        if p.ends_with("given") {
            continue;
        }
        for part in split_top_level(p) {
            // `import a.b.*` names a package, so there is no item to drop.
            if part.ends_with(".*") || part.ends_with("._") {
                out.push(Raw::Path(dotted(part)));
            } else {
                with_parent(out, dotted(part));
            }
        }
    }
}

fn csharp(text: &str, out: &mut Vec<Raw>) {
    let mut t = text.trim().trim_end_matches(';').trim();
    for kw in ["global ", "using ", "static "] {
        t = t.strip_prefix(kw).unwrap_or(t).trim();
    }
    let t = t.rsplit('=').next().unwrap_or(t).trim();
    out.push(Raw::Path(dotted(t)));
}

fn php(text: &str, out: &mut Vec<Raw>) {
    let t = text.trim().trim_end_matches(';').trim();
    if let Some(s) = quoted(t) {
        out.push(if s.starts_with('.') { Raw::Rel(s) } else { Raw::Path(s) });
        return;
    }
    let mut t = t.strip_prefix("use ").unwrap_or(t).trim();
    for kw in ["function ", "const "] {
        t = t.strip_prefix(kw).unwrap_or(t).trim();
    }
    for path in expand_braces(t) {
        for part in split_top_level(&path) {
            let p = strip_alias(part).trim_start_matches('\\').replace('\\', "/");
            if !p.is_empty() {
                with_parent(out, p);
            }
        }
    }
}

fn haskell(text: &str, out: &mut Vec<Raw>) {
    let module = text
        .split_whitespace()
        .skip(1)
        .find(|tok| tok.chars().next().is_some_and(char::is_uppercase));
    if let Some(m) = module {
        out.push(Raw::Path(dotted(m.split('(').next().unwrap_or(m))));
    }
}

fn julia(text: &str, out: &mut Vec<Raw>) {
    let t = text.trim();
    let t = t.strip_prefix("using").or_else(|| t.strip_prefix("import")).unwrap_or(t);
    let t = t.split(':').next().unwrap_or(t);
    for part in t.split(',') {
        let p = part.trim().trim_start_matches('.');
        if !p.is_empty() {
            out.push(Raw::Path(dotted(p)));
        }
    }
}

fn path_or_rel(s: String) -> Raw {
    if s.starts_with('.') {
        Raw::Rel(s)
    } else {
        Raw::Path(s)
    }
}

/// Targets of an import statement node.
pub fn from_statement(lang: &str, text: &str, file_dir: &str) -> Vec<Target> {
    let mut raw = Vec::new();
    match lang {
        "python" => python(text, &mut raw),
        "javascript" | "typescript" | "tsx" | "go" | "c" | "cpp" | "objc" => {
            let s = quoted(text).or_else(|| between(text, '<', '>'));
            if let Some(s) = s {
                raw.push(path_or_rel(s));
            }
        }
        "dart" => {
            if let Some(s) = quoted(text) {
                if let Some(p) = s.strip_prefix("package:") {
                    raw.push(Raw::Path(p.to_string()));
                } else if !s.starts_with("dart:") {
                    raw.push(Raw::Rel(s));
                }
            }
        }
        "rust" => rust(text, &mut raw),
        "java" | "kotlin" | "scala" => jvm(text, &mut raw),
        "csharp" => csharp(text, &mut raw),
        "php" => php(text, &mut raw),
        "swift" => {
            if let Some(last) = text.split_whitespace().last() {
                raw.push(Raw::Path(dotted(last)));
            }
        }
        "haskell" => haskell(text, &mut raw),
        "ocaml" => {
            if let Some(m) = text.split_whitespace().nth(1) {
                raw.push(Raw::Path(dotted(m)));
            }
        }
        "julia" => julia(text, &mut raw),
        _ => {}
    }
    finish(raw, file_dir)
}

/// Targets of a call that imports (`require("x")`, `source x`, `@import("x")`).
pub fn from_call(lang: &str, call: &str, arg: &str, file_dir: &str) -> Vec<Target> {
    let arg = arg.trim().trim_matches(|c| c == '"' || c == '\'' || c == '`').trim();
    if arg.is_empty() {
        return Vec::new();
    }
    let mut raw = Vec::new();
    match (lang, call) {
        ("ruby", "require_relative") | ("julia", "include") => {
            let rel = if arg.starts_with('.') { arg.to_string() } else { format!("./{arg}") };
            raw.push(Raw::Rel(rel));
        }
        ("zig", _) if arg.ends_with(".zig") => {
            let rel = if arg.starts_with('.') { arg.to_string() } else { format!("./{arg}") };
            raw.push(Raw::Rel(rel));
        }
        ("lua", "require") | ("elixir", _) => {
            for path in expand_braces(arg) {
                raw.push(Raw::Path(dotted(&path)));
            }
        }
        _ => raw.push(path_or_rel(arg.to_string())),
    }
    finish(raw, file_dir)
}

fn has_code_ext(seg: &str) -> Option<usize> {
    let (stem, ext) = seg.rsplit_once('.')?;
    let ext = ext.to_ascii_lowercase();
    let known = ext == "h"
        || lang::specs().iter().any(|s| s.exts.contains(&ext.as_str()))
        || lang::FALLBACK_EXTS.contains(&ext.as_str());
    (known && !stem.is_empty()).then_some(stem.len())
}

fn normalise(segs: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in segs {
        match s.as_str() {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            _ => out.push(s),
        }
    }
    if let Some(last) = out.last_mut() {
        if let Some(stem_len) = has_code_ext(last) {
            last.truncate(stem_len);
        }
    }
    if out.len() > 1 && out.last().is_some_and(|l| INDEX_STEMS.contains(&l.as_str())) {
        out.pop();
    }
    out
}

fn split(p: &str) -> Vec<String> {
    p.split('/').map(str::to_string).collect()
}

fn finish(raw: Vec<Raw>, file_dir: &str) -> Vec<Target> {
    let dir_segs: Vec<String> =
        file_dir.split('/').filter(|s| !s.is_empty()).map(str::to_string).collect();
    let mut out: Vec<Target> = Vec::new();
    for r in raw {
        let (segs, exact) = match r {
            Raw::Path(p) => {
                let segs: Vec<String> =
                    split(&p).into_iter().filter(|s| s != ".." && s != ".").collect();
                (normalise(segs), false)
            }
            Raw::Rel(p) => {
                let mut segs = dir_segs.clone();
                segs.extend(split(&p));
                (normalise(segs), true)
            }
            Raw::PyRel(dots, p) => {
                let mut segs = dir_segs.clone();
                for _ in 1..dots {
                    segs.pop();
                }
                if !p.is_empty() {
                    segs.extend(split(&p));
                }
                (normalise(segs), true)
            }
        };
        let Some(last) = segs.last() else { continue };
        let key = norm(last);
        if key.is_empty() {
            continue;
        }
        let joined = segs.join("/");
        let qualifier = if exact { format!("/{joined}") } else { joined };
        let target = Target { key, qualifier };
        if !out.contains(&target) {
            out.push(target);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quals(targets: Vec<Target>) -> Vec<String> {
        targets.into_iter().map(|t| t.qualifier).collect()
    }

    #[test]
    fn python_absolute_and_relative() {
        let t = from_statement("python", "from billing.base import BasePolicy, clamp", "app");
        assert_eq!(quals(t), ["billing/base", "billing/base/BasePolicy", "billing/base/clamp"]);
        let t = from_statement("python", "from . import metrics", "app/billing");
        assert_eq!(quals(t), ["/app/billing/metrics"]);
        let t = from_statement("python", "from ..utils import log as l", "app/billing");
        assert_eq!(quals(t), ["/app/utils", "/app/utils/log"]);
        let t = from_statement("python", "import os.path, json", "");
        assert_eq!(quals(t), ["os/path", "json"]);
    }

    #[test]
    fn javascript_relative_paths_resolve_against_the_file() {
        let t = from_statement("javascript", "import { clamp } from \"./math.js\";", "src/billing");
        assert_eq!(quals(t), ["/src/billing/math"]);
        let t = from_call("javascript", "require", "\"../metrics\"", "src/billing");
        assert_eq!(quals(t), ["/src/metrics"]);
        let t = from_statement("typescript", "import x from './lib/index'", "src");
        assert_eq!(quals(t), ["/src/lib"]);
        let t = from_statement("javascript", "import React from 'react'", "src");
        assert_eq!(t, vec![Target { key: "react".into(), qualifier: "react".into() }]);
    }

    #[test]
    fn rust_use_trees_expand() {
        let t = from_statement("rust", "use std::{collections::HashMap, time::Duration};", "");
        assert!(quals(t.clone()).contains(&"std/time/Duration".to_string()));
        assert!(quals(t).contains(&"std/collections".to_string()));
        let t = from_statement("rust", "use crate::billing::math::clamp;", "");
        assert_eq!(quals(t), ["billing/math", "billing/math/clamp"]);
        let t = from_statement("rust", "mod metrics;", "src");
        assert_eq!(quals(t), ["metrics"]);
    }

    #[test]
    fn jvm_csharp_php_and_c() {
        let t = from_statement("java", "import com.acme.billing.math.Clamp;", "");
        assert_eq!(quals(t), ["com/acme/billing/math", "com/acme/billing/math/Clamp"]);
        let t = from_statement("scala", "import a.b.{C, D => E}", "");
        assert!(quals(t).contains(&"a/b/D".to_string()));
        let t = from_statement("csharp", "using Acme.Billing.Math;", "");
        assert_eq!(quals(t), ["Acme/Billing/Math"]);
        let t = from_statement("php", "use Acme\\Billing\\Math\\Clamp;", "");
        assert!(quals(t).contains(&"Acme/Billing/Math/Clamp".to_string()));
        let t = from_statement("c", "#include \"billing/math.h\"", "src");
        assert_eq!(quals(t), ["billing/math"]);
        let t = from_statement("cpp", "#include <chrono>", "src");
        assert_eq!(quals(t), ["chrono"]);
    }

    #[test]
    fn import_calls() {
        let t = from_call("ruby", "require_relative", "\"base_policy\"", "lib/billing");
        assert_eq!(quals(t), ["/lib/billing/base_policy"]);
        let t = from_call("lua", "require", "\"billing.math\"", "");
        assert_eq!(quals(t), ["billing/math"]);
        let t = from_call("elixir", "alias", "Billing.Math", "");
        assert_eq!(t[0].key, "math");
        let t = from_call("zig", "@import", "\"billing/math.zig\"", "src");
        assert_eq!(quals(t), ["/src/billing/math"]);
    }
}
