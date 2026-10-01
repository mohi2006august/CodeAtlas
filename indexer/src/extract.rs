//! Generic tree-sitter walker: AST -> symbols + unresolved references (FR-01).
//!
//! One walker serves every grammar; the per-language differences live in the
//! rule tables of `lang.rs`. Files that cannot be parsed, or code files with no
//! grammar, fall back to line-window chunks flagged `fallback`.

use std::collections::{HashMap, HashSet};

use tree_sitter::{Node, Parser};

use crate::imports;
use crate::lang::{self, text, CallRule, DefRule, Detected, LangSpec, NameBy, Sym};
use crate::model::{hash_hex, norm, FileIndex, RawRef, RefKind, Symbol};

const MAX_SIGNATURE_CHARS: usize = 300;
const MAX_DOC_CHARS: usize = 600;
const CHUNK_LINES: u32 = 50;

/// Byte offsets of line starts, so spans can be hashed exactly like the
/// Python side does: `b"\n".join(l.rstrip(b"\r") for l in lines[s-1:e])`.
pub struct Lines<'a> {
    src: &'a [u8],
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    pub fn new(src: &'a [u8]) -> Self {
        let mut starts = vec![0];
        starts.extend(src.iter().enumerate().filter(|(_, b)| **b == b'\n').map(|(i, _)| i + 1));
        Lines { src, starts }
    }

    pub fn count(&self) -> u32 {
        if self.src.is_empty() {
            return 0;
        }
        let n = self.starts.len() as u32;
        if self.src.ends_with(b"\n") {
            n - 1
        } else {
            n
        }
    }

    /// Line `n` (1-based) without its line terminator.
    pub fn line(&self, n: u32) -> &'a [u8] {
        let i = (n - 1) as usize;
        let start = self.starts[i];
        let end = self.starts.get(i + 1).map(|e| e - 1).unwrap_or(self.src.len());
        let line = &self.src[start..end.max(start)];
        line.strip_suffix(b"\r").unwrap_or(line)
    }

    pub fn hash(&self, start: u32, end: u32) -> String {
        let end = end.min(self.count());
        let mut buf = Vec::new();
        for n in start.max(1)..=end {
            if n > start.max(1) {
                buf.push(b'\n');
            }
            buf.extend_from_slice(self.line(n));
        }
        hash_hex(&buf)
    }
}

/// Is this path a test file by common naming conventions?
pub fn is_test_path(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    let dirs = &parts[..parts.len().saturating_sub(1)];
    if dirs.iter().any(|d| matches!(*d, "test" | "tests" | "__tests__" | "spec" | "specs")) {
        return true;
    }
    let file = parts.last().copied().unwrap_or("");
    let stem = file.split('.').next().unwrap_or(file);
    file.contains(".test.")
        || file.contains(".spec.")
        || stem.starts_with("test_")
        || stem == "conftest"
        || ["_test", "_spec", "Test", "Tests", "Spec"].iter().any(|s| stem.ends_with(s) && stem.len() > s.len())
}

/// `test_retry` / `RetryTest` / `retry.spec` -> `retry`. `None` if nothing was stripped.
pub fn strip_test_affixes(name: &str) -> Option<String> {
    let mut s = name;
    for prefix in ["test_", "Test", "test"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            if !rest.is_empty() {
                s = rest;
                break;
            }
        }
    }
    for suffix in ["_test", "_spec", "Tests", "Test", "Spec"] {
        if let Some(rest) = s.strip_suffix(suffix) {
            if !rest.is_empty() {
                s = rest;
                break;
            }
        }
    }
    let s = s.trim_matches('_');
    (s != name && !s.is_empty()).then(|| s.to_string())
}

/// Display name of a module: file stem, or the directory for index-like files.
pub fn module_name(path: &str) -> String {
    let mut parts = path.rsplit('/');
    let file = parts.next().unwrap_or(path);
    let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file);
    if imports::INDEX_STEMS.contains(&stem) {
        if let Some(dir) = parts.next() {
            return dir.to_string();
        }
    }
    stem.to_string()
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

pub fn valid_ident(name: &str) -> bool {
    let body = name.trim_end_matches(['?', '!']);
    let mut chars = body.chars();
    match chars.next() {
        Some(c) if c.is_alphabetic() || c == '_' || c == '$' => chars.all(is_ident_char),
        _ => false,
    }
}

/// Normalise a receiver expression to a dotted path, `?` if it is not a plain path.
fn simple_qual(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    for sep in ["::", "->", "?.", "&.", "!.", "\\"] {
        s = s.replace(sep, ".");
    }
    s = s.replace(['$', '@'], "");
    if s.is_empty() {
        return String::new();
    }
    if s.len() > 80 || !s.chars().all(|c| is_ident_char(c) || c == '.') {
        return "?".to_string();
    }
    let parts: Vec<&str> = s.split('.').filter(|p| !p.is_empty()).collect();
    parts.join(".")
}

