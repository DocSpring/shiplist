# shiplist

shiplist puts every file in your repository in exactly one list, and removes each list before its stage of the build.
CI fails when a file isn't in a list, so every new file needs a decision about whether it ships.

```text
$ shiplist check
Files in no list (add each to exactly one list):
  config/partner_rates.yml
```

It's a small, static Rust binary for CI pipelines and Docker builds. It's most useful when you ship an image to people
outside your company, such as an on-premise install, where anything in the image is theirs to read. It also helps with
hosted apps: every file in a production image is one more file that someone who gets into a container can read.

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
- **Build output gets checked too.** git never sees what the build creates, such as source maps, so `shiplist verify`
  checks the real build directory, or an image saved with `docker save`, against the config.

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
