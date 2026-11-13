# shiplist

shiplist puts every file in your repository in exactly one list, and removes each list before its stage of the build.
CI fails when a file isn't in a list, so every new file needs a decision about whether it ships. Then it checks what
actually shipped: a Docker image, an npm package, a gem, a wheel, a crate or any other tar or zip file.

```text
$ shiplist check
Files in no list (add each to exactly one list):
  config/partner_rates.yml
```

It's a small, static Rust binary for CI pipelines, Docker builds and release workflows. It's most useful when what you
build leaves your hands: an image for an on-premise install, or a package anyone can download from a registry.

## Why

A `.dockerignore` or an `rm -rf` in a Dockerfile is a list of files to leave out, so anything new ships until someone
adds it to the list: a setup script with a test key in it, a folder of internal notes, a new tool's config. And the
tests run on the full repository, so they can pass while the image is missing a file the code reads.

shiplist turns that around:

- **Every tracked file is in exactly one list.** `shiplist check` takes the file list from `git ls-files` and fails when
  a file is in no list or in two.
- **Each list is removed before a named stage.** You define the stages, such as `test` and `production`. CI prunes the
  `test` list before running the tests, so code that reads a file production won't have fails in CI instead of in
  production.
- **Patterns that match nothing fail.** The config can't fill up with paths for files that were deleted years ago.
- **What you ship gets checked too.** git never sees what the build creates, such as source maps, so `shiplist verify`
  checks the real build directory, an image saved with `docker save`, or a package, against the config.

The lists work like double-entry bookkeeping: a file that one list leaves out (with an `exclude`) has to be claimed by
another list, and a file claimed twice is an error. So adding or changing a pattern can never quietly move a file into
or out of your image: any overlap fails the check, and someone decides.

## Install

