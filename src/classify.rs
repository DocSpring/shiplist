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
