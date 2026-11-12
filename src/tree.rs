use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use walkdir::WalkDir;

fn git(dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .with_context(|| format!("couldn't run git in {}", dir.display()))?;
    if !output.status.success() {
        bail!("git {} failed in {}: {}", args.join(" "), dir.display(), String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(output.stdout)
}

/// Every file git tracks, relative to `dir`, which must be the top of a git working tree.
pub fn tracked_files(dir: &Path) -> Result<Vec<String>> {
    let top = String::from_utf8(git(dir, &["rev-parse", "--show-toplevel"])?)?;
    let top = Path::new(top.trim()).canonicalize()?;
    let dir_abs = dir.canonicalize().with_context(|| format!("{} doesn't exist", dir.display()))?;
    if top != dir_abs {
        bail!("{} isn't the top of its git working tree ({}): run shiplist there", dir.display(), top.display());
    }
    let out = git(dir, &["ls-files", "-z"])?;
    let mut files: Vec<String> = out
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8(s.to_vec()).context("git listed a path that isn't UTF-8"))
        .collect::<Result<_>>()?;
    files.sort();
    files.dedup();
    Ok(files)
}

/// Whether the working tree has uncommitted changes to tracked files, or untracked files git doesn't ignore.
pub fn is_dirty(dir: &Path) -> Result<bool> {
    Ok(!git(dir, &["status", "--porcelain"])?.is_empty())
}

/// Every file and symlink under `dir` (not directories), relative to it, skipping any `.git` directory.
pub fn walk_files(dir: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    for entry in WalkDir::new(dir).follow_links(false).into_iter().filter_entry(|e| e.file_name() != ".git") {
        let entry = entry?;
        if entry.file_type().is_dir() {
            continue;
        }
        let rel = entry.path().strip_prefix(dir)?;
        let rel = rel.to_str().with_context(|| format!("path isn't UTF-8: {}", rel.display()))?;
        files.push(rel.replace(std::path::MAIN_SEPARATOR, "/"));
    }
    files.sort();
    Ok(files)
}