Download a binary for Linux (x86_64 or arm64, static) or macOS from the
[releases](https://github.com/DocSpring/shiplist/releases), or build it with Cargo:

```bash
cargo install --git https://github.com/DocSpring/shiplist --locked
```

## Config

`.shiplist.yml` at the root of the repository (or `--config <path>`):

```yaml
version: 1
stages: [test, production]

remove_before:
  test:
    include:
      - '.*'                       # dotfiles at the root
      - docs/
      - assets/                    # compiled into public/ by the build
      - path: node_modules/
        untracked: true            # git never has it: no stale-pattern error, and prune deletes it
    exclude:
      - .shiplist.yml              # the tests don't need it, but production removes it (see below)

  production:
    include:
      - .shiplist.yml
      - test/
      - bin/
    exclude:
      - bin/server                 # the app runs it: claimed by keep

keep:
  include:
    - app/
    - bin/server
    - Procfile

# Files that must never be in a build, tracked or not. `verify` fails on any of them.
forbid:
  - '**/*.map'
  - '**/.env*'

# Untracked files the build adds, which `verify` and `prune --no-git` allow
build_output:
  - public/
```

- `stages` names the stages in order. A file removed before a stage is gone from every later stage too. Every stage
  needs a `remove_before` list.
- A file is in a list when one of the list's `include` patterns matches it and none of its `exclude` patterns do.
- `keep` holds the files every stage gets.

### Patterns

Patterns are always relative to the repository root.

| Pattern | Matches |
| --- | --- |
| `docs/` | the `docs` directory and everything under it |
| `*.md` | Markdown files at the root only: `*` and `?` never cross a `/` |
| `.*` | dotfiles at the root, such as `.env` |
| `.*/**` | everything inside dot directories, such as `.github/workflows/ci.yml` |
| `**/AGENTS.md` | `AGENTS.md` at any depth, including the root |
| `app/**/*.md` | Markdown files anywhere under `app/` |
| `{path: tmp/, untracked: true}` | a path git never tracks |

`**` must be a whole path component. shiplist rejects `.**/*`, which in many glob libraries silently matches only one
directory level. It also rejects a leading `/`, `./` or `!`: put exceptions in the list's `exclude`.

## Commands

| Command | What it does |
| --- | --- |
| `shiplist check` | Fails unless every tracked file is in exactly one list, every pattern matches a tracked file, and every `exclude` takes a file out of its own list. |
| `shiplist list <stage>` | Prints the tracked files still there at a stage. `--removed` prints the ones removed at or before it. |
| `shiplist explain <path>...` | Shows which patterns match a path and what happens to it at each stage. |
| `shiplist prune <stage>` | Deletes the files removed at or before a stage. A directory whose tracked files are all removed goes with everything in it. |
| `shiplist verify <stage> --dir <dir>` | Fails on any file in the directory that the stage shouldn't have, that matches `forbid`, or that's in no list and not `build_output`. |
| `shiplist verify <stage> --image <tar> --root /app` | The same for an image saved with `docker save`, read without Docker. `--root` is where the repository's files are inside the image. |
| `shiplist verify <stage> --archive <file> --root <dir>` | The same for a package or any tar, `.tar.gz` or zip file: npm and Python source packages, `.crate`, Helm charts, wheels, `.nupkg`, `.jar`, `.gem` and Hex packages. `--root` strips the folder the package puts everything in, such as `package/` for npm. |

`check`, `list` and `prune` work at the top of a git working tree, or in a directory inside one that has its own
`.shiplist.yml`, such as a package in a monorepo. They never apply a repository's config to one of its subdirectories.

`check` and `verify` exit with 1 when they find a problem, and every command exits with 2 on an error (an invalid
config, an unknown stage).

`prune` deletes files, so it runs only when `CI=true` (or `CI=1`), or with `--yes` on a working tree with no
uncommitted changes. `--dry-run` prints what it would delete. `--no-git` walks the directory instead of asking git, for a
tree without `.git`, such as a build stage of a Dockerfile. Without git, every file must be in a list or match
`build_output`, or nothing is deleted.

```text
$ shiplist explain bin/server
bin/server
  remove_before.production: include bin/, but exclude bin/server
  keep: include bin/server
  => keep: kept at every stage
```

## In CI

Check the config on every push, prune before the tests, and prune a clean copy before building the image:

```yaml
# GitHub Actions
- run: shiplist check
- run: shiplist prune test           # GitHub Actions sets CI=true
- run: bundle exec rspec
```

```bash
# Build the image from a clean copy of the repository
git clone . build/src
shiplist prune production --dir build/src
rm -rf build/src/.git
docker build -f Dockerfile build/
docker save my-app:latest -o my-app.tar
shiplist verify production --image my-app.tar --root /app
```

In a Dockerfile, run it in a build stage:

```dockerfile
FROM my-app:latest AS pruned
COPY shiplist /usr/local/bin/shiplist
COPY .shiplist.yml /tmp/.shiplist.yml
RUN CI=true shiplist --config /tmp/.shiplist.yml prune on-premise --dir /app --no-git
```

## Packages

The same check works for anything you publish. Your packaging tool's own settings (`files` in package.json, the
gemspec's `files`, `MANIFEST.in`, `include` in Cargo.toml) still decide what goes in, and shiplist checks the package it
built before you publish it. Give the config a `package` stage:

```yaml
version: 1
stages: [package]

remove_before:
  package:
    include: ['.*', test/, docs/]

keep:
  include: [lib/, package.json, README.md, LICENSE]

forbid:
  - '**/.env*'
  - '**/*.map'
```

Then build the package and verify it in your release workflow:

```bash
npm pack
shiplist verify package --archive my-package-1.2.0.tgz --root package/
```

```text
Forbidden files:
  .env.development  (forbid: **/.env*)

Files that shouldn't be here at stage package:
  test/index.test.js  (remove_before.package)
```

| Package | Build | Verify |
| --- | --- | --- |
| npm | `npm pack` | `--archive name-1.2.0.tgz --root package/` |
| Ruby gem | `gem build name.gemspec` | `--archive name-1.2.0.gem` (shiplist reads the files inside its `data.tar.gz`) |
| Python source | `python -m build`, `uv build` | `--archive dist/name-1.2.0.tar.gz --root name-1.2.0/`, with `PKG-INFO` in `build_output` |
| Python wheel (flat layout) | `python -m build`, `uv build` | `--archive dist/name-1.2.0-py3-none-any.whl`, with `'name-*.dist-info/**'` in `build_output` |
| Rust crate | `cargo package` | `--archive target/package/name-1.2.0.crate --root name-1.2.0/`, with `Cargo.toml.orig` and `.cargo_vcs_info.json` in `build_output` |
| Helm chart | `helm package .` | `--archive name-1.2.0.tgz --root name/`, with the config in the chart's directory |
| Anything zipped | `zip -r bundle.zip ...` | `--archive bundle.zip` |

The paths in the package have to be the repository's paths. A wheel built from a `src/` layout stores `src/name/` as
`name/`, and a `.jar` or `.nupkg` holds compiled code, so verify the source package for those, or check them with
`forbid` and `build_output` alone.

`build_output` only covers files that no list claims, so keep a broad pattern such as `'.*'` from claiming a file the
packaging tool adds (`.cargo_vcs_info.json`): list your dotfiles by name instead. shiplist checks its own crate this way
in [its CI](.github/workflows/ci.yml).

### Let shiplist decide what goes in

Two ways to make shiplist's lists decide the package's contents, rather than only check them:

- **Give the packaging tool the list.** `shiplist list package` prints the files the package should have, one per line,
  for anything that takes a list of files. In a gemspec:

  ```ruby
  spec.files = IO.popen(%w[shiplist list package], &:read).split("\n")
  ```

  Bundler evaluates the gemspec too, so everyone who runs `bundle install` in the repository needs shiplist installed.
  `tar -T`, `zip -@` and `rsync --files-from` read the same list from stdin or a file.
- **Prune a clean copy, then build there.** This is what the Docker example above does, and it works for any tool that
  packages what's in the directory (npm, Python, Helm, zip):

  ```bash
  git clone . build/pkg
  shiplist prune package --dir build/pkg
  rm -rf build/pkg/.git
  (cd build/pkg && npm pack)
  ```

  Remove `.git` before you build: tools that ask git for the file list (a gemspec that runs `git ls-files`,
  `cargo package`, setuptools-scm) would still see the files `prune` deleted.

Then verify the package anyway: it catches anything the packaging tool adds on its own.

### More places to use it

- **Public mirrors of private code.** If you publish part of a private repository to a public one (docs, examples, an
  SDK), verify the mirror before the push, so an internal file can't leak out.
- **Helm charts.** `.helmignore` is a denylist, and charts are `.tgz` files.
- **Lambda and serverless zips, and browser extensions.** Source maps, `.env` files and test fixtures end up in these
  all the time.
- **Static sites and docs.** Verify the build folder before it goes to S3 or Pages, so drafts and source maps stay out.
- **Source tarballs** made with `git archive`, where `export-ignore` in `.gitattributes` is a denylist too.
- **Composer packages and Go modules,** which are built from the repository itself: run `shiplist check` on the
  repository, in the package's directory if it's in a monorepo.

## For coding agents

- **The skill.** `npx skills add DocSpring/shiplist` installs the shiplist [Agent Skill](https://agentskills.io) for
  Claude Code, Codex, Cursor and other agents. It covers what agents get wrong when `shiplist check` fails in CI:
  deciding which list a new file belongs in, fixing a file in two lists without deleting the pattern around it, and
  fixing the code (not the config) when a test fails only after `prune`. It also walks through setting shiplist up.
- **JSON output.** `--format json` on any command prints one JSON document on stdout, including for problems (exit 1)
  and errors (exit 2).
- **Working on shiplist itself:** read [AGENTS.md](AGENTS.md) for the design rules and how to test and release.

## Prior art

- [check-manifest](https://github.com/mgedmin/check-manifest) checks that every file in a Python package's git
  repository is either in the source distribution or ignored. shiplist works on any repository and adds stages,
  stale-pattern checks and image checks.
- CODEOWNERS validators such as [codeowners-validator](https://github.com/mszostok/codeowners-validator) catch
  patterns that match nothing, but a file can match several rules, and they don't remove files.
- [dive](https://github.com/wagoodman/dive), [container-structure-test](https://github.com/GoogleContainerTools/container-structure-test)
  and secret scanners inspect a built image against rules you write for the files you know about. shiplist starts from
  every file in the repository.

## Background

We built the first version at [DocSpring](https://docspring.com) in 2025, as a Ruby script in our Rails app's CI. Our
app ships as a Docker image that customers run on their own servers, and a list of files to remove kept missing new
ones. shiplist is a rewrite of that script, with the gaps we found since: build output that git never sees, and image
checks.

## License

MIT or Apache-2.0, at your option.