/// Split a callee expression into `(name, receiver)`. `foo.bar::<T>` -> `("bar", "foo")`.
fn split_callee(raw: &str) -> Option<(String, String)> {
    let mut t = raw.trim().to_string();
    if t.ends_with('>') {
        t = lang::strip_generics(&t);
    }
    let t = t.trim_end_matches("::").trim_end();
    let chars: Vec<(usize, char)> = t.char_indices().collect();
    let mut i = chars.len();
    while i > 0 && matches!(chars[i - 1].1, '?' | '!') && i == chars.len() {
        i -= 1;
    }
    while i > 0 && is_ident_char(chars[i - 1].1) {
        i -= 1;
    }
    let start = chars.get(i).map(|(b, _)| *b).unwrap_or(t.len());
    let name = &t[start..];
    if !valid_ident(name) {
        return None;
    }
    let rest = t[..start].trim_end();
    if rest.is_empty() {
        return Some((name.to_string(), String::new()));
    }
    for sep in ["::", "?.", "->", "&.", "!.", ".", ":", "\\"] {
        if let Some(receiver) = rest.strip_suffix(sep) {
            return Some((name.to_string(), simple_qual(receiver)));
        }
    }
    Some((name.to_string(), "?".to_string()))
}

fn clean_name(raw: &str) -> Option<String> {
    let s = lang::strip_generics(raw.trim().trim_matches(|c| c == '"' || c == '\'' || c == '`'));
    let s: String = s
        .replace("::", ".")
        .replace(':', ".")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let s = s.trim_matches('.').to_string();
    if s.is_empty() || s.contains(['(', ')', '{', '}', '"', '\'', ',', ';', '=']) {
        return None;
    }
    Some(s)
}

fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Strip comment markers and string quotes from doc text.
pub fn clean_doc(raw: &str) -> String {
    let mut s = raw.trim();
    for prefix in ["@moduledoc", "@doc"] {
        s = s.strip_prefix(prefix).unwrap_or(s).trim();
    }
    let s = s.trim_start_matches(['r', 'b', 'u', 'f', 'R', 'B', 'U', 'F']);
    let s = s.trim_matches(|c| c == '"' || c == '\'');
    let mut out = Vec::new();
    for line in s.lines() {
        let mut l = line.trim();
        for prefix in [
            "///", "//!", "//", "/**", "/*", "(**", "(*", "#'", "#", "-- |", "--|", "--", "{-|", "{-",
            ";;", "*",
        ] {
            if let Some(rest) = l.strip_prefix(prefix) {
                l = rest;
                break;
            }
        }
        for suffix in ["*/", "*)", "-}"] {
            l = l.strip_suffix(suffix).unwrap_or(l);
        }
        let l = l.trim();
        if !l.is_empty() {
            out.push(l.to_string());
        }
    }
    out.join("\n")
}

struct Scope {
    end_byte: usize,
    qual: String,
    sym: usize,
    class_like: bool,
    is_test: bool,
}

struct Walker<'a> {
    spec: &'static LangSpec,
    path: &'a str,
    dir: &'a str,
    src: &'a [u8],
    file_is_test: bool,
    symbols: Vec<Symbol>,
    /// Index of the enclosing symbol when each symbol was created.
    scope_parent: Vec<usize>,
    id_counts: HashMap<String, u32>,
    merge_index: HashMap<String, usize>,
    refs: Vec<(usize, RefKind, String, String, u32)>,
    seen_refs: HashSet<(usize, RefKind, String, String)>,
    scopes: Vec<Scope>,
    call_kinds: Vec<&'static str>,
    defs_found: usize,
}

impl<'a> Walker<'a> {
    fn current(&self) -> &Scope {
        self.scopes.last().expect("module scope is never popped")
    }

    fn add_ref(&mut self, kind: RefKind, name: String, qualifier: String, line: usize) {
        let src = self.current().sym;
        if self.seen_refs.insert((src, kind, name.clone(), qualifier.clone())) {
            self.refs.push((src, kind, name, qualifier, line as u32 + 1));
        }
    }

    fn walk(&mut self, root: Node) {
        let mut cursor = root.walk();
        loop {
            let node = cursor.node();
            while self.scopes.len() > 1 && node.start_byte() >= self.current().end_byte {
                self.scopes.pop();
            }
            let descend = self.visit(node);
            if descend && cursor.goto_first_child() {
                continue;
            }
            loop {
                if cursor.goto_next_sibling() {
                    break;
                }
                if !cursor.goto_parent() {
                    return;
                }
            }
        }
    }

