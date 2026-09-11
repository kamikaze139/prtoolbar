# AGENTS.md

How to work in this repository. Applies to coding agents and humans alike.

prtoolbar is a native macOS menu bar app, written in Rust, that lists your open
GitHub pull requests. It targets macOS 12+ and Rust 1.88+ (`rust-version` in
`Cargo.toml` is the source of truth). There is no webview, no Electron and no
config file.

## The one gate

```sh
just check
```

That runs everything CI runs: `cargo fmt --all --check`, `cargo clippy
--all-targets --all-features -- -D warnings`, `cargo test --all-features`, and
the Python tests for the release tooling. **Do not report work as finished
until it passes.** Other useful recipes:

| Command | Does |
|---|---|
| `just run` | Debug build with `RUST_LOG=prtoolbar=debug` |
| `just fmt` | Format |
| `just deny` | Licence and advisory check (`cargo install cargo-deny`) |
| `just scan-secrets` | gitleaks over the whole history (`brew install gitleaks`) |
| `just bundle-universal` | The universal, signed `.app` exactly as release CI builds it |

## Commit messages and PR titles

**[Conventional Commits](https://www.conventionalcommits.org/), always.** This
is not a style preference: `release-plz` reads these messages to decide the
next version number and to write `CHANGELOG.md`. A sloppy prefix silently
produces a wrong release.

```
type(scope): imperative summary

Optional body explaining why, wrapped at 72 columns.

BREAKING CHANGE: what stopped working, for anyone upgrading.
```

| Type | Use for | Version effect |
|---|---|---|
| `feat` | New user-visible behaviour | minor — `0.1.0` → `0.2.0` |
| `fix` | Bug fix | patch |
| `perf` | Faster or lighter, same behaviour | patch |
| `refactor` | Restructuring with no behaviour change | patch |
| `docs` | Documentation only | patch |
| `test` | Tests only | none |
| `ci` | Workflows, actions | none |
| `build` | Build system, dependencies, packaging | none |
| `chore` | Anything else with no user impact | none |
| `style` | Formatting only | none |

Rules that matter:

- **Scope** is the module or area: `model`, `github`, `auth`, `icons`,
  `avatars`, `menu`, `native`, `tree`, `worker`, `release`, `homebrew`.
  Optional, but use it when the change is local to one.
- Summary in the **imperative mood**, lower case, no trailing full stop, and
  under about 72 characters. "add the reviewer column", not "Added ...".
- A breaking change takes a `!` after the type (`feat(github)!: …`) or a
  `BREAKING CHANGE:` footer. While the version is below `1.0.0` this bumps the
  minor, not the major.
- Types marked "none" above never cut a release **on their own**. A branch of
  nothing but `chore:` commits produces no new version, which is usually what
  you want — but do not label a real fix as `chore` to avoid a release.

**PR titles follow exactly the same format.** All three merge methods are
enabled on this repository, so a PR title can become the commit message that
`release-plz` parses. Write it as a valid Conventional Commit every time. The
PR body should say why the change exists and how it was verified.

## Architecture

The rule that shapes everything: **logic is pure and tested; AppKit is
quarantined in one file.** If you find yourself wanting to test something in
`native.rs`, that logic belongs in a pure module instead.

| File | Role | Pure? |
|---|---|---|
| `src/model.rs` | Domain types, status derivation, reviewer merging, `Snapshot` transitions | yes |
| `src/tree.rs` | Repository/stack hierarchy for the popup table | yes |
| `src/github.rs` | GraphQL query, DTOs, mapping into `model` | yes apart from the request |
| `src/auth.rs` | Token discovery (`GITHUB_TOKEN`, then `gh auth token`) | yes apart from the subprocess |
| `src/menu.rs` | Tray tooltip and text helpers | yes |
| `src/icons.rs` | Bitmaps drawn at runtime, on a distance-field `Canvas` | yes |
| `src/avatars.rs` | Avatar download, cache, circular mask | fetch is injectable |
| `src/dismissal.rs` | Popover dismissal policy | yes |
| `src/worker.rs` | Background refresh thread | no |
| `src/events.rs` | Messages from the worker to the main thread | types only |
| `src/native.rs` | **All** AppKit and Objective-C | no |
| `src/main.rs` | Entry point only | no |
| `scripts/` | Python release tooling for the Homebrew tap | tested with `unittest` |

## Testing

- Unit tests live in the same file, in `#[cfg(test)] mod tests` **at the end of
  the file** — clippy's `items_after_test_module` fails the build otherwise.
- Write the test first and watch it fail before implementing. A test that has
  never failed has not been shown to test anything.
- Pure modules carry real coverage. Where a function does I/O, take the
  effectful part as a parameter so the core can be tested — see
  `auth::resolve_from` and `AvatarCache::fetch_missing_with`.
- `native.rs` tests only its pure helpers — `make_items` has two, covering
  outline parentage and identity across refreshes. Its AppKit rendering is not
  unit-testable: verify UI changes by running the app and looking at it
  (`screencapture` works for the menu bar; macOS returns black while a menu is
  tracking).
- Tests must not write outside `target/`, must not hit the network, and must
  not depend on wall-clock time or the developer's machine.

## Lints and style

- `clippy::pedantic` is on, warnings are denied. Fix the code rather than
  adding an `#[allow]`; if an allow is genuinely right, comment why.
- `unsafe_code = "deny"` crate-wide. `src/native.rs` carries the single
  file-level `#![allow(unsafe_code)]` because Objective-C interop needs it.
  **Give every `unsafe` block a `// SAFETY:` comment** saying what invariant
  makes it sound; the existing ones are not yet complete, so add them as you
  touch code. Do not introduce `unsafe` anywhere else.
- Match the surrounding comment density and naming. Comments explain *why*,
  not what the line already says.
- Doc comments (`///`) on public items; module headers (`//!`) say what the
  module is for.

## macOS specifics

- Every AppKit object is created and touched on the main thread. Work that
  blocks belongs on the worker thread, which talks back through `events.rs`.
- The menu bar glyph is GitHub's Octicons pull-request icon, transcribed into
  `icons::menubar_glyph` on the distance-field `Canvas`. If you change it,
  diff the result against the official SVG rather than eyeballing it, and keep
  `THIRD-PARTY-NOTICES.md` accurate.
- Menu bar images must be template images (pure black plus alpha) sized 36 px
  for 18 pt at 2x, so macOS tints them for light mode, dark mode and menu
  tracking. If you use an SF Symbol instead, give it an
  `NSImageSymbolConfiguration` — unconfigured symbols render lighter and
  smaller than the system's own icons.
- The menu bar shows the icon alone — no count, no badge. State belongs in the
  tooltip (`menu::tray_tooltip`) and the popup.

## Secrets

- **Never commit a credential.** CI scans the entire history with gitleaks and
  fails on a hit; run `just scan-secrets` before pushing anything sensitive.
- The GitHub token is read at runtime from `GITHUB_TOKEN` or `gh auth token`,
  held only in memory, and sent only to `api.github.com`. Never log it, never
  put it in an error message, never write it to disk.
- `.gitignore` already covers `.env`, `*.p12`, `*.pem`, `*.key`. Keep it that
  way.

## Releases and versioning

Versioning is automatic. **Do not hand-edit `version` in `Cargo.toml` and do
not hand-write `CHANGELOG.md`** — `release-plz` owns both.

1. Merge anything to `main`. `release-plz` commits the version bump and
   changelog back to `main` as `chore: release vX.Y.Z`.
2. It tags `vX.Y.Z` and creates the GitHub release. There is no release PR.
3. The same run then calls `release.yml`, which builds the universal `.app`,
   signs it and attaches it.

Every merge releases, so `feat:` gives a minor bump and every other type gives
a patch. Write the commit message for the changelog: it is the release notes.

Configuration is in `release-plz.toml`. The app is never published to a
registry (`publish = false` there), so `git_only = true` makes `release-plz`
derive the last released version from the `v*` git tags instead of the
crates.io index. `Cargo.toml` uses `publish = ["never-published"]` rather than
`publish = false` on purpose — see the comment there, and do not "simplify" it.
The one manual step is cutting `1.0.0`: set it by hand once, then automation
resumes.

## CI

`.github/workflows/ci.yml` on every push to `main` and every pull request:

| Job | Runner | Runs on PRs |
|---|---|---|
| fmt, clippy, test (+ Homebrew tooling tests, cask validation, release build) | macOS | yes, minus the release build |
| build on the declared MSRV | macOS | no, `main` only |
| licences and advisories (`cargo deny`) | Linux | yes |
| secret scan (gitleaks, full history) | Linux | yes |

The two macOS jobs compile the crate, which needs the Apple SDK: `native.rs`
is AppKit. The release build and the MSRV check are gated to `main` because
macOS runners cost roughly ten times what Linux ones do.

## Don't

- Don't disable or downgrade a lint to make CI pass.
- Don't add a dependency without checking it clears `cargo deny`, and prefer
  the system API over a crate for anything macOS already provides.
- Don't leave debugging scaffolding behind — preview harnesses, `dbg!`, tests
  that write into `/tmp`.
- Don't commit generated artefacts: `target/`, `*.app`, bundles.
- Don't widen the scope of a change beyond what was asked. Note the adjacent
  problem instead of fixing it silently.
- Don't claim something works without having run it. Paste the command output.
