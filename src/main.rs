use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::json;

use shiplist::config::{self, Config};
use shiplist::{check, classify, image, prune, tree, verify};

/// Put every file in a repository in exactly one list, and remove each list before its build stage.
#[derive(Parser)]
#[command(version, about, max_term_width = 100)]
struct Cli {
    /// The config file
    #[arg(long, global = true, default_value = config::DEFAULT_PATH)]
    config: PathBuf,

    /// Output format. `json` prints one JSON document on stdout, also for problems and errors
    #[arg(long, global = true, value_enum, default_value_t = Format::Text)]
    format: Format,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum Format {
    Text,
    Json,
}

#[derive(Subcommand)]
enum Command {
    /// Fail unless every tracked file is in exactly one list and every pattern still matches something
    Check {
        /// The top of the git working tree
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
    /// Print the tracked files that are still there at a stage
    List {
        /// A stage from the config's `stages`
        stage: String,
        /// Print the files removed at or before the stage instead
        #[arg(long)]
        removed: bool,
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
    /// Show which patterns match a path and what happens to it at each stage
    Explain {
        #[arg(required = true)]
        paths: Vec<String>,
    },
    /// Delete the files a stage doesn't get. Runs only in CI (CI=true) or with --yes
    Prune {
        /// A stage from the config's `stages`
        stage: String,
        /// The directory to prune
        #[arg(long, default_value = ".")]
        dir: PathBuf,
        /// Walk the directory instead of asking git (for a tree without .git, such as an image's /app)
        #[arg(long)]
        no_git: bool,
        /// Print what would be removed, and remove nothing
        #[arg(long)]
        dry_run: bool,
        /// Run outside CI. Refuses a working tree with uncommitted changes
        #[arg(long)]
        yes: bool,
    },
    /// Fail if a directory or a saved image has files that a stage shouldn't have
    Verify {
        /// A stage from the config's `stages`
        stage: String,
        /// A directory to check
        #[arg(long, conflicts_with = "image", required_unless_present = "image")]
        dir: Option<PathBuf>,
        /// A tarball from `docker save`
        #[arg(long)]
        image: Option<PathBuf>,
        /// Where the repository's files are inside the image
        #[arg(long, requires = "image", default_value = "/")]
        root: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let format = cli.format;
    match run(cli) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            if format == Format::Json {
                print_json(&json!({ "ok": false, "error": format!("{e:#}") }));
            } else {
                eprintln!("shiplist: {e:#}");
            }
            ExitCode::from(2)
        }
    }
}

fn print_json(value: &impl Serialize) {
    println!("{}", serde_json::to_string_pretty(value).expect("the reports always serialize"));
}

/// A report with `ok` in front of its fields.
#[derive(Serialize)]
struct WithOk<'a, T: Serialize> {
    ok: bool,
    #[serde(flatten)]
    report: &'a T,
}

fn in_ci() -> bool {
    matches!(std::env::var("CI").as_deref(), Ok("true" | "1"))
}

fn run(cli: Cli) -> Result<bool> {
    let config = Config::load(&cli.config)?;
    match cli.command {
        Command::Check { dir } => {
            let files = tree::tracked_files(&dir)?;
            let report = check::check(&config, &files);
            if cli.format == Format::Json {
                print_json(&WithOk { ok: report.is_ok(), report: &report });
                return Ok(report.is_ok());
            }
            if report.is_ok() {
                println!("{} files, each in exactly one list", files.len());
                return Ok(true);
            }
            eprint!("{}", report.render());
            Ok(false)
        }
        Command::List { stage, removed, dir } => {
            let stage = config.stage_index(&stage)?;
            let files = tree::tracked_files(&dir)?;
            let mut listed = Vec::new();
            for file in &files {
                let lists = classify::classify(&config, file);
                let [id] = lists.as_slice() else {
                    bail!("{file} isn't in exactly one list: run `shiplist check`");
                };
                if classify::present_at(*id, stage) != removed {
                    listed.push(file);
                }
            }
            if cli.format == Format::Json {
                print_json(&json!({ "stage": config.stages[stage], "removed": removed, "files": listed }));
            } else {
                for file in listed {
                    println!("{file}");
                }
            }
            Ok(true)
        }
        Command::Explain { paths } => {
            let explanations: Vec<_> = paths.iter().map(|p| classify::explain(&config, p)).collect();
            if cli.format == Format::Json {
                print_json(&explanations);
            } else {
                for e in &explanations {
                    print_explanation(e);
                }
            }
            Ok(true)
        }
        Command::Prune { stage, dir, no_git, dry_run, yes } => {
            let stage_index = config.stage_index(&stage)?;
            if !dry_run && !in_ci() {
                if !yes {
                    bail!("prune deletes files, so it runs only in CI (CI=true) or with --yes");
                }
                if !no_git && tree::is_dirty(&dir)? {
                    bail!("{} has uncommitted changes or untracked files: commit them first", dir.display());
                }
            }
            // Walk the directory only when something needs it: it can hold far more files than git tracks
            let needs_walk = no_git || prune::has_untracked_globs(&config, stage_index);
            let walked = if needs_walk { tree::walk_files(&dir)? } else { Vec::new() };
            let files = if no_git { walked.clone() } else { tree::tracked_files(&dir)? };
            let plan = prune::plan(&config, &files, stage_index, no_git)?;
            let untracked = prune::untracked_paths(&config, &dir, stage_index, &walked);
            apply(&dir, &plan, &untracked, dry_run, &stage, cli.format)
        }
        Command::Verify { stage, dir, image, root } => {
            let stage_index = config.stage_index(&stage)?;
            let (files, what) = match (dir, image) {
                (Some(dir), _) => (tree::walk_files(&dir)?, dir.display().to_string()),
                (None, Some(tarball)) => {
                    let files = image::image_files(&tarball)?;
                    (image::under_root(&files, &root), format!("{}:{root}", tarball.display()))
                }
                (None, None) => unreachable!("clap requires --dir or --image"),
            };
            let report = verify::verify(&config, &files, stage_index);
            if cli.format == Format::Json {
                print_json(&json!({
                    "ok": report.is_ok(),
                    "stage": stage,
                    "target": what,
                    "files": report.files,
                    "forbidden": report.forbidden,
                    "removed_but_present": report.removed_but_present,
                    "unexpected": report.unexpected,
                    "conflicts": report.conflicts,
                }));
                return Ok(report.is_ok());
            }
            if report.is_ok() {
                println!("{what}: {} files, all allowed at stage {stage}", files.len());
                return Ok(true);
            }
            eprint!("{}", report.render(&stage));
            Ok(false)
        }
    }
}

fn print_explanation(e: &classify::Explanation) {
    println!("{}", e.path);
    for m in &e.matches {
        if m.exclude.is_empty() {
            println!("  {}: include {}", m.list, m.include.join(", "));
        } else {
            println!("  {}: include {}, but exclude {}", m.list, m.include.join(", "), m.exclude.join(", "));
        }
    }
    match e.lists.as_slice() {
        [] => println!("  => in no list"),
        [list] => match e.stages.iter().find(|s| !s.present) {
            None => println!("  => {list}: kept at every stage"),
            Some(s) => println!("  => {list}: removed before {}", s.stage),
        },
        lists => println!("  => in {} lists: {}", lists.len(), lists.join(", ")),
    }
}

fn apply(
    dir: &Path,
    plan: &prune::Plan,
    untracked: &[String],
    dry_run: bool,
    stage: &str,
    format: Format,
) -> Result<bool> {
    let verb = if dry_run { "would remove" } else { "removed" };
    let (mut dirs, mut files) = (Vec::new(), Vec::new());
    for d in &plan.dirs {
        if dry_run || prune::remove(dir, d)? {
            dirs.push(d.clone());
        }
    }
    for f in plan.files.iter().chain(untracked) {
        let is_dir = dir.join(f).is_dir();
        if dry_run || prune::remove(dir, f)? {
            if is_dir { dirs.push(f.clone()) } else { files.push(f.clone()) }
        }
    }
    if format == Format::Json {
        print_json(&json!({ "stage": stage, "dry_run": dry_run, "directories": dirs, "files": files }));
        return Ok(true);
    }
    for d in &dirs {
        println!("{verb} {d}/");
    }
    for f in &files {
        println!("{verb} {f}");
    }
    println!("{verb} {} directories and {} files for stage {stage}", dirs.len(), files.len());
    Ok(true)
}
