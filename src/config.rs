use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::pattern::Pattern;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    version: u32,
    stages: Vec<String>,
    #[serde(default)]
    remove_before: BTreeMap<String, RawList>,
    keep: RawList,
    #[serde(default)]
    forbid: Vec<String>,
    #[serde(default)]
    build_output: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawList {
    #[serde(default)]
    include: Vec<RawEntry>,
    #[serde(default)]
    exclude: Vec<RawEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawEntry {
    Path(String),
    Detailed(RawDetailed),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDetailed {
    path: String,
    #[serde(default)]
    untracked: bool,
}

/// A pattern in one of the lists.
#[derive(Debug)]
pub struct Entry {
    pub pattern: Pattern,
    /// git never tracks what this pattern names (node_modules/): it may match no tracked file, and `prune` deletes
    /// whatever exists on disk.
    pub untracked: bool,
}

/// Which list a file is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ListId {
    /// Removed before the stage at this index of `Config::stages`.
    RemoveBefore(usize),
    Keep,
}

#[derive(Debug)]
pub struct List {
    pub id: ListId,
    pub include: Vec<Entry>,
    pub exclude: Vec<Entry>,
}

#[derive(Debug)]
pub struct Config {
    pub stages: Vec<String>,
    /// One list per stage, in stage order, then `keep`.
    pub lists: Vec<List>,
    pub forbid: Vec<Pattern>,
    pub build_output: Vec<Pattern>,
}

pub const DEFAULT_PATH: &str = ".shiplist.yml";

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("couldn't read {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("invalid config in {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let raw: RawConfig = serde_saphyr::from_str(text)?;
        if raw.version != 1 {
            bail!("unsupported version {} (this shiplist reads version 1)", raw.version);
        }
        if raw.stages.is_empty() {
            bail!("`stages` is empty: name at least one stage, such as [test, production]");
        }
        for (i, stage) in raw.stages.iter().enumerate() {
            if stage.is_empty() || !stage.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                bail!("stage name {stage:?} must be letters, digits, '-' or '_'");
            }
            if raw.stages[..i].contains(stage) {
                bail!("stage {stage:?} is listed twice in `stages`");
            }
        }
        for name in raw.remove_before.keys() {
            if !raw.stages.contains(name) {
                bail!("`remove_before.{name}` names a stage that isn't in `stages` {:?}", raw.stages);
            }
        }
        let mut remove_before = raw.remove_before;
        let mut lists = Vec::new();
        for (i, stage) in raw.stages.iter().enumerate() {
            let Some(list) = remove_before.remove(stage) else {
                bail!("stage {stage:?} has no `remove_before.{stage}` list");
            };
            lists.push(build_list(ListId::RemoveBefore(i), list, &format!("remove_before.{stage}"))?);
        }
        lists.push(build_list(ListId::Keep, raw.keep, "keep")?);
        let forbid = raw.forbid.iter().map(|p| Pattern::new(p)).collect::<Result<_>>().context("in `forbid`")?;
        let build_output =
            raw.build_output.iter().map(|p| Pattern::new(p)).collect::<Result<_>>().context("in `build_output`")?;
        Ok(Config { stages: raw.stages, lists, forbid, build_output })
    }

    pub fn list_name(&self, id: ListId) -> String {
        match id {
            ListId::RemoveBefore(i) => format!("remove_before.{}", self.stages[i]),
            ListId::Keep => "keep".to_string(),
        }
    }

    pub fn stage_index(&self, stage: &str) -> Result<usize> {
        self.stages
            .iter()
            .position(|s| s == stage)
            .with_context(|| format!("unknown stage {stage:?} (the config's stages are {:?})", self.stages))
    }
}

fn build_list(id: ListId, raw: RawList, name: &str) -> Result<List> {
    if raw.include.is_empty() && !raw.exclude.is_empty() {
        bail!("`{name}` has `exclude` patterns but no `include` patterns");
    }
    let entries = |raw: Vec<RawEntry>, part: &str| -> Result<Vec<Entry>> {
        raw.into_iter()
            .map(|e| {
                let (path, untracked) = match e {
                    RawEntry::Path(p) => (p, false),
                    RawEntry::Detailed(d) => (d.path, d.untracked),
                };
                Ok(Entry { pattern: Pattern::new(&path).with_context(|| format!("in `{name}.{part}`"))?, untracked })
            })
            .collect()
    };
    Ok(List { id, include: entries(raw.include, "include")?, exclude: entries(raw.exclude, "exclude")? })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
version: 1
stages: [test, production]
remove_before:
  test:
    include: ['.*', docs/, {path: node_modules/, untracked: true}]
    exclude: [.shiplist.yml]
  production:
    include: [.shiplist.yml, spec/]
keep:
  include: [app/]
forbid: ['**/*.map']
build_output: [public/assets/]
"#;

    #[test]
    fn parses_stages_in_order() {
        let c = Config::parse(GOOD).unwrap();
        assert_eq!(c.stages, ["test", "production"]);
        assert_eq!(c.lists.len(), 3);
        assert_eq!(c.list_name(c.lists[1].id), "remove_before.production");
        assert!(c.lists[0].include[2].untracked);
        assert_eq!(c.forbid.len(), 1);
    }

    #[test]
    fn rejects_mistakes() {
        let cases = [
            (GOOD.replace("version: 1", "version: 2"), "unsupported version"),
            (GOOD.replace("stages: [test, production]", "stages: [test]"), "isn't in `stages`"),
            (
                GOOD.replace("stages: [test, production]", "stages: [test, production, enterprise]"),
                "no `remove_before.enterprise`",
            ),
            (GOOD.replace("  production:", "  prodution:"), "isn't in `stages`"),
            (GOOD.replace("forbid:", "forbidden:"), "unknown field"),
            (GOOD.replace("include: [app/]", "includes: [app/]"), "unknown field"),
            (GOOD.replace("docs/", "/docs/"), "starts with '/'"),
        ];
        for (text, expected) in cases {
            let err = format!("{:#}", Config::parse(&text).unwrap_err());
            assert!(err.contains(expected), "expected {expected:?} in: {err}");
        }
    }
}
