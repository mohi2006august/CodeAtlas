//! Data model shared by the parser, the store and the graph resolver.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A symbol-level chunk. `id` is stable and human readable:
/// `py:billing/retry.py::RetryPolicy.next_delay` (modules omit the `::` part).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Symbol {
    pub id: String,
    /// function | method | class | module | chunk
    pub kind: String,
    pub lang: String,
    pub path: String,
    pub name: String,
    pub qualname: String,
    /// Qualified name of the enclosing scope ("" for top level).
    pub container: String,
    pub parent: Option<String>,
    /// 1-based inclusive line span, including leading doc comments.
    pub start_line: u32,
    pub end_line: u32,
    /// Line on which the definition itself starts.
    pub def_line: u32,
    pub signature: String,
    pub doc: Option<String>,
    pub body_hash: String,
    pub is_test: bool,
    /// True for line-window chunks produced when no AST was available.
    pub fallback: bool,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    Call,
    Inherit,
    Ref,
    Import,
    TestName,
    TestFile,
}

impl RefKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RefKind::Call => "call",
            RefKind::Inherit => "inherit",
            RefKind::Ref => "ref",
            RefKind::Import => "import",
            RefKind::TestName => "test_name",
            RefKind::TestFile => "test_file",
        }
    }

    pub fn parse(s: &str) -> Option<RefKind> {
        Some(match s {
            "call" => RefKind::Call,
            "inherit" => RefKind::Inherit,
            "ref" => RefKind::Ref,
            "import" => RefKind::Import,
            "test_name" => RefKind::TestName,
            "test_file" => RefKind::TestFile,
            _ => return None,
        })
    }
}

/// An unresolved reference found in source. The graph resolver turns these
/// into edges once the whole symbol table is known.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RawRef {
    pub src: String,
    /// Lookup key: a symbol name, or a normalised name for import/test refs.
    pub name: String,
    /// Dotted receiver for calls (`self`, `RetryPolicy`, `?` when complex) or
    /// the slash-separated target path for imports.
    pub qualifier: String,
    pub kind: RefKind,
    pub line: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FileIndex {
    pub path: String,
    pub lang: String,
    pub content_hash: String,
    pub size: u64,
    pub lines: u32,
    pub fallback: bool,
    pub is_test: bool,
    pub shard: String,
    pub symbols: Vec<Symbol>,
    pub refs: Vec<RawRef>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Edge {
    pub src: String,
    pub dst: String,
    /// calls | imports | inherits | tests | references
    #[serde(rename = "type")]
    pub kind: String,
    pub weight: f64,
}

/// First 16 bytes of SHA-256 as hex. Short enough to read, long enough to key caches.
pub fn hash_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(32);
    for b in &digest[..16] {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Case- and separator-insensitive key used to match `RetryPolicy`,
/// `retry_policy` and `retry-policy` against each other.
pub fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Languages whose files can import each other.
pub fn family(lang: &str) -> &'static str {
    match lang {
        "javascript" | "typescript" | "tsx" => "js",
        "c" | "cpp" | "objc" => "c",
        "java" | "kotlin" | "scala" => "jvm",
        "python" => "python",
        "rust" => "rust",
        "go" => "go",
        "csharp" => "csharp",
        "ruby" => "ruby",
        "php" => "php",
        "swift" => "swift",
        "bash" => "bash",
        "lua" => "lua",
        "elixir" => "elixir",
        "haskell" => "haskell",
        "ocaml" => "ocaml",
        "zig" => "zig",
        "dart" => "dart",
        "r" => "r",
        "julia" => "julia",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn norm_ignores_case_and_separators() {
        assert_eq!(norm("RetryPolicy"), norm("retry_policy"));
        assert_eq!(norm("retry-policy"), "retrypolicy");
    }

    #[test]
    fn hash_is_stable_and_short() {
        assert_eq!(hash_hex(b"abc").len(), 32);
        assert_eq!(hash_hex(b"abc"), hash_hex(b"abc"));
        assert_ne!(hash_hex(b"abc"), hash_hex(b"abd"));
    }
}
