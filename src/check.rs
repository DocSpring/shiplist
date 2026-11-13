use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

use crate::classify;
use crate::config::Config;

#[derive(Debug, Default, Serialize)]
pub struct Report {
    /// How many tracked files were checked.
    pub files: usize,
    pub unclassified: Vec<String>,
    pub conflicts: Vec<Conflict>,
    /// Patterns that match no tracked file.
    pub stale: Vec<PatternRef>,
    /// Exceptions that take no file out of their own list.
    pub dead_excludes: Vec<PatternRef>,
}

/// A file in more than one list.
#[derive(Debug, Serialize, PartialEq)]
pub struct Conflict {
    pub file: String,
    pub lists: Vec<ListHit>,
}

/// A list that claims a file, and the include patterns that matched it.
#[derive(Debug, Serialize, PartialEq)]
pub struct ListHit {
    pub list: String,
    pub include: Vec<String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct PatternRef {
    pub list: String,
    /// `include` or `exclude`
    pub part: &'static str,
    pub pattern: String,
}

impl std::fmt::Display for PatternRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}: {}", self.list, self.part, self.pattern)
    }
}

/// Above this many unclassified files, the text output groups them by top-level directory.
pub const GROUP_ABOVE: usize = 20;

/// Lists up to `GROUP_ABOVE` paths one by one; above that, one line per top-level directory with its file count, and
/// files at the root one by one.
pub fn group_paths(paths: &[String]) -> Vec<String> {
    if paths.len() <= GROUP_ABOVE {
        return paths.to_vec();
    }
    let mut groups: BTreeMap<&str, Vec<&String>> = BTreeMap::new();
    let mut root = Vec::new();
    for path in paths {
        match path.split_once('/') {
            Some((dir, _)) => groups.entry(dir).or_default().push(path),
            None => root.push(path.clone()),
        }
    }
    let mut lines: Vec<String> = groups
        .into_iter()
        .map(|(dir, files)| match files.as_slice() {
            [only] => only.to_string(),
            _ => format!("{dir}/  ({} files)", files.len()),
        })
        .collect();
    lines.extend(root);
    lines.sort();
    lines
}

impl Report {
    pub fn is_ok(&self) -> bool {
        self.unclassified.is_empty()
            && self.conflicts.is_empty()
            && self.stale.is_empty()
            && self.dead_excludes.is_empty()
    }

    pub fn render(&self) -> String {
        let mut out = String::new();
        if !self.unclassified.is_empty() {
            if self.unclassified.len() > GROUP_ABOVE {
                let _ = writeln!(
                    out,
                    "{} files in no list, grouped by top-level directory (`shiplist check --format json` lists each \
                     one). Add each file to exactly one list:",
                    self.unclassified.len()
                );
            } else {
                let _ = writeln!(out, "Files in no list (add each to exactly one list):");
            }
            for line in group_paths(&self.unclassified) {
                let _ = writeln!(out, "  {line}");
            }
            out.push('\n');
        }
        if !self.conflicts.is_empty() {
            let _ = writeln!(out, "Files in more than one list (each file belongs to exactly one):");
            for c in &self.conflicts {
                let _ = writeln!(out, "  {}", c.file);
                for hit in &c.lists {
                    let _ = writeln!(out, "    {} (include: {})", hit.list, hit.include.join(", "));
                }
            }
            out.push('\n');
        }
        if !self.stale.is_empty() {
            let _ = writeln!(
                out,
                "Patterns that match no tracked file (delete them, or mark one `untracked: true` if git never has it):"
            );
            for p in &self.stale {
                let _ = writeln!(out, "  {p}");
            }
            out.push('\n');
        }
        if !self.dead_excludes.is_empty() {
            let _ = writeln!(out, "Exceptions that match none of their own list's files (delete them):");
            for p in &self.dead_excludes {
                let _ = writeln!(out, "  {p}");
            }
            out.push('\n');
        }
        out
    }
}

