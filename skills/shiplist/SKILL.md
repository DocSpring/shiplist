---
name: shiplist
description: Works with shiplist, the CLI that puts every file in a repository in exactly one list (removed before a build stage, or kept) so CI decides which files reach the tests, the Docker image and published packages. Fixes a failing `shiplist check` (a file in no list, in two lists, a stale pattern, a dead exception) or `shiplist verify` (on a build directory, an image or a package such as an npm tarball, gem, wheel or crate), finds why a test fails only in CI after `shiplist prune`, decides which list a new file belongs in, and sets shiplist up in a repository's CI, image build or release workflow. Use when a repository has a .shiplist.yml, when CI output mentions shiplist, or when someone wants to keep dev files, tests, secrets or source maps out of a Docker image or a package. Not for .dockerignore-only setups or vulnerability scanning.
license: MIT OR Apache-2.0
metadata:
  author: DocSpring
  version: "0.1.0"
---

# shiplist

shiplist reads `.shiplist.yml` at the repository root. Every file git tracks must be in exactly one list:
`remove_before.<stage>` (deleted before that stage of the build and every later one) or `keep` (shipped at every
stage). The stages are named in `stages`, in order, such as `[test, production]`. CI runs `shiplist prune test` before
the tests, so code that reads a file production won't have fails in CI, and `shiplist prune production` on a clean copy
before building the image.

The lists are double-entry bookkeeping. An `exclude` only takes a file out of its own list, so another list must claim
it, and a file two lists claim fails the check. Keep it that way: never "fix" a check by making the lists overlap less
carefully or by widening a pattern until the error goes away.

## Commands

Add `--format json` whenever you read the output: every command then prints one JSON document on stdout, including
problems (exit 1) and errors (exit 2, `{"ok": false, "error": "..."}`).

| Command | Use it to |
| --- | --- |
| `shiplist check` | See every file in no list or two, stale patterns and dead exceptions. Text output groups more than 20 unclassified files by top-level directory; JSON lists each one. |
| `shiplist explain <path>...` | See which patterns match a path, which list it's in and whether each stage has it. Works for paths that don't exist yet. |
| `shiplist list <stage>` | See the tracked files a stage gets (`--removed`: the ones it doesn't). |
| `shiplist verify <stage> --dir D`, `--image app.tar --root /app` or `--archive pkg.tgz --root package/` | Check a build directory, a `docker save` tarball or a package. |
| `shiplist prune <stage>` | Delete a stage's files. CI only: see "Never" below. |

Patterns are anchored at the repository root: `docs/` is a directory and everything in it, `*` and `?` never cross a
`/`, and `**` is a whole path component (`.*/**`, `**/AGENTS.md`, `app/**/*.md`).

## A file in no list

Decide by what reads the file, not by what makes CI pass. Search the code for its path or name first.

1. Production code reads it at runtime (the app loads it, serves it, runs it): `keep`.
2. Only the tests, CI or the build read it (specs, fixtures, CI scripts, build config): the `remove_before` list of the
   first stage after the tests, usually `remove_before.production`.
3. Nothing the tests run reads it (docs, notes, editor and linter config, dev setup scripts): the first stage's list,
   usually `remove_before.test`. Prefer this one whenever the tests pass without the file: then the tests prove
   production doesn't need it.
4. It could hold a secret, credentials, customer or personal data, internal notes or anything a person outside the
   company shouldn't read: it must not go in `keep`. If you can't tell, ask the person you're working with before
   choosing, and say why. If it plainly holds secrets or real personal data, it probably shouldn't be in git at all:
   put it in the first stage's list so it can't ship, and tell the person. Deleting it from the repository and its
   history, rotating a secret and adding it to `.gitignore` are their decisions.

Use the file's own path by default, so the next file added beside it gets its own decision. Use a directory pattern
only when the folder has one clear purpose and every file that will ever go in it belongs in the same list, such as a
folder of test fixtures. Never use one for folders of data, exports, tools or anything that could hold sensitive files.
Don't widen an existing pattern or add `*` to get a group of unrelated files through.

When a broad pattern in one list already covers the new file's directory but the file belongs elsewhere (a dev script
under a kept `app/`), add both entries: the file in the right list's `include`, and in the other list's `exclude`.

## A file in two lists

The output names the include pattern in each list. Decide which list is right (the same questions as above), then add
the file, or the narrowest pattern for it, to the other list's `exclude`. Don't delete the other list's broad include:
it still covers the files around this one.

## A stale pattern

A pattern matches no tracked file because a file was deleted or renamed.

- Already covered: if a broader pattern in the same list matches the files this one was for, delete it.
- Renamed or moved: point the pattern at the new path, and check the new location still belongs in that list.
- Deleted: remove the pattern, then run `shiplist check` again. The other entry of the pair (an `exclude` elsewhere)
  usually turns into a dead exception; remove that too.
