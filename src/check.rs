use std::fmt::Write as _;

use crate::classify;
use crate::config::Config;

#[derive(Debug, Default)]
pub struct Report {
    pub unclassified: Vec<String>,
    /// A file and the patterns that put it in each of its lists.
    pub conflicts: Vec<(String, Vec<String>)>,
    /// `list.part: pattern` for patterns that match no tracked file.
    pub stale: Vec<String>,
    /// `list.exclude: pattern` for exceptions that take no file out of their own list.
    pub dead_excludes: Vec<String>,
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
            let _ = writeln!(out, "Files in no list (add each to exactly one list):");
            for f in &self.unclassified {
                let _ = writeln!(out, "  {f}");
            }
            out.push('\n');
        }
        if !self.conflicts.is_empty() {
            let _ = writeln!(out, "Files in more than one list (each file belongs to exactly one):");
            for (f, lists) in &self.conflicts {
                let _ = writeln!(out, "  {f}");
                for l in lists {
                    let _ = writeln!(out, "    {l}");
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
    let mut report = Report::default();

    for file in files {
        let matches = classify::matches(config, file);
        let claiming: Vec<_> = matches.iter().filter(|m| m.claims()).collect();
        match claiming.len() {
            0 => report.unclassified.push(file.clone()),
            1 => {}
            _ => report.conflicts.push((
                file.clone(),
                claiming
                    .iter()
                    .map(|m| format!("{} (include: {})", config.list_name(m.list.id), m.includes.join(", ")))
                    .collect(),
            )),
        }
    }

    for list in &config.lists {
        let name = config.list_name(list.id);
        for (part, entries) in [("include", &list.include), ("exclude", &list.exclude)] {
            for entry in entries.iter().filter(|e| !e.untracked) {
                let matched: Vec<&String> = files.iter().filter(|f| entry.pattern.is_match(f)).collect();
                if matched.is_empty() {
                    report.stale.push(format!("{name}.{part}: {}", entry.pattern.raw));
                } else if part == "exclude"
                    && !matched.iter().any(|f| list.include.iter().any(|inc| inc.pattern.is_match(f)))
                {
                    report.dead_excludes.push(format!("{name}.exclude: {}", entry.pattern.raw));
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
        assert_eq!(r.conflicts.len(), 1);
        assert_eq!(r.conflicts[0].0, "bin/setup");
        assert_eq!(r.conflicts[0].1, ["remove_before.production (include: bin/)", "keep (include: bin/)"]);
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
        assert_eq!(r.dead_excludes, ["keep.exclude: docs/"]);
        let c = Config::parse(CONFIG).unwrap();
        let r =
            check(&c, &files(&[".env", ".shiplist.yml", "app/assets/logo.svg", "spec/a.rb", "bin/rails", "app/a.rb"]));
        assert_eq!(r.stale, ["remove_before.test.include: docs/"]);
    }
}
