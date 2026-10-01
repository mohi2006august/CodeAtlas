//! Thin wrappers over the `git` CLI. Paths are relative to the indexed root
//! (`--relative`), so indexing a subdirectory of a repository works.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .context("running git")?;
    if !out.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(out.stdout)
}

fn split_z(bytes: Vec<u8>) -> Vec<String> {
    bytes
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).replace('\\', "/"))
        .collect()
}

pub fn is_repo(root: &Path) -> bool {
    git(root, &["rev-parse", "--is-inside-work-tree"])
        .map(|o| o.starts_with(b"true"))
        .unwrap_or(false)
}

pub fn head(root: &Path) -> Option<String> {
    let out = git(root, &["rev-parse", "HEAD"]).ok()?;
    let sha = String::from_utf8_lossy(&out).trim().to_string();
    (!sha.is_empty()).then_some(sha)
}

pub fn commit_exists(root: &Path, sha: &str) -> bool {
    git(root, &["cat-file", "-e", &format!("{sha}^{{commit}}")]).is_ok()
}

/// Tracked files plus untracked files that are not ignored.
pub fn ls_files(root: &Path) -> Result<Vec<String>> {
    Ok(split_z(git(root, &["ls-files", "-z", "--cached", "--others", "--exclude-standard"])?))
}

pub fn untracked(root: &Path) -> Result<Vec<String>> {
    Ok(split_z(git(root, &["ls-files", "-z", "--others", "--exclude-standard"])?))
}

/// Files that differ between `commit` and the working tree, plus untracked files.
pub fn changed_since(root: &Path, commit: &str) -> Result<Vec<String>> {
    let mut paths =
        split_z(git(root, &["diff", "--relative", "--name-only", "-z", "--no-renames", commit])?);
    paths.extend(untracked(root)?);
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Uncommitted changes relative to HEAD (modified, staged and untracked).
pub fn dirty(root: &Path) -> Result<Vec<String>> {
    let mut paths = match head(root) {
        Some(_) => split_z(git(root, &["diff", "--relative", "--name-only", "-z", "--no-renames", "HEAD"])?),
        None => Vec::new(),
    };
    paths.extend(untracked(root)?);
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Number of commits reachable from `to` but not from `from`.
pub fn commits_between(root: &Path, from: &str, to: &str) -> Option<u64> {
    let out = git(root, &["rev-list", "--count", &format!("{from}..{to}")]).ok()?;
    String::from_utf8_lossy(&out).trim().parse().ok()
}
