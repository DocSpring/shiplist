use std::fmt::Write as _;

use crate::classify;
use crate::config::Config;

#[derive(Debug, Default)]
pub struct Report {
    /// Files a list removes before this stage (or earlier), with that list's name.
    pub removed_but_present: Vec<(String, String)>,
    /// Files matching a `forbid` pattern, with the pattern.
    pub forbidden: Vec<(String, String)>,
    /// Files in no list and not matched by `build_output`.
    pub unexpected: Vec<String>,
    /// Files in more than one list.
    pub conflicts: Vec<String>,
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
            for (f, p) in &self.forbidden {
                let _ = writeln!(out, "  {f}  (forbid: {p})");
            }
            out.push('\n');
        }
        if !self.removed_but_present.is_empty() {
            let _ = writeln!(out, "Files that shouldn't be here at stage {stage}:");
            for (f, list) in &self.removed_but_present {
                let _ = writeln!(out, "  {f}  ({list})");
            }
            out.push('\n');
        }
        if !self.unexpected.is_empty() {
            let _ = writeln!(out, "Files in no list and not covered by `build_output`:");
            for f in &self.unexpected {
                let _ = writeln!(out, "  {f}");
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

/// Checks the files actually present (in a directory or an image) against what `stage` should contain.
pub fn verify(config: &Config, files: &[String], stage: usize) -> Report {
    let mut report = Report::default();
    for file in files {
        if let Some(p) = config.forbid.iter().find(|p| p.is_match(file)) {
            report.forbidden.push((file.clone(), p.raw.clone()));
            continue;
        }
        let lists = classify::classify(config, file);
        match lists.as_slice() {
            [id] if !classify::present_at(*id, stage) => {
                report.removed_but_present.push((file.clone(), config.list_name(*id)))
            }
            [_] => {}
            [] if config.build_output.iter().any(|p| p.is_match(file)) => {}
            [] => report.unexpected.push(file.clone()),
            _ => report.conflicts.push(file.clone()),
        }
    }
    report
}
