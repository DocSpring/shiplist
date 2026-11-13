# shiplist

A Rust CLI that puts every file in a repository in exactly one list and removes each list before its build stage. It
runs in other people's CI and Docker builds, and `prune` deletes files, so correctness comes before convenience.

## Layout

- `src/config.rs`: `.shiplist.yml` parsing and validation. `src/pattern.rs`: pattern syntax and matching.
- `src/classify.rs`: which lists claim a path, and `explain`. `src/check.rs`, `src/prune.rs`, `src/verify.rs`: the
  commands. `src/image.rs`: reads a `docker save` tarball. `src/tree.rs`: git's file list and directory walks.
  `src/main.rs`: the CLI and its text and JSON output.
- `tests/cli.rs`: runs the binary against real git repositories in temp directories.
- `skills/shiplist/SKILL.md`: the Agent Skill for people using shiplist (`npx skills add DocSpring/shiplist`).

## Design rules

These are why the tool exists. Don't change them to make a feature simpler; raise it with a maintainer instead.

- **Exclusive lists (double-entry bookkeeping).** A file claimed by two lists is an error, and an `exclude` only takes a
  file out of its own list, so another list must claim it. Never add ordered or first-match-wins rules, priorities or
  "most specific pattern wins": each makes it possible for a pattern change to move a file into or out of an image
  without any check failing.
- **No silent matches.** Patterns are anchored at the root, `*` never crosses `/`, and `**` must be a whole path
  component. Reject a pattern that could surprise someone (`.**/*`, a leading `/` or `!`) with an error that says what
  to write instead, rather than guessing.
- **Every pattern must earn its place.** `check` fails on a pattern that matches no tracked file (unless
  `untracked: true`) and on an `exclude` that takes nothing out of its own list.
- **`prune` is careful.** It runs only with `CI=true`/`CI=1` or `--yes`, refuses a working tree with uncommitted
  changes, and deletes nothing unless every file has exactly one list. Keep all three.
- **Output is for people and agents.** Text output says how to fix each problem. `--format json` prints one JSON
  document on stdout for every outcome, including errors. Adding a JSON field is fine; renaming or removing one is a
  breaking change (a major version once we're at 1.0, a minor one before).
- Exit codes: 0 ok, 1 `check` or `verify` found problems, 2 an error.

## Changes

- Every behaviour change gets a test: a unit test next to the code for matching and classification, and a case in
  `tests/cli.rs` for anything a user sees (output, exit codes, files deleted).
- Update `README.md` and `skills/shiplist/SKILL.md` in the same commit when commands, flags, output or the config
  format change. The skill says how to decide things; the README is the reference. Don't copy one into the other.
- Before committing: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. CI runs them on Linux
  and macOS, plus `cargo deny check` for licences.
- Dependencies must be permissively licensed (MIT, Apache-2.0, BSD, ISC, Zlib, Unlicense, Unicode): people ship this
  binary in their images. `deny.toml` enforces it. Add as few as possible; shell out to `git` rather than linking
  libgit2.

## Releasing

1. Bump `version` in `Cargo.toml` and `metadata.version` in `skills/shiplist/SKILL.md` (a test checks they match), then
   run `cargo build` to update `Cargo.lock`.
2. Commit, then tag `vX.Y.Z` on that commit and push the tag. `.github/workflows/release.yml` builds the Linux (musl,
   x86_64 and arm64) and macOS binaries, attests them and creates the GitHub release with checksums.
3. `cargo publish` from the tagged commit.

## Public repository

Everything here is public. Examples use made-up files (`config/partner_rates.yml`, `sk_test_not_a_real_key`), never a
real company's paths, secrets or customers.
