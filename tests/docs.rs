//! Keeps the README and the skill in step with the code.

use shiplist::config::Config;

fn read(path: &str) -> String {
    std::fs::read_to_string(format!("{}/{path}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// The YAML code blocks that hold a whole config (`version: 1`), with any indentation removed.
fn configs(markdown: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut block: Option<(usize, Vec<String>)> = None;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        match &mut block {
            None if trimmed == "```yaml" => block = Some((line.len() - trimmed.len(), Vec::new())),
            Some((_, lines)) if trimmed == "```" => {
                let text = lines.join("\n");
                if text.starts_with("version: 1") {
                    out.push(text);
                }
                block = None;
            }
            Some((indent, lines)) => lines.push(line.get(*indent..).unwrap_or("").to_string()),
            None => {}
        }
    }
    out
}

#[test]
fn example_configs_parse() {
    for file in ["README.md", "skills/shiplist/SKILL.md"] {
        let found = configs(&read(file));
        assert!(!found.is_empty(), "no example config found in {file}");
        for text in found {
            Config::parse(&text).unwrap_or_else(|e| panic!("{file}: {e:#}\n{text}"));
        }
    }
}

#[test]
fn skill_version_matches_the_crate() {
    let skill = read("skills/shiplist/SKILL.md");
    let expected = format!("  version: \"{}\"\n", env!("CARGO_PKG_VERSION"));
    assert!(
        skill.contains(&expected),
        "skills/shiplist/SKILL.md must have metadata.version {}",
        env!("CARGO_PKG_VERSION")
    );
}
