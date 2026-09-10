# prtoolbar

A tiny macOS menu bar app that lists your open GitHub pull requests.

- One line per PR with a coloured dot: green ready, red blocked (CI failed or
  changes requested), yellow waiting, grey draft.
- Under each PR: the requested reviewers as avatars, with ✓ / ✗ / 💬 / ⏳.
- Click a PR to open it in your browser.
- Count of open PRs next to the menu bar icon. Refreshes every minute. Past
  50 open PRs the title shows `50+` and the menu ends with `Showing 50 of N`.

Written in Rust with a native `NSStatusItem` menu. No webview, no Electron,
no config file.

## Requirements

- macOS 12 or newer.
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
| `src/icons.rs` | Status dots and the menu bar glyph, drawn at runtime. |
| `src/avatars.rs` | Avatar download, circular mask, avatar strips. |
| `src/menu.rs` | Builds the `NSMenu` from a snapshot. |
| `src/worker.rs` | Background refresh thread. |
| `src/main.rs` | Event loop wiring. |
| `docs/superpowers/specs/` | Design document. |
| `docs/superpowers/plans/` | Implementation plan. |

## Development

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

Lints are configured in `Cargo.toml` (`clippy::pedantic`, `unsafe_code = forbid`).
CI runs the same on `macos-latest`, plus `cargo deny` for licences and advisories.

## License

MIT, see `LICENSE`.
