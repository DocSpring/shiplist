use serde::Serialize;

use crate::config::{Config, List, ListId};

/// How one list sees one path.
#[derive(Debug)]
pub struct ListMatch<'a> {
    pub list: &'a List,
    pub includes: Vec<&'a str>,
    pub excludes: Vec<&'a str>,
}

impl ListMatch<'_> {
    /// The path is in this list: an include matches and no exclude does.
    pub fn claims(&self) -> bool {
        !self.includes.is_empty() && self.excludes.is_empty()
    }
}

/// Every list's view of a path, including lists whose excludes take it back out.
pub fn matches<'a>(config: &'a Config, path: &str) -> Vec<ListMatch<'a>> {
    config
        .lists
        .iter()
        .map(|list| ListMatch {
            list,
            includes: list
                .include
                .iter()
                .filter(|e| e.pattern.is_match(path))
                .map(|e| e.pattern.raw.as_str())
                .collect(),
            excludes: list
                .exclude
                .iter()
                .filter(|e| e.pattern.is_match(path))
                .map(|e| e.pattern.raw.as_str())
                .collect(),
        })
        .collect()
}

/// The lists that claim a path. A correct config gives every tracked file exactly one.
pub fn classify(config: &Config, path: &str) -> Vec<ListId> {
    config
        .lists
        .iter()
        .filter(|list| {
            list.include.iter().any(|e| e.pattern.is_match(path))
                && !list.exclude.iter().any(|e| e.pattern.is_match(path))
        })
        .map(|list| list.id)
        .collect()
}

/// Whether a file in this list is still there at the stage with this index.
pub fn present_at(id: ListId, stage: usize) -> bool {
    match id {
        ListId::Keep => true,
        ListId::RemoveBefore(removed_at) => stage < removed_at,
    }
}

/// What `explain` reports for one path.
#[derive(Debug, Serialize)]
pub struct Explanation {
    pub path: String,
    /// Every list with an include pattern that matches the path.
    pub matches: Vec<ExplainedList>,
    /// The lists the path is in: exactly one when the config is right.
    pub lists: Vec<String>,
    /// For each stage, whether the path is still there.
    pub stages: Vec<StagePresence>,
}

#[derive(Debug, Serialize)]
pub struct ExplainedList {
    pub list: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// The path is in this list: an include matches and no exclude does.
    pub claims: bool,
}

#[derive(Debug, Serialize)]
pub struct StagePresence {
    pub stage: String,
    pub present: bool,
}

pub fn explain(config: &Config, path: &str) -> Explanation {
    let matches = matches(config, path);
    let claiming: Vec<ListId> = matches.iter().filter(|m| m.claims()).map(|m| m.list.id).collect();
    Explanation {
        path: path.to_string(),
        matches: matches
            .iter()
            .filter(|m| !m.includes.is_empty())
            .map(|m| ExplainedList {
                list: config.list_name(m.list.id),
                include: m.includes.iter().map(|s| s.to_string()).collect(),
                exclude: m.excludes.iter().map(|s| s.to_string()).collect(),
                claims: m.claims(),
            })
            .collect(),
        lists: claiming.iter().map(|id| config.list_name(*id)).collect(),
        stages: match claiming.as_slice() {
            [id] => config
                .stages
                .iter()
                .enumerate()
                .map(|(i, stage)| StagePresence { stage: stage.clone(), present: present_at(*id, i) })
                .collect(),
            _ => Vec::new(),
        },
    }
}
