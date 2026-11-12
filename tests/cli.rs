use std::path::Path;
use std::process::{Command, Output};

const CONFIG: &str = r#"version: 1
stages: [test, production]

remove_before:
  test:
    include:
      - '.*'
      - docs/
      - assets/
      - {path: node_modules/, untracked: true}
    exclude:
      - .shiplist.yml
  production:
    include:
      - .shiplist.yml
      - test/
      - bin/
    exclude:
      - bin/server

keep:
  include:
    - app/
    - bin/server
    - Procfile

forbid:
  - '**/*.map'

build_output:
  - public/
"#;

fn write(dir: &Path, path: &str, text: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, text).unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false"])
        .args(args)
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok, "git {args:?} failed");
}

fn repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    git(d, &["init", "-q", "-b", "main"]);
    write(d, ".shiplist.yml", CONFIG);
    write(d, ".env.development", "API_KEY=not_a_real_key\n");
    write(d, "docs/setup.md", "# Setup\n");
    write(d, "assets/logo.svg", "<svg/>\n");
    write(d, "test/app_test.rb", "\n");
    write(d, "bin/seed", "\n");
    write(d, "bin/server", "\n");
    write(d, "app/server.rb", "\n");
    write(d, "app/lib/util.rb", "\n");
    write(d, "Procfile", "web: bin/server\n");
    git(d, &["add", "-A"]);
    git(d, &["commit", "-qm", "init"]);
    tmp
}

fn shiplist(dir: &Path, args: &[&str], ci: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_shiplist"));
    cmd.current_dir(dir).args(args).env_remove("CI");
    if ci {
        cmd.env("CI", "true");
    }
    cmd.output().unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn check_passes_then_fails_on_a_new_file() {
    let r = repo();
    let o = shiplist(r.path(), &["check"], false);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(stdout(&o), "10 files, each in exactly one list\n");

    write(r.path(), "config/rates.yml", "rate: 1\n");
    git(r.path(), &["add", "-A"]);
    let o = shiplist(r.path(), &["check"], false);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(stderr(&o), "Files in no list (add each to exactly one list):\n  config/rates.yml\n\n");
}

#[test]
fn check_fails_on_a_stale_pattern() {
    let r = repo();
    // bin/ still matches bin/server, so deleting bin/seed leaves no stale pattern
    git(r.path(), &["rm", "-q", "bin/seed"]);
    let o = shiplist(r.path(), &["check"], false);
    assert!(o.status.success(), "{}", stderr(&o));

    git(r.path(), &["rm", "-q", "-r", "docs"]);
    let o = shiplist(r.path(), &["check"], false);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(
        stderr(&o),
        "Patterns that match no tracked file (delete them, or mark one `untracked: true` if git never has it):\n  \
         remove_before.test.include: docs/\n\n"
    );
}

#[test]
fn list_and_explain() {
    let r = repo();
    let o = shiplist(r.path(), &["list", "production"], false);
    assert_eq!(stdout(&o), "Procfile\napp/lib/util.rb\napp/server.rb\nbin/server\n");
    let o = shiplist(r.path(), &["list", "test", "--removed"], false);
    assert_eq!(stdout(&o), ".env.development\nassets/logo.svg\ndocs/setup.md\n");
    let o = shiplist(r.path(), &["explain", "bin/server"], false);
    assert_eq!(
        stdout(&o),
        "bin/server\n  remove_before.production: include bin/, but exclude bin/server\n  keep: include bin/server\n  => keep: kept at every stage\n"
    );
    let o = shiplist(r.path(), &["list", "staging"], false);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("unknown stage \"staging\""));
}

#[test]
fn prune_refuses_outside_ci() {
    let r = repo();
    let o = shiplist(r.path(), &["prune", "test"], false);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("runs only in CI"));
    assert!(r.path().join("docs/setup.md").exists());

    write(r.path(), "app/server.rb", "changed\n");
    let o = shiplist(r.path(), &["prune", "test", "--yes"], false);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("uncommitted changes"));
    assert!(r.path().join("docs/setup.md").exists());
}

#[test]
fn prune_test_then_production() {
    let r = repo();
    write(r.path(), "node_modules/left-pad/index.js", "\n");
    write(r.path(), "docs/untracked-notes.md", "\n");

    let o = shiplist(r.path(), &["prune", "test", "--dry-run"], false);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(r.path().join("docs/setup.md").exists());

    let o = shiplist(r.path(), &["prune", "test"], true);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(
        stdout(&o),
        "removed assets/\nremoved docs/\nremoved .env.development\nremoved node_modules/\nremoved 3 directories and 1 files for stage test\n"
    );
    assert!(!r.path().join("docs").exists());
    assert!(r.path().join("test/app_test.rb").exists());

    let o = shiplist(r.path(), &["prune", "production"], true);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(
        stdout(&o),
        "removed test/\nremoved .shiplist.yml\nremoved bin/seed\nremoved 1 directories and 2 files for stage production\n"
    );
    let mut left = shiplist::tree::walk_files(r.path()).unwrap();
    left.sort();
    assert_eq!(left, ["Procfile", "app/lib/util.rb", "app/server.rb", "bin/server"]);
}

#[test]
fn verify_a_directory_and_prune_without_git() {
    let r = repo();
    let config = r.path().join(".shiplist.yml").to_str().unwrap().to_string();
    let ctx = tempfile::tempdir().unwrap();
    let c = ctx.path();
    write(c, "app/server.rb", "\n");
    write(c, "bin/server", "\n");
    write(c, "Procfile", "\n");
    write(c, "public/app.js", "\n");
    let o = shiplist(r.path(), &["--config", &config, "verify", "production", "--dir", c.to_str().unwrap()], false);
    assert!(o.status.success(), "{}", stderr(&o));

    write(c, "public/app.js.map", "{}\n");
    write(c, "test/app_test.rb", "\n");
    write(c, "notes.txt", "\n");
    let o = shiplist(r.path(), &["--config", &config, "verify", "production", "--dir", c.to_str().unwrap()], false);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(
        stderr(&o),
        "Forbidden files:\n  public/app.js.map  (forbid: **/*.map)\n\n\
         Files that shouldn't be here at stage production:\n  test/app_test.rb  (remove_before.production)\n\n\
         Files in no list and not covered by `build_output`:\n  notes.txt\n\n"
    );

    // Without git, a file in no list stops prune before it removes anything
    let o = shiplist(
        r.path(),
        &["--config", &config, "prune", "production", "--no-git", "--dir", c.to_str().unwrap()],
        true,
    );
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("notes.txt: in no list"), "{}", stderr(&o));
    assert!(c.join("test/app_test.rb").exists());

    std::fs::remove_file(c.join("notes.txt")).unwrap();
    let o = shiplist(
        r.path(),
        &["--config", &config, "prune", "production", "--no-git", "--dir", c.to_str().unwrap()],
        true,
    );
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(!c.join("test").exists());
    assert!(c.join("public/app.js").exists());
}
