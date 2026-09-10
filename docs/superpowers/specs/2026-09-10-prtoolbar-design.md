# prtoolbar — design

A macOS menu bar app, written in Rust, that lists all of your open GitHub pull
requests with their status and requested reviewers. Clicking a PR opens it in
the browser. Nothing else.

## Goals

- See at a glance: how many open PRs I have, and whether each one is green,
  blocked, waiting, or a draft.
- See who is reviewing each PR and where they stand (approved, changes
  requested, pending).
- One click to the PR's web page.
- KISS: native macOS menu, no webview, no async runtime, no config file
  required to get started.

## Non-goals (v1)

- PRs where I am the *reviewer* (may come in v2 as a second section).
- GitLab / Bitbucket / GitHub Enterprise hosts.
- Notifications, sounds, badges beyond the count.
- Merging, commenting, or any write action.
- Windows / Linux builds. The crates used are cross-platform, but only macOS
  is targeted and tested.

## Decisions made without user input

The session that produced this spec was non-interactive. These calls were
made by the author and are cheap to change before implementation starts:

| Topic | Decision | Alternative considered |
|---|---|---|
| Forge | GitHub only (`gh` is installed and logged in) | GitLab MRs |
| UI | Native `NSStatusItem` menu via `tray-icon` + `muda` | Tauri popover with HTML UI (prettier, ~10x heavier) |
| Event loop | `tao` (closure-based, simple, activation policy API) | `winit` 0.30+ (trait-based, more boilerplate) |
| HTTP | `ureq` 3 blocking client on a worker thread | `reqwest` + `tokio` |
| Auth | `GITHUB_TOKEN` env, else `gh auth token` | PAT in a config file, OAuth device flow |
| Scope | PRs authored by me | Also review-requested PRs |

## Architecture

```
┌──────────────┐  every 60 s / on demand  ┌──────────────────┐
│ worker thread│ ───────────────────────► │ GitHub GraphQL   │
│ (github.rs)  │ ◄─────────────────────── │ + avatar images  │
└──────┬───────┘   Vec<PullRequest>        └──────────────────┘
       │ EventLoopProxy::send_event(AppEvent::Loaded | Failed)
       ▼
┌──────────────┐   build menu            ┌──────────────────┐
│ tao event    │ ──────────────────────► │ NSStatusItem +   │
│ loop (main)  │ ◄────────────────────── │ NSMenu (tray-icon│
└──────────────┘   MenuEvent(id) → open  │ / muda)          │
                                         └──────────────────┘
```

Two threads. The main thread owns the event loop, the tray icon and the menu.
A single worker thread fetches data and sends immutable snapshots to the main
thread through `tao::event_loop::EventLoopProxy`. The main thread never
blocks on I/O.

### Modules

| File | Responsibility | Depends on |
|---|---|---|
| `src/main.rs` | Wire everything: create event loop, tray, spawn worker, dispatch events. Thin. | all below |
| `src/model.rs` | Pure domain types: `PullRequest`, `Reviewer`, `ReviewState`, `CiState`, `Status`. `Status::derive(&PullRequest)`. No I/O. | – |
| `src/github.rs` | GraphQL query string, response DTOs (`serde`), `fetch_open_prs(token) -> Result<Vec<PullRequest>>`. Maps DTOs into `model` types. | `model`, `ureq`, `serde_json` |
| `src/auth.rs` | `resolve_token() -> Result<String>`: env var, then `gh auth token`. | – |
| `src/avatars.rs` | `AvatarCache`: fetch avatar PNG/JPEG by URL, decode, resize to 16 px (32 px @2x), circular mask, in-memory `HashMap<String, RgbaImage>`. `compose_strip(&[RgbaImage]) -> RgbaImage`. | `image`, `ureq` |
| `src/icons.rs` | Generate status-dot icons (green/red/yellow/grey) and the menubar template icon at 1x/2x. Convert `RgbaImage` to `tray_icon::Icon` / `muda::Icon`. | `image`, `model` |
| `src/menu.rs` | `build_menu(&Snapshot) -> (Menu, HashMap<MenuId, Action>)`. Formats labels. `Action::OpenUrl(String) | Refresh | Quit`. | `model`, `icons`, `avatars`, `muda` |
| `src/worker.rs` | Spawns the refresh thread: loop { fetch → send event; wait 60 s or until a `Refresh` signal arrives on an `mpsc` channel }. | `github`, `auth`, `avatars` |

