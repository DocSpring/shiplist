use anyhow::{Result, bail};
use globset::{GlobBuilder, GlobMatcher};

/// A path pattern, always anchored at the root of the tree.
///
/// - `dir/` matches the directory and everything under it.
/// - Anything else is a glob over the whole path: `*` and `?` never cross a `/`, `**` as a whole path component matches
///   any number of directories, and a leading `.` is an ordinary character (`*` matches dotfiles).
#[derive(Debug, Clone)]
pub struct Pattern {
    pub raw: String,
    kind: Kind,
}

#[derive(Debug, Clone)]
enum Kind {
    Dir(String),
    Glob(GlobMatcher),
}

impl Pattern {
    pub fn new(raw: &str) -> Result<Self> {
        if raw.is_empty() {
            bail!("empty pattern");
        }
        if raw.starts_with('/') {
            bail!("pattern {raw:?} starts with '/': patterns are always relative to the root, so leave it out");
        }
        if raw.starts_with("./") {
            bail!("pattern {raw:?} starts with './': patterns are always relative to the root, so leave it out");
        }
        if raw.starts_with('!') {
            bail!("pattern {raw:?} starts with '!': put exceptions in the list's `exclude` instead");
        }
        for component in raw.trim_end_matches('/').split('/') {
            if component.contains("**") && component != "**" {
                bail!(
                    "pattern {raw:?}: `**` must be a whole path component (like `a/**/b` or `.*/**`), \
                     otherwise it would only match within one directory"
                );
            }
        }
        if let Some(dir) = raw.strip_suffix('/') {
            if dir.contains(['*', '?', '[', '{']) {
                bail!("pattern {raw:?}: a directory pattern (ending in '/') can't contain wildcards; use `{dir}/**`");
            }
            return Ok(Pattern { raw: raw.to_string(), kind: Kind::Dir(dir.to_string()) });
        }
        let glob = GlobBuilder::new(raw)
            .literal_separator(true)
            .backslash_escape(true)
            .build()
            .map_err(|e| anyhow::anyhow!("pattern {raw:?}: {e}"))?;
        Ok(Pattern { raw: raw.to_string(), kind: Kind::Glob(glob.compile_matcher()) })
    }

    pub fn is_match(&self, path: &str) -> bool {
        match &self.kind {
            Kind::Dir(dir) => path == dir || (path.starts_with(dir.as_str()) && path[dir.len()..].starts_with('/')),
            Kind::Glob(glob) => glob.is_match(path),
        }
    }

    /// The directory a `dir/` pattern names, for removing untracked directories.
    pub fn dir(&self) -> Option<&str> {
        match &self.kind {
            Kind::Dir(dir) => Some(dir),
            Kind::Glob(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(p: &str, f: &str) -> bool {
        Pattern::new(p).unwrap().is_match(f)
    }

    #[test]
    fn directories() {
        assert!(m("docs/", "docs/a.md"));
        assert!(m("docs/", "docs/a/b.md"));
        assert!(m("docs/", "docs"));
        assert!(!m("docs/", "docs2/a.md"));
        assert!(!m("docs/", "app/docs/a.md"));
        assert!(m("app/assets/", "app/assets/x.svg"));
    }

    #[test]
    fn globs_are_anchored_and_stay_in_one_directory() {
        assert!(m("*.md", "README.md"));
        assert!(!m("*.md", "docs/a.md"));
        assert!(m(".*", ".env"));
        assert!(!m(".*", ".circleci/config.yml"));
        assert!(m("*", ".env"));
        assert!(m("convox*", "convox.yml"));
        assert!(!m("bin/seed", "x/bin/seed"));
    }

    #[test]
    fn double_star_crosses_directories() {
        assert!(m(".*/**", ".circleci/config.yml"));
        assert!(m(".*/**", ".circleci/a/b.yml"));
        assert!(!m(".*/**", ".env"));
        assert!(m("**/AGENTS.md", "AGENTS.md"));
        assert!(m("**/AGENTS.md", "a/b/AGENTS.md"));
        assert!(m("app/**/*.md", "app/x.md"));
        assert!(m("app/**/*.md", "app/a/b/x.md"));
        assert!(m("public/**/*.map.*", "public/w/js/a.js.map.gz"));
    }

    #[test]
    fn rejects_patterns_that_would_surprise() {
        for p in ["", "/app/", "./app/", "!app/", ".**/*", "a**b", "app/*/", "a/**x/b"] {
            assert!(Pattern::new(p).is_err(), "{p:?} should be rejected");
        }
    }
}
