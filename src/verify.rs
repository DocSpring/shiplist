use std::fmt::Write as _;

use serde::Serialize;

use crate::check;
use crate::classify;
use crate::config::Config;

#[derive(Debug, Default, Serialize)]
pub struct Report {
    /// How many files were checked.
    pub files: usize,
    /// Files matching a `forbid` pattern.
    pub forbidden: Vec<Forbidden>,
    /// Files a list removes before this stage (or earlier).
    pub removed_but_present: Vec<Removed>,
    /// Files in no list and not matched by `build_output`.
    pub unexpected: Vec<String>,
    /// Files in more than one list.
    pub conflicts: Vec<String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Forbidden {
    pub file: String,
    pub pattern: String,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Removed {
    pub file: String,
    pub list: String,
}

impl Report {
    pub fn is_ok(&self) -> bool {
        self.removed_but_present.is_empty()
            && self.forbidden.is_empty()
            && self.unexpected.is_empty()
            && self.conflicts.is_empty()
    }

    pub fn render(&self, stage: &str) -> String {
        let mut out = String::new();
        if !self.forbidden.is_empty() {
            let _ = writeln!(out, "Forbidden files:");
            for f in &self.forbidden {
                let _ = writeln!(out, "  {}  (forbid: {})", f.file, f.pattern);
            }
            out.push('\n');
        }
        if !self.removed_but_present.is_empty() {
            let _ = writeln!(
                out,
                "Files that shouldn't be here at stage {stage}{}:",
                grouped_note(self.removed_but_present.len())
            );
            let files: Vec<(String, String)> =
                self.removed_but_present.iter().map(|r| (r.file.clone(), r.list.clone())).collect();
            for line in group_with_lists(&files) {
                let _ = writeln!(out, "  {line}");
            }
            out.push('\n');
        }
        if !self.unexpected.is_empty() {
            let _ = writeln!(
                out,
                "Files in no list and not covered by `build_output`{}:",
                grouped_note(self.unexpected.len())
            );
            for line in check::group_paths(&self.unexpected) {
                let _ = writeln!(out, "  {line}");
            }
            out.push('\n');
        }
        if !self.conflicts.is_empty() {
            let _ = writeln!(out, "Files in more than one list:");
            for f in &self.conflicts {
                let _ = writeln!(out, "  {f}");
            }
            out.push('\n');
        }
        out
    }
}

fn grouped_note(n: usize) -> &'static str {
    if n > check::GROUP_ABOVE { ", grouped by top-level directory (`--format json` lists each one)" } else { "" }
}

/// Each file with its list, up to `GROUP_ABOVE`; above that, one line per top-level directory with its file count and
/// the lists that remove them.
fn group_with_lists(files: &[(String, String)]) -> Vec<String> {
    if files.len() <= check::GROUP_ABOVE {
        return files.iter().map(|(f, list)| format!("{f}  ({list})")).collect();
    }
    // Per top-level entry: its first file, how many files, and the lists that remove them
    let mut groups: std::collections::BTreeMap<&str, (&str, usize, std::collections::BTreeSet<&str>)> =
        Default::default();
    for (file, list) in files {
        let key = file.split_once('/').map_or(file.as_str(), |(dir, _)| dir);
        let group = groups.entry(key).or_insert((file, 0, Default::default()));
        group.1 += 1;
        group.2.insert(list);
    }
    groups
        .into_iter()
        .map(|(key, (first, n, lists))| {
            let lists = lists.into_iter().collect::<Vec<_>>().join(", ");
            if n == 1 { format!("{first}  ({lists})") } else { format!("{key}/  ({n} files, {lists})") }
        })
        .collect()
}

/// The top-level directory every file is in, when there is exactly one, such as npm's `package/`.
pub fn single_top_dir(files: &[String]) -> Option<&str> {
    let first = files.first()?.split_once('/')?.0;
    files.iter().all(|f| f.split_once('/').is_some_and(|(d, _)| d == first)).then_some(first)
}

/// Checks the files actually present (in a directory or an image) against what `stage` should contain.
pub fn verify(config: &Config, files: &[String], stage: usize) -> Report {
    let mut report = Report { files: files.len(), ..Report::default() };
    for file in files {
        if let Some(p) = config.forbid.iter().find(|p| p.is_match(file)) {
            report.forbidden.push(Forbidden { file: file.clone(), pattern: p.raw.clone() });
            continue;
        }
        let lists = classify::classify(config, file);
        match lists.as_slice() {
            [id] if !classify::present_at(*id, stage) => {
                report.removed_but_present.push(Removed { file: file.clone(), list: config.list_name(*id) })
            }
            [_] => {}
            [] if config.build_output.iter().any(|p| p.is_match(file)) => {}
            [] => report.unexpected.push(file.clone()),
            _ => report.conflicts.push(file.clone()),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_files_by_directory_with_their_lists() {
        let few: Vec<(String, String)> = vec![("spec/a.rb".into(), "remove_before.package".into())];
        assert_eq!(group_with_lists(&few), ["spec/a.rb  (remove_before.package)"]);
        let mut many: Vec<(String, String)> =
            (0..25).map(|i| (format!("spec/{i}.rb"), "remove_before.package".into())).collect();
        many.push(("docs/a.md".into(), "remove_before.test".into()));
        many.push(("Rakefile".into(), "remove_before.package".into()));
        assert_eq!(
            group_with_lists(&many),
            [
                "Rakefile  (remove_before.package)",
                "docs/a.md  (remove_before.test)",
                "spec/  (25 files, remove_before.package)"
            ]
        );
    }

    #[test]
    fn finds_a_single_top_level_directory() {
        let files = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(single_top_dir(&files(&["package/a.js", "package/lib/b.js"])), Some("package"));
        assert_eq!(single_top_dir(&files(&["package/a.js", "README.md"])), None);
        assert_eq!(single_top_dir(&files(&["a/x", "b/y"])), None);
        assert_eq!(single_top_dir(&[]), None);
    }
}