Rule of thumb: everything that can be unit-tested without a display or the
network lives in `model.rs`, `menu.rs` label formatting, `github.rs`
deserialization, and `avatars.rs` composition.

## Data

### GraphQL query

```graphql
query {
  viewer { login }
  search(query: "is:pr is:open author:@me archived:false sort:updated-desc",
         type: ISSUE, first: 50) {
    nodes {
      ... on PullRequest {
        number title url isDraft updatedAt
        repository { nameWithOwner }
        reviewDecision                       # APPROVED | CHANGES_REQUESTED | REVIEW_REQUIRED | null
        commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }  # SUCCESS | FAILURE | PENDING | ERROR | EXPECTED | null
        reviewRequests(first: 10) { nodes { requestedReviewer {
          ... on User { login avatarUrl(size: 64) }
          ... on Team { name avatarUrl(size: 64) } } } }
        latestReviews(first: 10) { nodes { state author { login avatarUrl(size: 64) } } }
      }
    }
  }
}
```

Endpoint `https://api.github.com/graphql`, header `Authorization: bearer <token>`,
`User-Agent: prtoolbar/<version>`. 50 PRs is the cap; more than that is not a
menu problem this tool should solve.

### Domain model (`model.rs`)

```rust
pub struct PullRequest {
    pub repo: String,          // "owner/name"
    pub number: u64,
    pub title: String,
    pub url: String,
    pub is_draft: bool,
    pub ci: CiState,           // Success | Failure | Pending | None
    pub decision: ReviewDecision, // Approved | ChangesRequested | Required | None
    pub reviewers: Vec<Reviewer>,
}

pub struct Reviewer {
    pub login: String,         // user login or team name
    pub avatar_url: String,
    pub state: ReviewState,    // Pending | Approved | ChangesRequested | Commented
}

pub enum Status { Ready, Blocked, Waiting, Draft }
```

`Status::derive`:

1. `is_draft` → `Draft` (grey)
2. `ci == Failure` or `decision == ChangesRequested` → `Blocked` (red)
3. `ci == Success` and `decision == Approved` → `Ready` (green)
4. otherwise → `Waiting` (yellow)

Reviewer list = union of `reviewRequests` (state `Pending`) and
`latestReviews` (state from the review), deduplicated by login, with the
review state winning over a pending request. Order: requested first, then
reviews, each in API order.

## Menu layout

```
●  owner/repo #123   Fix the flaky thing            ← click opens URL
   [🙂🙂]  alice ✓  bob ⏳                             ← disabled, avatar strip as icon
●  owner/repo #98    Add feature                     
   no reviewers requested                            ← disabled
──────────────────────────────
Refresh                                    ⌘R
Updated 12:03                                        ← disabled
──────────────────────────────
Quit prtoolbar                             ⌘Q
```

- Status dot: 16 px circle rendered at 2x (32 px bitmap) for Retina. Colors:
  green `#34C759`, red `#FF3B30`, yellow `#FFCC00`, grey `#8E8E93`
  (Apple system colors).
- PR label: `{repo} #{number}   {title}`, title truncated to 60 chars with `…`.
- Reviewer label: each reviewer as `{login}{symbol}` joined by two spaces.
  Symbols: `✓` approved, `✗` changes requested, `💬` commented, `⏳` pending.
- Reviewer icon: horizontal strip of up to 5 circular 16 px avatars with 2 px
  gaps, composed into one bitmap. Avatars that fail to load are drawn as a
  grey circle. More than 5 reviewers → 5 avatars and `+N` in the label.
- Menubar: template icon (monochrome pull-request glyph) with `set_title`
  showing the count, e.g. `⑂ 3`. Zero PRs → no title. While the first load is
  in flight → title `…`. On error → title `!` and an extra disabled item
  `⚠ {short error}` at the top of the menu, previous PR list preserved.
- Empty state: single disabled item `No open pull requests 🎉`.

## Behaviour

- Startup: set activation policy to `Accessory` (no Dock icon, works from
  `cargo run` without a bundle). Show tray icon immediately with `…`, spawn
  worker, worker fetches at once.
