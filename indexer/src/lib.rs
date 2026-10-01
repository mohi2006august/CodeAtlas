//! CodeAtlas structural indexer: tree-sitter parsing into symbol-level chunks,
//! a cross-reference graph, and incremental updates stored in SQLite.

pub mod extract;
pub mod git;
pub mod graph;
pub mod imports;
pub mod index;
pub mod lang;
pub mod model;
pub mod store;
pub mod walk;