pub fn check(config: &Config, files: &[String]) -> Report {
    let mut report = Report { files: files.len(), ..Report::default() };

    for file in files {
        let matches = classify::matches(config, file);
        let claiming: Vec<_> = matches.iter().filter(|m| m.claims()).collect();
        match claiming.len() {
            0 => report.unclassified.push(file.clone()),
            1 => {}
            _ => report.conflicts.push(Conflict {
                file: file.clone(),
                lists: claiming
                    .iter()
                    .map(|m| ListHit {
                        list: config.list_name(m.list.id),
                        include: m.includes.iter().map(|s| s.to_string()).collect(),
                    })
                    .collect(),
            }),
        }
    }

    for list in &config.lists {
        let name = config.list_name(list.id);
        for (part, entries) in [("include", &list.include), ("exclude", &list.exclude)] {
            for entry in entries.iter().filter(|e| !e.untracked) {
                let matched: Vec<&String> = files.iter().filter(|f| entry.pattern.is_match(f)).collect();
                let pattern_ref = || PatternRef { list: name.clone(), part, pattern: entry.pattern.raw.clone() };
                if matched.is_empty() {
                    report.stale.push(pattern_ref());
                } else if part == "exclude"
                    && !matched.iter().any(|f| list.include.iter().any(|inc| inc.pattern.is_match(f)))
                {
                    report.dead_excludes.push(pattern_ref());
                }
            }
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    const CONFIG: &str = r#"
version: 1
stages: [test, production]
remove_before:
  test:
    include: ['.*', docs/, app/assets/]
    exclude: [.shiplist.yml]
  production:
    include: [.shiplist.yml, spec/, bin/]
    exclude: [bin/rails]
keep:
  include: [app/, bin/rails]
  exclude: [app/assets/]
"#;

    #[test]
    fn a_complete_config_passes() {
        let c = Config::parse(CONFIG).unwrap();
        let r = check(
            &c,
            &files(&[
                ".env",
                ".shiplist.yml",
                "docs/a.md",
                "app/assets/logo.svg",
                "spec/a.rb",
                "bin/rails",
                "bin/setup",
                "app/a.rb",
            ]),
        );
        assert!(r.is_ok(), "{}", r.render());
        assert_eq!(r.files, 8);
    }

    #[test]
    fn reports_each_problem_once() {
        let c = Config::parse(&CONFIG.replace("include: [app/, bin/rails]", "include: [app/, bin/]")).unwrap();
        let r = check(
            &c,
            &files(&[
                ".env",
                ".shiplist.yml",
                "docs/a.md",
                "app/assets/logo.svg",
                "spec/a.rb",
                "bin/rails",
                "bin/setup",
                "app/a.rb",
                "Gemfile",
            ]),
        );
        assert_eq!(r.unclassified, ["Gemfile"]);
        assert_eq!(
            r.conflicts,
            [Conflict {
                file: "bin/setup".into(),
                lists: vec![
                    ListHit { list: "remove_before.production".into(), include: vec!["bin/".into()] },
                    ListHit { list: "keep".into(), include: vec!["bin/".into()] },
                ],
            }]
        );
        assert!(r.stale.is_empty());
    }

    #[test]
    fn stale_patterns_and_dead_excludes() {
        let c = Config::parse(&CONFIG.replace("exclude: [app/assets/]", "exclude: [app/assets/, docs/]")).unwrap();
        let r = check(
            &c,
            &files(&[
                ".env",
                ".shiplist.yml",
                "docs/a.md",
                "app/assets/logo.svg",
                "spec/a.rb",
                "bin/rails",
                "app/a.rb",
            ]),
        );
        assert_eq!(r.dead_excludes.iter().map(|p| p.to_string()).collect::<Vec<_>>(), ["keep.exclude: docs/"]);
        let c = Config::parse(CONFIG).unwrap();
        let r =
            check(&c, &files(&[".env", ".shiplist.yml", "app/assets/logo.svg", "spec/a.rb", "bin/rails", "app/a.rb"]));
        assert_eq!(r.stale.iter().map(|p| p.to_string()).collect::<Vec<_>>(), ["remove_before.test.include: docs/"]);
    }

    #[test]
    fn groups_many_paths_by_top_level_directory() {
        let few = files(&["a.txt", "docs/a.md"]);
        assert_eq!(group_paths(&few), few);
        let mut many: Vec<String> = (0..30).map(|i| format!("docs/{i}.md")).collect();
        many.push("scripts/only.sh".into());
        many.push("Makefile".into());
        assert_eq!(group_paths(&many), ["Makefile", "docs/  (30 files)", "scripts/only.sh"]);
    }
}