- Refresh interval: 60 s. `Refresh` menu item sends a signal on the worker's
  channel; the worker wakes immediately.
- Menu rebuild: on every `AppEvent::Loaded`, build a fresh `Menu`, swap it in
  with `TrayIcon::set_menu`, replace the id→action map. Rebuilding is cheap
  (tens of items) and avoids diffing logic.
- Click on PR item → `open::that(url)`. Any error from `open` is logged, not
  shown.
- Avatars are fetched by the worker *after* the PR list, so a slow CDN never
  delays the list. Cache is keyed by URL and lives for the process lifetime.
  Cache misses are fetched with a 5 s timeout.

## Error handling

| Failure | Behaviour |
|---|---|
| No token found | Tray shows `!`; menu item `⚠ No GitHub token — set GITHUB_TOKEN or run gh auth login`. Retry each interval. |
| HTTP / GraphQL error | Tray `!`, item `⚠ GitHub: {status or message}`; keep previous list. Retry next interval. |
| Rate limited (403/429) | Same as above; message says "rate limited". No special backoff beyond the 60 s interval. |
| Avatar fetch fails | Grey placeholder circle for that reviewer. Never fails the refresh. |
| `open` fails | `log::warn!`, nothing in the UI. |

Errors are `anyhow::Error` at boundaries; `thiserror` is not needed in v1.
Logging via `log` + `env_logger`, controlled by `RUST_LOG`.

## Configuration

None required. Optional environment variables:

| Var | Default | Meaning |
|---|---|---|
| `GITHUB_TOKEN` | – | Token; overrides `gh auth token`. Needs `repo` read scope (classic) or PR read (fine-grained). |
| `PRTOOLBAR_INTERVAL_SECS` | `60` | Refresh interval. Minimum 15. |
| `RUST_LOG` | `warn` | Log level. |

A config file is a v2 concern.

## Testing

- `model`: table-driven tests for `Status::derive` (every branch) and
  reviewer merging/dedup.
- `github`: deserialize a checked-in fixture
  `tests/fixtures/search_response.json` (covers user + team reviewers, null
  `statusCheckRollup`, null `reviewDecision`) and assert the mapped
  `Vec<PullRequest>`.
- `menu`: label formatting (truncation, reviewer symbols, `+N`).
- `avatars`: `compose_strip` produces the expected dimensions; circular mask
  makes corner pixels transparent.
- `auth`: env var takes precedence (test sets the var; `gh` path is not unit
  tested).
- No tests touch the network or a display. Manual smoke test via `cargo run`
  is part of the plan's final task.

## Packaging & repo hygiene

- Crate `prtoolbar`, edition 2024, `rust-toolchain.toml` pinning `stable`.
- `Cargo.toml` `[lints]`: `clippy::pedantic` warn, `unsafe_code = "forbid"`,
  `missing_docs` off (binary crate).
- `rustfmt.toml`, `.editorconfig`, `.gitignore`, MIT `LICENSE`, `README.md`.
- GitHub Actions `ci.yml` on `macos-latest`: `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo build --release`.
- `cargo-deny` config `deny.toml` (licenses + advisories).
- Release: `[package.metadata.bundle]` for `cargo-bundle` producing
  `prtoolbar.app` with `LSUIElement = true`. Not wired into CI in v1; a
  `just bundle` recipe in a `justfile` is enough.

## Dependencies (v1)

| Crate | Version | Why |
|---|---|---|
| `tao` | 0.37 | Event loop, `EventLoopProxy`, Accessory activation policy |
| `tray-icon` | 0.24 | `NSStatusItem`, re-exports `muda` for menus |
| `ureq` | 3 | Blocking HTTPS with rustls, tiny |
| `serde`, `serde_json` | 1 | GraphQL response parsing |
| `image` | 0.25 | Decode avatars, draw dots, compose strips (features: `png`, `jpeg`) |
| `open` | 5 | Open URLs |
| `anyhow` | 1 | Error plumbing |
| `log`, `env_logger` | 0.4 / 0.11 | Logging |

## Open questions for the user

None blocking. Things you might want to change:

1. Also list PRs where your review is requested (second section)?
2. Prefer a Tauri popover UI over a native menu?
3. Should the menubar icon turn red when any PR is blocked?
