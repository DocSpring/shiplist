use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};

use crate::classify;
use crate::config::{Config, ListId};

#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// Directories to delete with everything in them: every listed file under them is removed.
    pub dirs: Vec<String>,
    /// Files outside those directories.
    pub files: Vec<String>,
}

/// The files and directories to delete so that `files` look the way they will at `stage`.
///
/// Every file must have exactly one list, or nothing is removed. With `allow_build_output` (a directory walk, not git's
/// list), a file in no list is allowed when a `build_output` pattern matches it, and it stays.
pub fn plan(config: &Config, files: &[String], stage: usize, allow_build_output: bool) -> Result<Plan> {
    let mut removed = BTreeSet::new();
    let mut problems = Vec::new();
    for file in files {
        let lists = classify::classify(config, file);
        match lists.as_slice() {
            [id] => {
                if !classify::present_at(*id, stage) {
                    removed.insert(file.as_str());
                }
            }
            [] if allow_build_output && config.build_output.iter().any(|p| p.is_match(file)) => {}
            [] => problems.push(format!("{file}: in no list")),
            _ => problems.push(format!(
                "{file}: in {}",
                lists.iter().map(|id| config.list_name(*id)).collect::<Vec<_>>().join(" and ")
            )),
        }
    }
    if !problems.is_empty() {
        anyhow::bail!(
            "refusing to remove anything until every file has exactly one list:\n  {}",
            problems.join("\n  ")
        );
    }

    // For each directory: how many files are under it, and how many of those are removed.
    let mut counts: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for file in files {
        let is_removed = removed.contains(file.as_str());
        for (i, _) in file.match_indices('/') {
            let c = counts.entry(&file[..i]).or_default();
            c.0 += 1;
            if is_removed {
                c.1 += 1;
            }
        }
    }
    let mut dirs: Vec<String> = Vec::new();
    for (dir, (total, gone)) in &counts {
        if total == gone && !dirs.iter().any(|d| dir.starts_with(d.as_str()) && dir[d.len()..].starts_with('/')) {
            dirs.push(dir.to_string());
        }
    }
    let files = removed
        .into_iter()
        .filter(|f| !dirs.iter().any(|d| f.starts_with(d.as_str()) && f[d.len()..].starts_with('/')))
        .map(String::from)
        .collect();
    Ok(Plan { dirs, files })
}

/// Whether a list removed at or before `stage` has an `untracked: true` glob (not a `dir/`), which needs a walk.
pub fn has_untracked_globs(config: &Config, stage: usize) -> bool {
    config.lists.iter().any(|list| {
        matches!(list.id, ListId::RemoveBefore(at) if at <= stage)
            && list.include.iter().any(|e| e.untracked && e.pattern.dir().is_none())
    })
}

/// Paths named by `untracked: true` patterns in the lists removed at or before `stage`, which exist under `root`.
pub fn untracked_paths(config: &Config, root: &Path, stage: usize, walked: &[String]) -> Vec<String> {
    let mut out = BTreeSet::new();
    for list in &config.lists {
        let ListId::RemoveBefore(at) = list.id else { continue };
        if at > stage {
            continue;
        }
        for entry in list.include.iter().filter(|e| e.untracked) {
            match entry.pattern.dir() {
                Some(dir) => {
                    if root.join(dir).symlink_metadata().is_ok() {
                        out.insert(dir.to_string());
                    }
                }
                None => out.extend(walked.iter().filter(|f| entry.pattern.is_match(f)).cloned()),
            }
        }
    }
    out.into_iter().collect()
}

pub fn remove(root: &Path, path: &str) -> Result<bool> {
    let full = root.join(path);
    match full.symlink_metadata() {
        Err(_) => Ok(false),
        Ok(meta) if meta.is_dir() => {
            std::fs::remove_dir_all(&full).with_context(|| format!("couldn't remove {}", full.display()))?;
            Ok(true)
        }
        Ok(_) => {
            std::fs::remove_file(&full).with_context(|| format!("couldn't remove {}", full.display()))?;
            Ok(true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_whole_directories_where_it_can() {
        let c = Config::parse(
            r#"
version: 1
stages: [test, production]
remove_before:
  test:
    include: [docs/, lib/dev.rb]
  production:
    include: [spec/]
keep:
  include: [lib/, app.rb]
  exclude: [lib/dev.rb]
"#,
        )
        .unwrap();
        let files: Vec<String> = ["app.rb", "docs/a.md", "docs/b/c.md", "lib/dev.rb", "lib/x.rb", "spec/a.rb"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            plan(&c, &files, 0, false).unwrap(),
            Plan { dirs: vec!["docs".into()], files: vec!["lib/dev.rb".into()] }
        );
        assert_eq!(
            plan(&c, &files, 1, false).unwrap(),
            Plan { dirs: vec!["docs".into(), "spec".into()], files: vec!["lib/dev.rb".into()] }
        );
        let mut bad = files.clone();
        bad.push("README.md".into());
        assert!(format!("{:#}", plan(&c, &bad, 0, false).unwrap_err()).contains("README.md: in no list"));
    }
}