- Only mark a pattern `untracked: true` when git never tracks that path (`node_modules/`, a build cache). Never to
  silence a tracked path.

## A dead exception

An `exclude` that takes no file out of its own list does nothing. Delete it. It's usually the leftover half of a pair
whose other half you just changed.

## A test that fails only in CI, or an image missing a file

After `shiplist prune test`, the test run doesn't have the files production won't have. A test that passes locally and
fails in CI with a missing file means the code reads a file the image leaves out, and production would fail the same
way. Fix the code so it reads what production has (a compiled asset, a file in a kept directory). Don't move the file
into `keep` or a later stage to make the test pass: `shiplist explain <path>` shows which list removes it and why.

To reproduce, prune a throwaway clone, never the working copy:

```bash
git clone . /tmp/shiplist-repro && cd /tmp/shiplist-repro
shiplist --config "$OLDPWD/.shiplist.yml" prune test --yes
# then run the failing test here
```

## A failing verify

- `forbidden`: something put a forbidden file (a source map, an `.env`) in the build. Find the step that writes it and
  stop it, or delete it before the image is built. Never shrink `forbid`.
- `removed_but_present`: the build copied the file from somewhere other than the pruned tree, such as the original
  checkout. Copy from the pruned copy instead.
- `unexpected`: a file in no list that the build created. If the build makes it on purpose, add it to `build_output`;
  otherwise find out where it came from.

## A package that fails verify

The packaging tool's own settings decide what goes in the package (`files` in package.json, `.npmignore`, the gemspec's
`files`, `MANIFEST.in` or the build backend's config, `include` in Cargo.toml, `.helmignore`). When `shiplist verify
--archive` reports a file that shouldn't be in the package, fix those settings so the tool leaves it out, then rebuild
and verify again. Don't move the file to `keep` because it's in the package: decide its list with the questions in "A
file in no list", as for any file.

- A file the packaging tool adds itself (a wheel's `dist-info/`, `PKG-INFO`, `Cargo.toml.orig`,
  `.cargo_vcs_info.json`) goes in `build_output`. `build_output` only covers files no list claims, so if a broad pattern
  such as `'.*'` claims it, name your dotfiles in that list instead.
- Every file reported as unexpected under one folder (`package/`, `name-1.2.0/`): `--root` is missing or wrong.
- A wheel from a `src/` layout or a compiled package (`.jar`, `.nupkg`) doesn't use the repository's paths: verify the
  source package instead.

When the repository's gemspec builds `spec.files` from `shiplist list package`, the lists already decide the gem's
contents; a verify failure then means the gemspec adds files of its own.

## Never

- Run `shiplist prune` in a developer's working copy, or set `CI=true` to get past its guard. It deletes files. Use a
  throwaway clone, or `--dry-run` to see what it would do.
- Put a file in `keep` "to be safe". Every kept file ships.
- Remove a `forbid` pattern or mark a tracked path `untracked: true` to make a check pass.

## Setting up shiplist in a repository

1. Start a config with the stages and the obvious entries, then let `shiplist check` list the rest:

   ```yaml
   version: 1
   stages: [test, production]

   remove_before:
     test:
       include: ['.*', '.*/**', '*.md', docs/]
       exclude: [.shiplist.yml]
     production:
       include: [.shiplist.yml, test/, spec/]

   keep:
     include: [src/]
   ```

   `check` reports the patterns your repository has no files for (`spec/`, `src/`): delete or change those first.

2. Run `shiplist check` and work through the grouped output one top-level directory at a time, using the questions in
   "A file in no list". Dotfiles the tests read (`.rspec`, `.ruby-version`, `.nvmrc`) need an `exclude` in the first
   stage and an entry in a later list. Ask the person about any directory you can't place.
3. Test the result in a throwaway clone: `shiplist prune test --yes`, then run the whole test suite there. Fix code that
   reads a removed file; move a file to a later stage only when the tests themselves need it.
4. Wire up CI: `shiplist check` on every push, `shiplist prune test` before the tests, `shiplist prune production --dir
   <clean clone>` before the image build, then `shiplist verify production --image <saved image> --root <app dir>` with
   `forbid` patterns for files that must never ship (`'**/*.map'`, `'**/.env*'`) and `build_output` for what the build
   adds.

For a package, use a `package` stage (`stages: [package]`, or `[test, package]` to prune before the tests too), and in
the release workflow build the package and run `shiplist verify package --archive <file> --root <folder>` before
publishing. In a monorepo, give each package its own `.shiplist.yml` in its directory and run shiplist there. The README
has the build command and `--root` for each package type.
