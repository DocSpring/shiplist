use std::fmt::Write as _;

use serde::Serialize;

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
            let _ = writeln!(out, "Files that shouldn't be here at stage {stage}:");
            for r in &self.removed_but_present {
                let _ = writeln!(out, "  {}  ({})", r.file, r.list);
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