    fn visit(&mut self, node: Node) -> bool {
        if !node.is_named() {
            return false;
        }
        let kind = node.kind();
        if let Some(custom) = self.spec.custom_def {
            if let Some((sym, name)) = custom(node, self.src) {
                self.handle_def(node, sym, &name, None);
                return true;
            }
        }
        if self.spec.name == "rust" && kind == "mod_item" && node.child_by_field_name("body").is_none() {
            self.handle_import_node(node);
            return false;
        }
        if self.spec.import_kinds.contains(&kind)
            || (kind == "export_statement" && node.child_by_field_name("source").is_some())
        {
            self.handle_import_node(node);
            return false;
        }
        let spec = self.spec;
        for rule in spec.defs.iter().filter(|r| r.kind == kind) {
            if let Some(name) = self.match_def(node, rule) {
                self.handle_def(node, rule.sym, &name, Some(rule));
                return true;
            }
        }
        for rule in spec.calls.iter().filter(|r| r.kind == kind) {
            self.handle_call(node, rule);
        }
        if spec.type_ref_kinds.contains(&kind) {
            self.handle_type_ref(node);
            return false;
        }
        true
    }

    fn descend<'t>(node: Node<'t>, path: &[&str]) -> Option<Node<'t>> {
        let mut n = node;
        for step in path {
            n = match step.strip_prefix('#') {
                Some(kind) => {
                    let mut cursor = n.walk();
                    let found = n.named_children(&mut cursor).find(|c| c.kind() == kind);
                    found?
                }
                None => n.child_by_field_name(step)?,
            };
        }
        Some(n)
    }

    fn match_def(&self, node: Node, rule: &DefRule) -> Option<String> {
        if !rule.value_path.is_empty() {
            let value = Self::descend(node, rule.value_path)?;
            if !rule.value_kinds.is_empty() && !rule.value_kinds.contains(&value.kind()) {
                return None;
            }
        }
        if let Some(guard) = rule.guard {
            if !guard(node, self.src) {
                return None;
            }
        }
        match &rule.name {
            NameBy::Field(field) => Some(text(node.child_by_field_name(field)?, self.src).to_string()),
            NameBy::Declarator => lang::name_declarator(node, self.src),
            NameBy::GoReceiver => lang::name_go_receiver(node, self.src),
            NameBy::FirstKind(kinds) => {
                let mut cursor = node.walk();
                let found = node.named_children(&mut cursor).find(|c| kinds.contains(&c.kind()));
                found.map(|n| text(n, self.src).to_string())
            }
            NameBy::Const(name) => Some(name.to_string()),
            NameBy::Custom(f) => f(node, self.src),
        }
    }

    /// Walk up through wrapper nodes (decorators, `export`, `template<>`).
    fn outer<'t>(&self, node: Node<'t>) -> Node<'t> {
        let mut outer = node;
        while let Some(parent) = outer.parent() {
            let k = parent.kind();
            let sole = self.spec.sole_wrappers.contains(&k) && parent.named_child_count() == 1;
            if self.spec.wrappers.contains(&k) || sole {
                outer = parent;
            } else {
                break;
            }
        }
        outer
    }

    /// Leading comments / attributes directly above a definition.
    /// Returns (first row, doc text, test marker, type signature).
    fn leading(&self, outer: Node) -> (usize, Option<String>, bool, Option<String>) {
        let mut start_row = outer.start_position().row;
        let mut docs: Vec<String> = Vec::new();
        let mut test = false;
        let mut type_sig = None;
        let mut cur = outer;
        while let Some(prev) = cur.prev_sibling() {
            let k = prev.kind();
            let is_comment = self.spec.comment_kinds.contains(&k);
            let is_lead = self.spec.leading_kinds.contains(&k);
            if !(is_comment || is_lead) || prev.end_position().row + 1 < start_row {
                break;
            }
            let t = text(prev, self.src);
            if is_comment {
                if t.starts_with("#!") {
                    break;
                }
                // A trailing comment belongs to the code on its line, not to us.
                if let Some(pp) = prev.prev_sibling() {
                    let pk = pp.kind();
                    let pp_is_extra =
                        self.spec.comment_kinds.contains(&pk) || self.spec.leading_kinds.contains(&pk);
                    if !pp_is_extra && pp.end_position().row == prev.start_position().row {
                        break;
                    }
                }
                docs.push(t.to_string());
            } else {
                match k {
                    "unary_operator" if !t.starts_with('@') => break,
                    "unary_operator" if t.starts_with("@doc") || t.starts_with("@moduledoc") => {
                        docs.push(t.to_string())
                    }
                    "string_literal" => docs.push(t.to_string()),
                    "signature" => type_sig = Some(collapse_ws(t)),
                    "attribute_item" if t.contains("test") => test = true,
                    _ => {}
                }
            }
            start_row = prev.start_position().row;
            cur = prev;
        }
        docs.reverse();
        let doc = docs.iter().map(|d| clean_doc(d)).filter(|d| !d.is_empty()).collect::<Vec<_>>().join("\n");
        (start_row, (!doc.is_empty()).then_some(doc), test, type_sig)
    }

    fn docstring(&self, body: Option<Node>) -> Option<String> {
        if !self.spec.docstring {
            return None;
        }
        let first = body?.named_child(0)?;
        if first.kind() != "expression_statement" {
            return None;
        }
        let string = first.named_child(0)?;
        (string.kind() == "string").then(|| clean_doc(text(string, self.src))).filter(|d| !d.is_empty())
    }

    /// Elixir keeps a module's doc inside its body as `@moduledoc`.
    fn inner_moduledoc(&self, body: Option<Node>) -> Option<String> {
        let first = body?.named_child(0)?;
        let t = text(first, self.src);
        (first.kind() == "unary_operator" && t.starts_with("@moduledoc"))
            .then(|| clean_doc(t))
            .filter(|d| !d.is_empty())
    }

    fn find_body<'t>(&self, node: Node<'t>, rule: Option<&DefRule>) -> Option<Node<'t>> {
        let value = rule
            .filter(|r| !r.value_path.is_empty())
            .and_then(|r| Self::descend(node, r.value_path));
        for n in value.into_iter().chain(std::iter::once(node)) {
            if let Some(body) = n.child_by_field_name("body") {
                return Some(body);
            }
            let mut cursor = n.walk();
            let found = n.named_children(&mut cursor).find(|c| self.spec.body_kinds.contains(&c.kind()));
            if found.is_some() {
                return found;
            }
        }
        None
    }

    fn signature(&self, node: Node, outer: Node, body: Option<Node>) -> String {
        let start = outer.start_byte();
        // A comment between the header and the body is not part of the signature.
        let mut cursor = node.walk();
        let comment_start = node
            .children(&mut cursor)
            .filter(|c| self.spec.comment_kinds.contains(&c.kind()) && c.start_byte() > start)
            .map(|c| c.start_byte())
            .min();
        let body_start = body.map(|b| b.start_byte()).filter(|&e| e > start);
        let cut = match (body_start, comment_start) {
            (Some(b), Some(c)) => Some(b.min(c)),
            (b, c) => b.or(c),
        };
        let end = match cut {
            Some(e) => e,
            None => {
                let rest = &self.src[start..outer.end_byte()];
                let first_line = rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
                let brace = rest[..first_line].iter().position(|&b| b == b'{').unwrap_or(first_line);
                start + brace
            }
        };
        let raw = String::from_utf8_lossy(&self.src[start..end]);
        let sig = collapse_ws(&raw);
        let sig = sig.trim_end_matches('{').trim_end();
        truncate_chars(sig, MAX_SIGNATURE_CHARS)
    }

    fn unique_id(&mut self, base: String) -> String {
        let count = self.id_counts.entry(base.clone()).or_insert(0);
        *count += 1;
        if *count == 1 {
            base
        } else {
            format!("{base}~{count}")
        }
    }

    fn handle_def(&mut self, node: Node, sym: Sym, raw_name: &str, rule: Option<&DefRule>) {
        let Some(name) = clean_name(raw_name) else { return };
        let parent = self.current();
        let parent_qual = parent.qual.clone();
        let parent_sym = parent.sym;
        let parent_class_like = parent.class_like;
        let parent_is_test = parent.is_test;
        let qual = if parent_qual.is_empty() { name.clone() } else { format!("{parent_qual}.{name}") };

        if matches!(sym, Sym::Namespace | Sym::ClassScope) {
            if sym == Sym::ClassScope {
                if let Some(trait_node) = node.child_by_field_name("trait") {
                    // `impl Trait for Type`: the type inherits from the trait.
                    let trait_name = lang::strip_generics(text(trait_node, self.src));
                    let trait_name = trait_name.rsplit("::").next().unwrap_or("").to_string();
                    let type_id = format!("{}:{}::{}", self.spec.prefix, self.path, qual);
                    if let Some(idx) = self.symbols.iter().position(|s| s.id == type_id) {
                        if valid_ident(&trait_name) {
                            let key = (idx, RefKind::Inherit, trait_name.clone(), String::new());
                            if self.seen_refs.insert(key) {
                                let line = node.start_position().row as u32 + 1;
                                self.refs.push((idx, RefKind::Inherit, trait_name, String::new(), line));
                            }
                        }
                    }
                }
            }
            let is_test = parent_is_test || matches!(name.as_str(), "tests" | "test");
            self.scopes.push(Scope {
                end_byte: node.end_byte(),
                qual,
                sym: parent_sym,
                class_like: sym == Sym::ClassScope,
                is_test,
            });
            return;
        }

        let outer = self.outer(node);
        let (lead_row, lead_doc, lead_test, type_sig) = self.leading(outer);
        let body = self.find_body(node, rule);
        let doc = self
            .docstring(body)
            .or(lead_doc)
            .or_else(|| self.inner_moduledoc(body))
            .map(|d| truncate_chars(&d, MAX_DOC_CHARS));
        let signature = type_sig.unwrap_or_else(|| self.signature(node, outer, body));
        let kind = match sym {
            Sym::Class => "class",
            _ if parent_class_like || name.contains('.') => "method",
            _ => "function",
        };
        let leaf = rule.map(|r| r.leaf).unwrap_or(false);
        let is_test = self.file_is_test
            || parent_is_test
            || rule.map(|r| r.test).unwrap_or(false)
            || lead_test
            || signature.contains("@Test");
        let start_line = lead_row as u32 + 1;
        let mut end_row = outer.end_position().row;
        if outer.end_position().column == 0 && end_row > outer.start_position().row {
            end_row -= 1;
        }
        let end_line = end_row as u32 + 1;
        let base_id = format!("{}:{}::{}", self.spec.prefix, self.path, qual);
        self.defs_found += 1;

        if self.spec.merge_clauses {
            if let Some(&idx) = self.merge_index.get(&base_id) {
                if self.symbols[idx].kind == kind {
                    let s = &mut self.symbols[idx];
                    s.start_line = s.start_line.min(start_line);
                    s.end_line = s.end_line.max(end_line);
                    if s.doc.is_none() {
                        s.doc = doc;
                    }
                    self.scopes.push(Scope {
                        end_byte: node.end_byte(),
                        qual: if leaf { parent_qual } else { qual },
                        sym: idx,
                        class_like: false,
                        is_test,
                    });
                    return;
                }
            }
        }

        let id = self.unique_id(base_id.clone());
        let simple = name.rsplit('.').next().unwrap_or(&name).to_string();
        let container = qual.rsplit_once('.').map(|(c, _)| c.to_string()).unwrap_or_default();
        let idx = self.symbols.len();
        self.symbols.push(Symbol {
            id,
            kind: kind.to_string(),
            lang: self.spec.name.to_string(),
            path: self.path.to_string(),
            name: simple.clone(),
            qualname: qual.clone(),
            container,
            parent: None,
            start_line,
            end_line,
            def_line: outer.start_position().row as u32 + 1,
            signature,
            doc,
            body_hash: String::new(),
            is_test,
            fallback: false,
        });
        self.scope_parent.push(parent_sym);
        self.merge_index.entry(base_id).or_insert(idx);
        self.scopes.push(Scope {
            end_byte: node.end_byte(),
            qual: if leaf { parent_qual } else { qual },
            sym: idx,
            class_like: sym == Sym::Class && !leaf,
            is_test,
        });
        if sym == Sym::Class {
            self.inherit_refs(node, &simple);
        }
    }

    fn collect_names(&self, node: Node, out: &mut Vec<(String, usize)>, root: bool) {
        let k = node.kind();
        if !root
            && (k.contains("type_arguments")
                || k.contains("type_parameters")
                || k.contains("keyword_argument")
                || k.contains("arguments"))
        {
            return;
        }
        if node.named_child_count() == 0 {
            let identifier_like = k.ends_with("identifier")
                || matches!(k, "constant" | "name" | "variable" | "alias" | "type_constructor");
            let t = text(node, self.src);
            if identifier_like && valid_ident(t) {
                out.push((t.to_string(), node.start_position().row));
            }
            return;
        }
        let mut cursor = node.walk();
        let children: Vec<Node> = node.named_children(&mut cursor).collect();
        for child in children {
            self.collect_names(child, out, false);
        }
    }

    fn inherit_refs(&mut self, node: Node, own_name: &str) {
        let mut found: Vec<(String, usize)> = Vec::new();
        for entry in self.spec.inherit {
            let mut cursor = node.walk();
            let by_field: Vec<Node> = node.children_by_field_name(entry, &mut cursor).collect();
            let mut cursor = node.walk();
            let by_kind: Vec<Node> = node.named_children(&mut cursor).filter(|c| c.kind() == *entry).collect();
            for n in by_field.into_iter().chain(by_kind) {
                self.collect_names(n, &mut found, true);
            }
        }
        for (name, row) in found {
            if name != own_name {
                self.add_ref(RefKind::Inherit, name, String::new(), row);
            }
        }
    }

    fn select<'t>(node: Node<'t>, selector: &str) -> Option<Node<'t>> {
        if selector.is_empty() {
            return node.named_child(0);
        }
        Self::descend(node, &[selector])
    }

    fn first_arg<'t>(node: Node<'t>) -> Option<Node<'t>> {
        for field in ["arguments", "argument", "args"] {
            if let Some(a) = node.child_by_field_name(field) {
                if a.kind().contains("argument") && a.named_child_count() > 0 {
                    return a.named_child(0);
                }
                return Some(a);
            }
        }
        let mut cursor = node.walk();
        let args = node.named_children(&mut cursor).find(|c| c.kind().contains("argument"));
        match args {
            Some(a) if a.named_child_count() > 0 => a.named_child(0),
            Some(a) => Some(a),
            None => node.named_child(1),
        }
    }

    fn handle_import_node(&mut self, node: Node) {
        let t = text(node, self.src);
        let targets = imports::from_statement(self.spec.name, t, self.dir);
        let row = node.start_position().row;
        let module = 0;
        for target in targets {
            if self.seen_refs.insert((module, RefKind::Import, target.key.clone(), target.qualifier.clone())) {
                self.refs.push((module, RefKind::Import, target.key, target.qualifier, row as u32 + 1));
            }
        }
    }

    fn handle_import_call(&mut self, node: Node, call_name: &str) {
        let Some(arg) = Self::first_arg(node) else { return };
        let mut arg_text = text(arg, self.src).trim();
        // R wraps the value: `argument value: (string)`.
        if let Some(v) = arg.child_by_field_name("value") {
            arg_text = text(v, self.src).trim();
        }
        let targets = imports::from_call(self.spec.name, call_name, arg_text, self.dir);
        let row = node.start_position().row;
        for target in targets {
            if self.seen_refs.insert((0, RefKind::Import, target.key.clone(), target.qualifier.clone())) {
                self.refs.push((0, RefKind::Import, target.key, target.qualifier, row as u32 + 1));
            }
        }
    }

    fn handle_call(&mut self, node: Node, rule: &CallRule) {
        let (name, qualifier) = if !rule.object.is_empty() {
            let Some(name_node) = node.child_by_field_name(rule.callee) else { return };
            let name = text(name_node, self.src).trim().to_string();
            let qualifier = node
                .child_by_field_name(rule.object)
                .map(|o| simple_qual(text(o, self.src)))
                .unwrap_or_default();
            (name, qualifier)
        } else {
            let Some(callee) = Self::select(node, rule.callee) else { return };
            if self.call_kinds.contains(&callee.kind()) {
                return;
            }
            let raw = text(callee, self.src).trim();
            if self.spec.import_calls.contains(&raw) {
                self.handle_import_call(node, raw);
                return;
            }
            let Some(parts) = split_callee(raw) else { return };
            parts
        };
        if qualifier.is_empty() && self.spec.import_calls.contains(&name.as_str()) {
            self.handle_import_call(node, &name);
            return;
        }
        if self.spec.ignore_calls.contains(&name.as_str()) || !valid_ident(&name) {
            return;
        }
        self.add_ref(RefKind::Call, name, qualifier, node.start_position().row);
    }

    fn handle_type_ref(&mut self, node: Node) {
        let mut names = Vec::new();
        if node.named_child_count() == 0 {
            let t = text(node, self.src);
            if valid_ident(t) {
                names.push((t.to_string(), node.start_position().row));
            }
        } else {
            self.collect_names_all(node, &mut names);
        }
        for (name, row) in names {
            let builtin = name.len() <= 7 && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
            if !builtin {
                self.add_ref(RefKind::Ref, name, String::new(), row);
            }
        }
    }

    fn collect_names_all(&self, node: Node, out: &mut Vec<(String, usize)>) {
        if node.named_child_count() == 0 {
            let k = node.kind();
            let t = text(node, self.src);
            if (k.ends_with("identifier") || k == "name") && valid_ident(t) {
                out.push((t.to_string(), node.start_position().row));
            }
            return;
        }
        let mut cursor = node.walk();
        let children: Vec<Node> = node.named_children(&mut cursor).collect();
        for child in children {
            self.collect_names_all(child, out);
        }
    }

    fn module_doc(&self, root: Node) -> Option<String> {
        let first = root.named_child(0)?;
        if self.spec.docstring && first.kind() == "expression_statement" {
            let s = first.named_child(0)?;
            return (s.kind() == "string").then(|| clean_doc(text(s, self.src)));
        }
        if self.spec.comment_kinds.contains(&first.kind()) {
            let mut docs = Vec::new();
            let mut n = Some(first);
            let mut last_row = first.start_position().row;
            while let Some(c) = n {
                if !self.spec.comment_kinds.contains(&c.kind()) || c.start_position().row > last_row + 1 {
                    break;
                }
                let t = text(c, self.src);
                if !t.starts_with("#!") {
                    docs.push(clean_doc(t));
                }
                last_row = c.end_position().row;
                n = c.next_named_sibling();
            }
            let doc = docs.join("\n");
            return (!doc.is_empty()).then(|| truncate_chars(&doc, MAX_DOC_CHARS));
        }
        None
    }
}

