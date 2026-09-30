//! `codeatlas-indexer`: command-line entry point for the Rust indexer.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use indexer::lang::{self, Detected};

#[derive(Parser)]
#[command(name = "codeatlas-indexer", version, about = "CodeAtlas structural indexer")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List supported languages as JSON.
    Languages,
    /// Print the tree-sitter syntax tree of a file (debugging aid for grammar rules).
    Sexp { file: PathBuf },
}

fn main() -> Result<()> {
    match Cli::parse().command {
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
    }
    Ok(())
}
