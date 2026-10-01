//! `codeatlas-indexer`: command-line entry point for the Rust indexer.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use indexer::index::{self, IndexOptions, Mode};
use indexer::lang::{self, Detected};

#[derive(Parser)]
#[command(name = "codeatlas-indexer", version, about = "CodeAtlas structural indexer")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Index a repository (incremental by default) and print a JSON report.
    Index {
        repo: PathBuf,
        /// SQLite database path (default: <repo>/.codeatlas/atlas.db).
        #[arg(long)]
        db: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "auto")]
        mode: Mode,
        /// Skip files larger than this many KiB.
        #[arg(long, default_value_t = 1024)]
        max_file_kb: u64,
        /// Extra gitignore-style patterns to exclude (repeatable).
        #[arg(long = "exclude")]
        excludes: Vec<String>,
        /// Directory depth used as the shard key for monorepos.
        #[arg(long, default_value_t = 1)]
        shard_depth: usize,
    },
    /// List supported languages as JSON.
    Languages,
    /// Print the tree-sitter syntax tree of a file (debugging aid for grammar rules).
    Sexp { file: PathBuf },
    /// Extract symbols and unresolved references from one file as JSON.
    Parse {
        file: PathBuf,
        /// Path to report in symbol ids (defaults to the file path as given).
        #[arg(long)]
        as_path: Option<String>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Index { repo, db, mode, max_file_kb, excludes, shard_depth } => {
            let db = db.unwrap_or_else(|| repo.join(".codeatlas").join("atlas.db"));
            let report = index::run(&IndexOptions {
                root: repo,
                db,
                mode,
                max_file_bytes: max_file_kb * 1024,
                excludes,
                shard_depth,
            })?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        Command::Languages => {
            let langs: Vec<_> = lang::specs()
                .iter()
                .map(|s| serde_json::json!({"name": s.name, "prefix": s.prefix, "extensions": s.exts}))
                .collect();
            println!("{}", serde_json::to_string_pretty(&langs)?);
        }
        Command::Sexp { file } => {
            let src = std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let path = file.to_string_lossy().replace('\\', "/");
            let Some(Detected::Spec(spec)) = lang::detect(&path, &src) else {
                bail!("no grammar for {path}");
            };
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&(spec.language)())?;
            let tree = parser.parse(&src, None).context("parse failed")?;
            println!("{}", tree.root_node().to_sexp());
        }
        Command::Parse { file, as_path } => {
            let src = std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let path = as_path.unwrap_or_else(|| file.to_string_lossy().replace('\\', "/"));
            let Some(index) = indexer::extract::extract_file(&path, &src) else {
                bail!("{path} is not an indexed file type");
            };
            println!("{}", serde_json::to_string_pretty(&index)?);
        }
    }
    Ok(())
}