fn module_symbol(prefix: &str, lang: &str, path: &str, lines: &Lines, doc: Option<String>, is_test: bool) -> Symbol {
    let count = lines.count();
    Symbol {
        id: format!("{prefix}:{path}"),
        kind: "module".to_string(),
        lang: lang.to_string(),
        path: path.to_string(),
        name: module_name(path),
        qualname: String::new(),
        container: String::new(),
        parent: None,
        start_line: 1,
        end_line: count.max(1),
        def_line: 1,
        signature: format!("module {path}"),
        doc,
        body_hash: lines.hash(1, count),
        is_test,
        fallback: false,
    }
}

/// Line-window chunks for files without a usable AST. Windows prefer to end
/// on a blank line so they do not cut through the middle of a block.
fn chunk_symbols(prefix: &str, lang: &str, path: &str, lines: &Lines, is_test: bool) -> Vec<Symbol> {
    let count = lines.count();
    let module_id = format!("{prefix}:{path}");
    let mut out = Vec::new();
    let mut start = 1;
    while start <= count {
        let mut end = (start + CHUNK_LINES - 1).min(count);
        if end < count {
            if let Some(blank) = (end.saturating_sub(15).max(start + 10)..=end)
                .rev()
                .find(|&n| lines.line(n).iter().all(|b| b.is_ascii_whitespace()))
            {
                end = blank;
            }
        }
        let first = (start..=end)
            .map(|n| String::from_utf8_lossy(lines.line(n)).trim().to_string())
            .find(|l| !l.is_empty())
            .unwrap_or_default();
        let name = format!("L{start}-{end}");
        out.push(Symbol {
            id: format!("{module_id}::{name}"),
            kind: "chunk".to_string(),
            lang: lang.to_string(),
            path: path.to_string(),
            name: name.clone(),
            qualname: name,
            container: String::new(),
            parent: Some(module_id.clone()),
            start_line: start,
            end_line: end,
            def_line: start,
            signature: truncate_chars(&first, 120),
            doc: None,
            body_hash: lines.hash(start, end),
            is_test,
            fallback: true,
        });
        start = end + 1;
    }
    out
}

