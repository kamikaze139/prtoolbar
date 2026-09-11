# prtoolbar

A tiny macOS menu bar app that lists your open GitHub pull requests.

## Homebrew installation

Install the universal Mac app with Homebrew:

```sh
brew install --cask kamikaze139/tap/prtoolbar
gh auth login --hostname github.com
open -a prtoolbar
```

Homebrew installs the universal app in Applications and includes GitHub CLI.
Skip `gh auth login` if you are already signed in. No Rust compiler is needed.

Apple signing credentials are optional. If an unnotarized build is blocked on
first launch, try opening it, then go to **System Settings → Privacy & Security
→ Open Anyway**.

To update or remove it:

```sh
brew upgrade --cask kamikaze139/tap/prtoolbar
open -a prtoolbar
# Or remove it:
brew uninstall --cask prtoolbar
```

Maintainers: see [Homebrew publishing setup](#homebrew-publishing-setup).

## Requirements

- macOS 12 or newer.
- Rust 1.88 or newer when building from source.
- A GitHub token, either in `GITHUB_TOKEN` or via the GitHub CLI
  (`gh auth login`). The token needs read access to the repositories that
  hold your PRs.

## Run

```sh
cargo run --release
```

Or with [just](https://github.com/casey/just): `just run` for a debug build with
logging, `just check` for the same checks CI runs.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `GITHUB_TOKEN` | – | Token to use. Falls back to `gh auth token`. |
| `PRTOOLBAR_INTERVAL_SECS` | `60` | Refresh interval in seconds (minimum 15). |
| `RUST_LOG` | `warn` | Log filter, e.g. `prtoolbar=debug`. |

## Build an app bundle

```sh
cargo install cargo-bundle
just bundle          # → target/release/bundle/osx/prtoolbar.app
```

The bundle sets `LSUIElement` so the app has no Dock icon.

## Project layout

| Path | What |
|---|---|
| `src/model.rs` | Domain types and the status rules. Pure, fully tested. |
| `src/github.rs` | GraphQL query and response mapping. |
| `src/auth.rs` | Token discovery. |
| `src/icons.rs` | Menu bar glyph and avatar placeholder drawing. |
| `src/avatars.rs` | Avatar download, cache and circular mask. |
| `src/menu.rs` | Tray title and label helpers. |
| `src/dismissal.rs` | Outside-click policy, including the tray toggle race. |
| `src/tree.rs` | Stable repository/stack tree and collapse state identities. |
| `src/native.rs` | AppKit popover, outline table, event monitors and interop. |
| `src/events.rs` | Events passed from the worker to the main thread. |
| `src/worker.rs` | Background refresh thread. |
| `src/main.rs` | Event loop wiring. |
| `docs/superpowers/specs/` | Design document. |
| `docs/superpowers/plans/` | Implementation plan. |

## Development

Conventions, architecture rules and the commit-message format live in
[AGENTS.md](AGENTS.md). Read it before making a change.

```sh
just check     # everything CI runs
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

Lints are configured in `Cargo.toml` (`clippy::pedantic`, `unsafe_code = deny`).
Only `src/native.rs` permits documented unsafe Objective-C interop; UI objects
remain on the main thread.

CI (`.github/workflows/ci.yml`) runs on every push to `main` and every pull
request:

| Job | Runner | What it checks | On PRs |
|---|---|---|---|
| fmt, clippy, test | macOS | `cargo fmt --check`, `clippy -D warnings`, `cargo test`, release build — all `--locked` | yes, minus the release build |
| build on the declared MSRV | macOS | `cargo check` on the `rust-version` from `Cargo.toml` | no, `main` only |
| licences and advisories | Linux | `cargo deny` | yes |
| secret scan | Linux | `gitleaks` over the full git history | yes |

The two macOS-only jobs compile the crate, which needs the Apple SDK:
`src/native.rs` uses AppKit's accessory activation policy to hide the Dock icon. The slow
release build and the MSRV compile are gated to `main` because macOS runners
cost roughly ten times what Linux ones do, and `release.yml` builds the real
artifact at tag time anyway.


## Install a release

Download `prtoolbar-vX.Y.Z-universal-apple-darwin.zip` from the
[releases page](https://github.com/kamikaze139/prtoolbar/releases), unzip it and
move `prtoolbar.app` to `/Applications`. The binary is universal, so it runs
natively on both Apple silicon and Intel Macs.

Check the download against its published checksum:

```sh
shasum -a 256 -c prtoolbar-vX.Y.Z-universal-apple-darwin.zip.sha256
```

Without Apple signing credentials, releases use free ad-hoc signing and are
not notarized. If macOS blocks the first launch, try opening the app, then go to
**System Settings → Privacy & Security → Open Anyway**. The release notes state
whether that particular build was notarized.

## Releases and versioning

The version is derived from the commit messages, which follow
[Conventional Commits](https://www.conventionalcommits.org/). No one edits
`version` in `Cargo.toml` by hand.

| Commit prefix | Bump |
|---|---|
| `feat:` | minor — `0.1.0` → `0.2.0` |
| `fix:`, `perf:`, `refactor:`, `docs:` | patch — `0.1.0` → `0.1.1` |
| breaking (`feat!:` or a `BREAKING CHANGE:` trailer) | minor, while the version is below `1.0.0` |
| `chore:`, `ci:`, `build:`, `test:`, `style:` | none — these never cut a release on their own |

The flow is three steps, all in GitHub Actions:

1. Push to `main`. `release-plz` opens or updates a **release PR** that bumps
   `Cargo.toml` and `Cargo.lock` and writes `CHANGELOG.md`.
2. Merge that PR. `release-plz` tags the merge commit `vX.Y.Z` and creates the
   GitHub release, using the changelog section as the body.
3. The same workflow run then calls the **Release** workflow, which builds the
   universal `prtoolbar.app`, signs it, and attaches the zip and its SHA-256 to
   that release. It is called directly rather than triggered by the tag push,
   because GitHub will not start a run from a tag pushed with `GITHUB_TOKEN`.
4. The public Homebrew tap checks for new releases hourly, copies the completed
   archive, and updates its cask. Users receive it through `brew upgrade`.

Behaviour is configured in `release-plz.toml`. prtoolbar is an application, not
a library, so it is never published to crates.io (`publish = false`). Because
nothing is published, `git_only = true` tells `release-plz` to read the last
released version from the `v*` git tags; without it, it consults the crates.io
index, never finds the crate, concludes the current version is still an
unreleased first release, and opens no release PR at all.

To cut `1.0.0`, or to make breaking changes bump the major version from then
on, set the version in `Cargo.toml` to `1.0.0` once by hand and merge; the
automation takes over again from the next commit.

### Repository secrets

All of these are optional. The table says what degrades without each one.

| Secret | Used for | If unset |
|---|---|---|
| `RELEASE_PLZ_TOKEN` | Opening the release PR and pushing the tag | Falls back to `GITHUB_TOKEN`. Releases and app builds still happen; the only loss is that CI does not run on the release PR |
| `MACOS_CERTIFICATE_P12`, `MACOS_CERTIFICATE_PASSWORD`, `MACOS_SIGNING_IDENTITY` | Developer ID signing | The app is ad-hoc signed |
| `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD` | Apple notarization | Notarization is skipped; release notes and the cask explain the first-launch approval |

`RELEASE_PLZ_TOKEN` is a fine-grained personal access token scoped to this
repository with **Contents: read and write** and **Pull requests: read and
write**. It exists only to work around GitHub's rule that events created with
the built-in `GITHUB_TOKEN` do not trigger further workflows — here, that the
release PR opens without CI running on it. Closing and reopening that PR by
hand starts CI too.

`MACOS_CERTIFICATE_P12` is a base64 encoding of a Developer ID Application
certificate exported as `.p12`:

```sh
base64 -i DeveloperID.p12 | pbcopy
```

The Release workflow can always be run by hand from the Actions tab against any
existing tag, which is the escape hatch when a secret is missing or a build
needs repeating.

### Homebrew publishing setup

The public [kamikaze139/homebrew-tap](https://github.com/kamikaze139/homebrew-tap)
repository hosts the app ZIP, checksum and `Casks/prtoolbar.rb`. Its **Update
prtoolbar** workflow checks the public source repository's latest release hourly.
It can also be run manually from the tap's Actions tab for an immediate update.

The tap uses its own built-in `GITHUB_TOKEN` with **Contents: write**. No personal
access token, source-repository variable, or cross-repository secret is needed.
The source repository must remain public for this setup.

The source Release workflow uploads `homebrew.json` after attaching the ZIP and
checksum. This records the tag, archive hash and actual notarization status. The tap validates
that metadata and the archive checksum before publishing; missing assets are
retried on the next run. Current and older versions are skipped.

The tap's workflow template is [packaging/homebrew/update.yml](packaging/homebrew/update.yml).
Its `scripts/` directory contains copies of `render_cask.py`, `publish_homebrew.py`
and `sync_homebrew.py`; copy relevant changes there when modifying the tooling.
Apple signing and notarization remain optional.

No token beyond the built-in `GITHUB_TOKEN` is needed for a release-plz tag to
produce a build. The first release uses the existing `0.1.0` package version.

Validation runs in CI and locally with:

```sh
python3 -m unittest discover -s scripts -p 'test_*.py'
```

## Security

- The repository holds no credentials. The GitHub token is read at runtime from
  `GITHUB_TOKEN` or `gh auth token`, kept only in memory, sent only to
  `api.github.com` as an `Authorization` header, and never logged — failures
  report HTTP status codes, not the token.
- Every CI run scans the entire git history with
  [gitleaks](https://github.com/gitleaks/gitleaks) and fails on a hit. Run the
  same check locally with `just scan-secrets`.
- `cargo deny` checks dependency licences and advisories on every run.
- The crate denies unsafe code except for the documented AppKit boundary in `src/native.rs`.

## License

MIT, see `LICENSE`. The menu bar glyph is GitHub's Octicons pull-request
icon, also MIT; see `THIRD-PARTY-NOTICES.md`.