/// Parse and extract one file. `None` when the file type is not indexed.
pub fn extract_file(path: &str, src: &[u8]) -> Option<FileIndex> {
    let detected = lang::detect(path, src)?;
    let lines = Lines::new(src);
    let file_is_test = is_test_path(path);
    let base = FileIndex {
        path: path.to_string(),
        lang: String::new(),
        content_hash: hash_hex(src),
        size: src.len() as u64,
        lines: lines.count(),
        fallback: false,
        is_test: file_is_test,
        shard: String::new(),
        symbols: Vec::new(),
        refs: Vec::new(),
    };
    let spec = match detected {
        Detected::Fallback(ext) => {
            let mut symbols = vec![module_symbol(&ext, &ext, path, &lines, None, file_is_test)];
            symbols.extend(chunk_symbols(&ext, &ext, path, &lines, file_is_test));
            return Some(FileIndex { lang: ext, fallback: true, symbols, ..base });
        }
        Detected::Spec(spec) => spec,
    };

    let mut parser = Parser::new();
    let tree = parser.set_language(&(spec.language)()).ok().and_then(|_| parser.parse(src, None));
    let module = |doc| module_symbol(spec.prefix, spec.name, path, &lines, doc, file_is_test);
    let Some(tree) = tree else {
        let mut symbols = vec![module(None)];
        symbols.extend(chunk_symbols(spec.prefix, spec.name, path, &lines, file_is_test));
        return Some(FileIndex { lang: spec.name.to_string(), fallback: true, symbols, ..base });
    };
    let root = tree.root_node();
    let dir = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let mut walker = Walker {
        spec,
        path,
        dir,
        src,
        file_is_test,
        symbols: Vec::new(),
        scope_parent: Vec::new(),
        id_counts: HashMap::new(),
        merge_index: HashMap::new(),
        refs: Vec::new(),
        seen_refs: HashSet::new(),
        scopes: Vec::new(),
        call_kinds: spec.calls.iter().map(|c| c.kind).collect(),
        defs_found: 0,
    };
    let doc = walker.module_doc(root);
    walker.symbols.push(module(doc));
    walker.scope_parent.push(0);
    walker.scopes.push(Scope {
        end_byte: usize::MAX,
        qual: String::new(),
        sym: 0,
        class_like: false,
        is_test: file_is_test,
    });
    walker.walk(root);

    // Syntax errors everywhere and nothing recognised: chunk it instead.
    if root.has_error() && walker.defs_found == 0 && lines.count() >= 10 {
        let mut symbols = vec![walker.symbols.swap_remove(0)];
        symbols.extend(chunk_symbols(spec.prefix, spec.name, path, &lines, file_is_test));
        return Some(FileIndex { lang: spec.name.to_string(), fallback: true, symbols, ..base });
    }

    let mut symbols = walker.symbols;
    let mut by_qual: HashMap<String, usize> = HashMap::new();
    for (i, s) in symbols.iter().enumerate().skip(1) {
        let entry = by_qual.entry(s.qualname.clone()).or_insert(i);
        if symbols[*entry].kind != "class" && s.kind == "class" {
            *entry = i;
        }
    }
    let ids: Vec<String> = symbols.iter().map(|s| s.id.clone()).collect();
    let kinds: Vec<String> = symbols.iter().map(|s| s.kind.clone()).collect();
    for i in 1..symbols.len() {
        let parent = match by_qual.get(&symbols[i].container) {
            Some(&p) if !symbols[i].container.is_empty() && p != i => p,
            _ => walker.scope_parent[i],
        };
        let s = &mut symbols[i];
        s.parent = Some(ids[parent].clone());
        if s.kind == "function" && kinds[parent] == "class" {
            s.kind = "method".to_string();
        }
        s.body_hash = lines.hash(s.start_line, s.end_line);
    }

    let mut refs: Vec<RawRef> = walker
        .refs
        .into_iter()
        .map(|(src_idx, kind, name, qualifier, line)| RawRef {
            src: ids[src_idx].clone(),
            name,
            qualifier,
            kind,
            line,
        })
        .collect();

    // Test conventions become refs too, so incremental resolution treats them uniformly.
    if file_is_test {
        let stem = module_name(path);
        let stem = stem.split('.').next().unwrap_or(&stem).to_string();
        if let Some(target) = strip_test_affixes(&stem) {
            refs.push(RawRef {
                src: ids[0].clone(),
                name: norm(&target),
                qualifier: String::new(),
                kind: RefKind::TestFile,
                line: 1,
            });
        }
    }
    for s in symbols.iter().skip(1).filter(|s| s.is_test) {
        if let Some(target) = strip_test_affixes(&s.name) {
            let key = norm(&target);
            if key.len() >= 3 {
                refs.push(RawRef {
                    src: s.id.clone(),
                    name: key,
                    qualifier: String::new(),
                    kind: RefKind::TestName,
                    line: s.def_line,
                });
            }
        }
    }
    refs.sort_by(|a, b| (a.line, &a.src, a.kind, &a.name).cmp(&(b.line, &b.src, b.kind, &b.name)));
    refs.dedup();

    Some(FileIndex { lang: spec.name.to_string(), symbols, refs, ..base })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_hash_ignores_crlf() {
        let lf = Lines::new(b"a\nb\nc\n");
        let crlf = Lines::new(b"a\r\nb\r\nc\r\n");
        assert_eq!(lf.count(), 3);
        assert_eq!(lf.hash(1, 2), crlf.hash(1, 2));
        assert_eq!(lf.hash(2, 3), hash_hex(b"b\nc"));
    }

    #[test]
    fn splits_callees() {
        assert_eq!(split_callee("compute_delay"), Some(("compute_delay".into(), "".into())));
        assert_eq!(split_callee("self.next_delay"), Some(("next_delay".into(), "self".into())));
        assert_eq!(split_callee("Duration::from_millis"), Some(("from_millis".into(), "Duration".into())));
        assert_eq!(split_callee("a.b().c"), Some(("c".into(), "?".into())));
        assert_eq!(split_callee("handlers[i]"), None);
        assert_eq!(split_callee("std::min<int>"), Some(("min".into(), "std".into())));
    }

    #[test]
    fn test_path_conventions() {
        assert!(is_test_path("tests/billing/test_retry.py"));
        assert!(is_test_path("src/retry.test.ts"));
        assert!(is_test_path("pkg/retry_test.go"));
        assert!(is_test_path("src/test/java/RetryPolicyTest.java"));
        assert!(!is_test_path("billing/retry.py"));
        assert!(!is_test_path("src/contest.py"));
        assert_eq!(strip_test_affixes("test_retry").as_deref(), Some("retry"));
        assert_eq!(strip_test_affixes("RetryPolicyTest").as_deref(), Some("RetryPolicy"));
        assert_eq!(strip_test_affixes("retry"), None);
    }

    #[test]
    fn unparseable_code_falls_back_to_chunks() {
        let src = "this is ( not { valid python ]\n".repeat(30);
        let fi = extract_file("broken.py", src.as_bytes()).unwrap();
        assert!(fi.fallback);
        assert!(fi.symbols.iter().any(|s| s.kind == "chunk"));
        assert_eq!(fi.symbols[0].kind, "module");
    }

    #[test]
    fn unknown_code_extensions_are_chunked() {
        let fi = extract_file("db/schema.sql", b"create table t (id int);\n").unwrap();
        assert!(fi.fallback);
        assert_eq!(fi.symbols[1].id, "sql:db/schema.sql::L1-1");
    }
}
