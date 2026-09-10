# prtoolbar Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A macOS menu bar app in Rust that lists the user's open GitHub PRs with a status colour and reviewer avatars, and opens a PR on click.

**Architecture:** Main thread owns a `tao` event loop, a `tray-icon` status item and its `muda` menu. One worker thread fetches PRs over GitHub GraphQL with `ureq`, then avatars, and posts immutable snapshots to the main thread through `EventLoopProxy`. The menu is rebuilt from scratch on every update. All logic that does not need a display or the network lives in pure modules with unit tests.

**Tech Stack:** Rust 2024 (stable), `tao` 0.37, `tray-icon` 0.24 (re-exports `muda`), `ureq` 3, `serde`/`serde_json`, `image` 0.25, `open` 5, `anyhow`, `log`/`env_logger`, `jiff` 0.2.

**Spec:** `docs/superpowers/specs/2026-09-10-prtoolbar-design.md`

## Global Constraints

- Crate name `prtoolbar`, edition 2024, `rust-version = "1.85"`, `unsafe_code = "forbid"`. Because `forbid(unsafe_code)` is on, tests may not call `std::env::set_var`; pass environment values into functions as parameters instead.
- `clippy::pedantic` is on at warn level and CI runs with `-D warnings`. Every task ends with `cargo fmt --all --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test` passing.
- Only macOS is targeted. `muda::Menu` and menu items can only be created on the main thread, so `build_menu` is never called from tests. Tests cover the pure helpers next to it.
- All bitmaps handed to `muda`/`tray-icon` are 36 px tall (rendered at 18 pt).
- Apple system colours: green `#34C759`, red `#FF3B30`, yellow `#FFCC00`, grey `#8E8E93`.
- GitHub GraphQL endpoint `https://api.github.com/graphql`, header `Authorization: bearer <token>`, cap of 50 PRs.
- Refresh interval default 60 s, minimum 15 s, env `PRTOOLBAR_INTERVAL_SECS`.
- Commit after every task with a conventional-commit message.

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `Cargo.toml`, `rust-toolchain.toml`, `rustfmt.toml`, `.editorconfig`, `.gitignore`, `LICENSE`, `README.md`, `justfile`, `deny.toml`, `.github/workflows/ci.yml`, `bundle/Info.plist.ext` | done in Task 0 | Project hygiene |
| `src/main.rs` | exists as skeleton, rewritten in Task 8 | Event loop wiring, `App` state |
| `src/events.rs` | create | `AppEvent` enum shared by worker and main |
| `src/model.rs` | create | Domain types, `Status::derive`, `merge_reviewers`, `Snapshot` |
| `src/github.rs` | create | GraphQL query, DTOs, `parse_response`, `fetch_open_prs` |
| `tests/fixtures/search_response.json` | create | Recorded-shape GraphQL response |
| `src/auth.rs` | create | Token discovery |
| `src/icons.rs` | create | Status dots, menu bar glyph, `Icon` conversions |
| `src/avatars.rs` | create | `AvatarCache`, circular mask, strip composition |
| `src/menu.rs` | create | Labels, `tray_title`, `build_menu` |
| `src/worker.rs` | create | HTTP agents, refresh thread, interval parsing |

---

### Task 0: Scaffold (already done)

The repository already contains the crate scaffold, lint configuration, CI, licence, README, and a skeleton `src/main.rs` that shows a placeholder tray icon with a Quit item and proves that `EventLoopProxy` can be used from a worker thread and from `MenuEvent::set_event_handler`. Verify it before starting:

- [ ] **Step 1: Verify the scaffold builds clean**

Run: `cargo fmt --all --check && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: all three succeed; `cargo test` reports 0 tests.

---

### Task 1: Domain model

**Files:**
- Create: `src/model.rs`
- Modify: `src/main.rs` (add `mod model;`)

**Interfaces:**
- Produces: `CiState`, `ReviewDecision`, `ReviewState`, `Reviewer`, `PullRequest`, `Status`, `Status::derive(&PullRequest) -> Status`, `merge_reviewers(Vec<Reviewer>, Vec<Reviewer>) -> Vec<Reviewer>`, `Snapshot`.

- [ ] **Step 1: Write the failing tests**

Create `src/model.rs` with only the test module for now:

```rust
//! Pure domain types and the rules that turn GitHub data into a status.
//! No I/O lives here, so everything is unit-testable.

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(is_draft: bool, ci: CiState, decision: ReviewDecision) -> PullRequest {
        PullRequest {
            repo: "acme/widgets".into(),
            number: 7,
            title: "Add thing".into(),
            url: "https://github.com/acme/widgets/pull/7".into(),
            is_draft,
            ci,
            decision,
            reviewers: Vec::new(),
        }
    }

    fn reviewer(login: &str, state: ReviewState) -> Reviewer {
        Reviewer {
            login: login.into(),
            avatar_url: format!("https://avatars.example/{login}"),
            state,
        }
    }

    #[test]
    fn draft_wins_over_everything() {
        assert_eq!(
            Status::derive(&pr(true, CiState::Failure, ReviewDecision::Approved)),
            Status::Draft
        );
    }

    #[test]
    fn failing_ci_is_blocked() {
        assert_eq!(
            Status::derive(&pr(false, CiState::Failure, ReviewDecision::Approved)),
            Status::Blocked
        );
    }

    #[test]
    fn changes_requested_is_blocked() {
        assert_eq!(
            Status::derive(&pr(false, CiState::Success, ReviewDecision::ChangesRequested)),
            Status::Blocked
        );
    }

    #[test]
    fn approved_and_green_is_ready() {
        assert_eq!(
            Status::derive(&pr(false, CiState::Success, ReviewDecision::Approved)),
            Status::Ready
        );
    }

    #[test]
    fn nothing_configured_is_ready() {
        assert_eq!(
            Status::derive(&pr(false, CiState::None, ReviewDecision::None)),
            Status::Ready
        );
    }

    #[test]
    fn pending_ci_is_waiting() {
        assert_eq!(
            Status::derive(&pr(false, CiState::Pending, ReviewDecision::Approved)),
            Status::Waiting
        );
    }

    #[test]
    fn review_required_is_waiting() {
        assert_eq!(
            Status::derive(&pr(false, CiState::Success, ReviewDecision::Required)),
            Status::Waiting
        );
    }

    #[test]
    fn merge_keeps_requested_order_and_dedups_by_login() {
        let requested = vec![
            reviewer("alice", ReviewState::Pending),
            reviewer("bob", ReviewState::Pending),
        ];
        let reviewed = vec![
            reviewer("bob", ReviewState::Approved),
            reviewer("carol", ReviewState::Commented),
        ];
        let merged = merge_reviewers(requested, reviewed);
        let summary: Vec<(&str, ReviewState)> =
            merged.iter().map(|r| (r.login.as_str(), r.state)).collect();
        assert_eq!(
            summary,
            vec![
                ("alice", ReviewState::Pending),
                ("bob", ReviewState::Approved),
                ("carol", ReviewState::Commented),
            ]
        );
    }

    #[test]
    fn merge_never_downgrades_a_review_to_pending() {
        let requested = vec![reviewer("bob", ReviewState::Pending)];
        let reviewed = vec![reviewer("bob", ReviewState::ChangesRequested)];
        let merged = merge_reviewers(reviewed, requested);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].state, ReviewState::ChangesRequested);
    }
}
```

Add `mod model;` to the top of `src/main.rs` (after the `//!` doc comments).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test model 2>&1 | head -20`
Expected: compile errors, `cannot find type PullRequest` etc.

- [ ] **Step 3: Write the implementation**

Insert above the `#[cfg(test)]` module in `src/model.rs`:

```rust
/// Combined CI result of the PR's head commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiState {
    Success,
    Failure,
    Pending,
    /// No checks are configured.
    None,
}

/// GitHub's overall review decision for the PR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    Approved,
    ChangesRequested,
    /// Reviews are required but not complete.
    Required,
    /// The repository does not require reviews.
    None,
}

/// Where one reviewer stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewState {
    Pending,
    Approved,
    ChangesRequested,
    Commented,
}

/// A person or team that was asked to review, or that has reviewed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reviewer {
    /// User login or team name.
    pub login: String,
    pub avatar_url: String,
    pub state: ReviewState,
}

/// One open pull request authored by the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    /// `owner/name`.
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub is_draft: bool,
    pub ci: CiState,
    pub decision: ReviewDecision,
    pub reviewers: Vec<Reviewer>,
}

/// The single colour shown next to a PR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    /// Green: nothing is holding it back.
    Ready,
    /// Red: CI failed or changes were requested.
    Blocked,
    /// Yellow: waiting for CI or reviews.
    Waiting,
    /// Grey: draft.
    Draft,
}

impl Status {
    /// Collapse CI and review state into one colour.
    pub fn derive(pr: &PullRequest) -> Self {
        if pr.is_draft {
            return Self::Draft;
        }
        if pr.ci == CiState::Failure || pr.decision == ReviewDecision::ChangesRequested {
            return Self::Blocked;
        }
        let ci_ok = matches!(pr.ci, CiState::Success | CiState::None);
        let review_ok = matches!(pr.decision, ReviewDecision::Approved | ReviewDecision::None);
        if ci_ok && review_ok {
            return Self::Ready;
        }
        Self::Waiting
    }
}

/// Merge requested reviewers with submitted reviews.
///
/// Order is preserved (first list first). Duplicates by login are collapsed;
/// a submitted review always beats a pending request.
pub fn merge_reviewers(first: Vec<Reviewer>, second: Vec<Reviewer>) -> Vec<Reviewer> {
    let mut out: Vec<Reviewer> = Vec::with_capacity(first.len() + second.len());
    for reviewer in first.into_iter().chain(second) {
        if let Some(existing) = out.iter_mut().find(|r| r.login == reviewer.login) {
            if reviewer.state != ReviewState::Pending {
                existing.state = reviewer.state;
            }
        } else {
            out.push(reviewer);
        }
    }
    out
}

/// Everything the main thread needs to render the menu.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub prs: Vec<PullRequest>,
    /// Local wall-clock time of the last successful fetch, `HH:MM`.
    pub updated_at: Option<String>,
    /// Message of the most recent failed fetch, cleared on success.
    pub error: Option<String>,
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test model`
Expected: `test result: ok. 9 passed`

- [ ] **Step 5: Lint and commit**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings`
Expected: no warnings. (`dead_code` warnings for unused types are expected until Task 8 wires things up; silence them for now by adding `#![allow(dead_code)]` at the top of `src/main.rs` with the comment `// Removed in Task 8 once every module is wired up.`)

```bash
git add src/model.rs src/main.rs
git commit -m "feat(model): domain types, status derivation, reviewer merge"
```

---

### Task 2: GitHub GraphQL client

**Files:**
- Create: `src/github.rs`
- Create: `tests/fixtures/search_response.json`
- Modify: `src/main.rs` (add `mod github;`)

**Interfaces:**
- Consumes: `model::{CiState, PullRequest, ReviewDecision, ReviewState, Reviewer, merge_reviewers}`.
- Produces: `github::parse_response(&str) -> anyhow::Result<Vec<PullRequest>>`, `github::fetch_open_prs(&ureq::Agent, token: &str) -> anyhow::Result<Vec<PullRequest>>`.

- [ ] **Step 1: Create the fixture**

Create `tests/fixtures/search_response.json`:

```json
{
  "data": {
    "search": {
      "nodes": [
        {
          "number": 42,
          "title": "Fix the flaky integration test",
          "url": "https://github.com/acme/widgets/pull/42",
          "isDraft": false,
          "repository": { "nameWithOwner": "acme/widgets" },
          "reviewDecision": "APPROVED",
          "commits": {
            "nodes": [
              { "commit": { "statusCheckRollup": { "state": "SUCCESS" } } }
            ]
          },
          "reviewRequests": {
            "nodes": [
              { "requestedReviewer": { "login": "alice", "avatarUrl": "https://avatars.githubusercontent.com/u/1?s=64&v=4" } },
              { "requestedReviewer": { "name": "platform-team", "avatarUrl": "https://avatars.githubusercontent.com/t/9?s=64&v=4" } },
              { "requestedReviewer": null }
            ]
          },
          "latestReviews": {
            "nodes": [
              { "state": "APPROVED", "author": { "login": "bob", "avatarUrl": "https://avatars.githubusercontent.com/u/2?s=64&v=4" } },
              { "state": "DISMISSED", "author": { "login": "dave", "avatarUrl": "https://avatars.githubusercontent.com/u/4?s=64&v=4" } }
            ]
          }
        },
        {
          "number": 3,
          "title": "WIP: rewrite everything",
          "url": "https://github.com/acme/tools/pull/3",
          "isDraft": true,
          "repository": { "nameWithOwner": "acme/tools" },
          "reviewDecision": null,
          "commits": {
            "nodes": [
              { "commit": { "statusCheckRollup": null } }
            ]
          },
          "reviewRequests": { "nodes": [] },
          "latestReviews": { "nodes": [] }
        }
      ]
    }
  }
}
```

- [ ] **Step 2: Write the failing tests**

Create `src/github.rs`:

```rust
//! GitHub GraphQL client: one query, its DTOs, and the mapping into `model`.

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/search_response.json");

    #[test]
    fn parses_fixture_into_pull_requests() {
        let prs = parse_response(FIXTURE).expect("fixture parses");
        assert_eq!(prs.len(), 2);

        let first = &prs[0];
        assert_eq!(first.repo, "acme/widgets");
        assert_eq!(first.number, 42);
        assert_eq!(first.url, "https://github.com/acme/widgets/pull/42");
        assert!(!first.is_draft);
        assert_eq!(first.ci, CiState::Success);
        assert_eq!(first.decision, ReviewDecision::Approved);

        let reviewers: Vec<(&str, ReviewState)> =
            first.reviewers.iter().map(|r| (r.login.as_str(), r.state)).collect();
        assert_eq!(
            reviewers,
            vec![
                ("alice", ReviewState::Pending),
                ("platform-team", ReviewState::Pending),
                ("bob", ReviewState::Approved),
            ],
            "null reviewer and DISMISSED review are dropped"
        );

        let second = &prs[1];
        assert!(second.is_draft);
        assert_eq!(second.ci, CiState::None);
        assert_eq!(second.decision, ReviewDecision::None);
        assert!(second.reviewers.is_empty());
    }

    #[test]
    fn graphql_errors_become_an_error() {
        let body = r#"{"data":null,"errors":[{"message":"Bad credentials"},{"message":"nope"}]}"#;
        let err = parse_response(body).unwrap_err();
        assert_eq!(err.to_string(), "GitHub: Bad credentials; nope");
    }

    #[test]
    fn missing_data_is_an_error() {
        let err = parse_response(r"{}").unwrap_err();
        assert_eq!(err.to_string(), "GitHub returned no data");
    }

    #[test]
    fn ci_state_mapping() {
        assert_eq!(ci_state("SUCCESS"), CiState::Success);
        assert_eq!(ci_state("FAILURE"), CiState::Failure);
        assert_eq!(ci_state("ERROR"), CiState::Failure);
        assert_eq!(ci_state("PENDING"), CiState::Pending);
        assert_eq!(ci_state("EXPECTED"), CiState::Pending);
    }
}
```

Add `mod github;` to `src/main.rs`.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test github 2>&1 | head -20`
Expected: compile errors about `parse_response` and `ci_state` missing.

- [ ] **Step 4: Write the implementation**

Insert above the test module in `src/github.rs`:

```rust
use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use ureq::Agent;

use crate::model::{
    CiState, PullRequest, ReviewDecision, ReviewState, Reviewer, merge_reviewers,
};

const ENDPOINT: &str = "https://api.github.com/graphql";

/// Open PRs authored by the token owner, newest activity first.
const QUERY: &str = r#"
query {
  search(query: "is:pr is:open author:@me archived:false sort:updated-desc", type: ISSUE, first: 50) {
    nodes {
      ... on PullRequest {
        number title url isDraft
        repository { nameWithOwner }
        reviewDecision
        commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
        reviewRequests(first: 10) { nodes { requestedReviewer {
          ... on User { login avatarUrl(size: 64) }
          ... on Team { name avatarUrl(size: 64) } } } }
        latestReviews(first: 10) { nodes { state author { login avatarUrl(size: 64) } } }
      }
    }
  }
}
"#";

/// Fetch the user's open pull requests.
///
/// The agent must be configured with `http_status_as_error(false)` so that
/// non-2xx responses reach the status mapping below.
pub fn fetch_open_prs(agent: &Agent, token: &str) -> Result<Vec<PullRequest>> {
    let mut response = agent
        .post(ENDPOINT)
        .header("Authorization", format!("bearer {token}"))
        .send_json(serde_json::json!({ "query": QUERY }))
        .context("GitHub request failed")?;

    let status = response.status().as_u16();
    match status {
        200 => {}
        401 => bail!("GitHub rejected the token (401)"),
        403 | 429 => bail!("GitHub rate limited (HTTP {status})"),
        other => bail!("GitHub HTTP {other}"),
    }

    let body = response
        .body_mut()
        .read_to_string()
        .context("reading GitHub response")?;
    parse_response(&body)
}

/// Turn a raw GraphQL response body into pull requests.
pub fn parse_response(body: &str) -> Result<Vec<PullRequest>> {
    let parsed: GraphQlResponse =
        serde_json::from_str(body).context("GitHub returned unexpected JSON")?;

    if let Some(errors) = parsed.errors.filter(|errors| !errors.is_empty()) {
        let messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
        bail!("GitHub: {}", messages.join("; "));
    }

    let data = parsed.data.ok_or_else(|| anyhow!("GitHub returned no data"))?;
    Ok(data.search.nodes.into_iter().map(PullRequest::from).collect())
}

// --- DTOs -------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct GraphQlResponse {
    data: Option<Data>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Debug, Deserialize)]
struct GraphQlError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct Data {
    search: Nodes<PrNode>,
}

#[derive(Debug, Deserialize)]
struct Nodes<T> {
    nodes: Vec<T>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrNode {
    number: u64,
    title: String,
    url: String,
    is_draft: bool,
    repository: Repository,
    review_decision: Option<String>,
    commits: Nodes<CommitNode>,
    review_requests: Nodes<ReviewRequest>,
    latest_reviews: Nodes<Review>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Repository {
    name_with_owner: String,
}

#[derive(Debug, Deserialize)]
struct CommitNode {
    commit: Commit,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Commit {
    status_check_rollup: Option<Rollup>,
}

#[derive(Debug, Deserialize)]
struct Rollup {
    state: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewRequest {
    requested_reviewer: Option<Actor>,
}

#[derive(Debug, Deserialize)]
struct Review {
    state: String,
    author: Option<Actor>,
}

/// A `User` (has `login`), a `Team` (has `name`), or something else (neither).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Actor {
    login: Option<String>,
    name: Option<String>,
    avatar_url: Option<String>,
}

impl Actor {
    fn into_reviewer(self, state: ReviewState) -> Option<Reviewer> {
        let login = self.login.or(self.name)?;
        Some(Reviewer {
            login,
            avatar_url: self.avatar_url.unwrap_or_default(),
            state,
        })
    }
}

impl From<PrNode> for PullRequest {
    fn from(node: PrNode) -> Self {
        let ci = node
            .commits
            .nodes
            .first()
            .and_then(|c| c.commit.status_check_rollup.as_ref())
            .map_or(CiState::None, |rollup| ci_state(&rollup.state));

        let requested: Vec<Reviewer> = node
            .review_requests
            .nodes
            .into_iter()
            .filter_map(|r| r.requested_reviewer)
            .filter_map(|actor| actor.into_reviewer(ReviewState::Pending))
            .collect();

        let reviewed: Vec<Reviewer> = node
            .latest_reviews
            .nodes
            .into_iter()
            .filter_map(|review| {
                let state = review_state(&review.state)?;
                review.author?.into_reviewer(state)
            })
            .collect();

        Self {
            repo: node.repository.name_with_owner,
            number: node.number,
            title: node.title,
            url: node.url,
            is_draft: node.is_draft,
            ci,
            decision: review_decision(node.review_decision.as_deref()),
            reviewers: merge_reviewers(requested, reviewed),
        }
    }
}

fn ci_state(state: &str) -> CiState {
    match state {
        "SUCCESS" => CiState::Success,
        "FAILURE" | "ERROR" => CiState::Failure,
        _ => CiState::Pending,
    }
}

fn review_decision(decision: Option<&str>) -> ReviewDecision {
    match decision {
        Some("APPROVED") => ReviewDecision::Approved,
        Some("CHANGES_REQUESTED") => ReviewDecision::ChangesRequested,
        Some("REVIEW_REQUIRED") => ReviewDecision::Required,
        _ => ReviewDecision::None,
    }
}

/// `DISMISSED` and `PENDING` (an unsubmitted review) are not shown.
fn review_state(state: &str) -> Option<ReviewState> {
    match state {
        "APPROVED" => Some(ReviewState::Approved),
        "CHANGES_REQUESTED" => Some(ReviewState::ChangesRequested),
        "COMMENTED" => Some(ReviewState::Commented),
        _ => None,
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test github`
Expected: `test result: ok. 4 passed`

- [ ] **Step 6: Lint and commit**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings`

```bash
git add src/github.rs tests/fixtures/search_response.json src/main.rs
git commit -m "feat(github): GraphQL query, DTOs and mapping to domain model"
```

---

### Task 3: Token discovery

**Files:**
- Create: `src/auth.rs`
- Modify: `src/main.rs` (add `mod auth;`)

**Interfaces:**
- Produces: `auth::resolve_token() -> anyhow::Result<String>`, `auth::resolve_from(Option<String>, impl FnOnce() -> Option<String>) -> anyhow::Result<String>`.

- [ ] **Step 1: Write the failing tests**

Create `src/auth.rs`:

```rust
//! Find a GitHub token: `GITHUB_TOKEN`, then the `gh` CLI.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_token_wins() {
        let token = resolve_from(Some(" ghp_env ".into()), || Some("ghp_cli".into())).unwrap();
        assert_eq!(token, "ghp_env");
    }

    #[test]
    fn empty_env_falls_back_to_gh() {
        let token = resolve_from(Some("   ".into()), || Some("ghp_cli".into())).unwrap();
        assert_eq!(token, "ghp_cli");
    }

    #[test]
    fn missing_everything_is_a_clear_error() {
        let err = resolve_from(None, || None).unwrap_err();
        assert_eq!(
            err.to_string(),
            "No GitHub token: set GITHUB_TOKEN or run `gh auth login`"
        );
    }
}
```

Add `mod auth;` to `src/main.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test auth 2>&1 | head`
Expected: compile error, `resolve_from` not found.

- [ ] **Step 3: Write the implementation**

Insert above the test module:

```rust
use std::process::Command;

use anyhow::{Result, anyhow};

/// Locations tried for `gh`. App bundles launched from Finder do not inherit
/// the shell `PATH`, so Homebrew's prefixes are tried explicitly.
const GH_CANDIDATES: &[&str] = &["gh", "/opt/homebrew/bin/gh", "/usr/local/bin/gh"];

/// Resolve a token from the environment, falling back to `gh auth token`.
pub fn resolve_token() -> Result<String> {
    resolve_from(std::env::var("GITHUB_TOKEN").ok(), gh_auth_token)
}

/// Pure core of [`resolve_token`], testable without touching the environment.
pub fn resolve_from(
    env_token: Option<String>,
    gh_lookup: impl FnOnce() -> Option<String>,
) -> Result<String> {
    if let Some(token) = env_token
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
    {
        return Ok(token);
    }
    gh_lookup().ok_or_else(|| anyhow!("No GitHub token: set GITHUB_TOKEN or run `gh auth login`"))
}

fn gh_auth_token() -> Option<String> {
    GH_CANDIDATES.iter().find_map(|gh| {
        let output = Command::new(gh).args(["auth", "token"]).output().ok()?;
        if !output.status.success() {
            return None;
        }
        let token = String::from_utf8(output.stdout).ok()?.trim().to_owned();
        (!token.is_empty()).then_some(token)
    })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test auth`
Expected: `test result: ok. 3 passed`

- [ ] **Step 5: Lint and commit**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings`

```bash
git add src/auth.rs src/main.rs
git commit -m "feat(auth): resolve token from GITHUB_TOKEN or gh CLI"
```

---

### Task 4: Icons

**Files:**
- Create: `src/icons.rs`
- Modify: `src/main.rs` (add `mod icons;`)

**Interfaces:**
- Consumes: `model::Status`.
- Produces: `icons::ICON_PX: u32 = 36`, `icons::status_dot(Status) -> RgbaImage`, `icons::menubar_glyph() -> RgbaImage`, `icons::fill_circle(&mut RgbaImage, cx: f32, cy: f32, radius: f32, rgb: [u8; 3])`, `icons::tray_icon(&RgbaImage) -> tray_icon::Icon`, `icons::menu_icon(&RgbaImage) -> tray_icon::menu::Icon`.

- [ ] **Step 1: Write the failing tests**

Create `src/icons.rs`:

```rust
//! Bitmaps drawn at runtime: status dots and the menu bar glyph.
//!
//! Everything is 36 px tall. `muda` and `tray-icon` scale images to 18 pt on
//! macOS, so 36 px is exactly 2x for Retina displays.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Status;

    #[test]
    fn status_dot_is_coloured_in_the_middle_and_clear_in_the_corner() {
        let dot = status_dot(Status::Ready);
        assert_eq!((dot.width(), dot.height()), (ICON_PX, ICON_PX));
        let centre = dot.get_pixel(ICON_PX / 2, ICON_PX / 2).0;
        assert_eq!(centre, [0x34, 0xC7, 0x59, 0xFF]);
        assert_eq!(dot.get_pixel(0, 0).0[3], 0, "corner is transparent");
    }

    #[test]
    fn every_status_has_a_distinct_colour() {
        let colours: Vec<[u8; 4]> = [Status::Ready, Status::Blocked, Status::Waiting, Status::Draft]
            .iter()
            .map(|s| status_dot(*s).get_pixel(ICON_PX / 2, ICON_PX / 2).0)
            .collect();
        for (i, a) in colours.iter().enumerate() {
            for b in &colours[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn menubar_glyph_is_a_black_template_image() {
        let glyph = menubar_glyph();
        assert_eq!((glyph.width(), glyph.height()), (ICON_PX, ICON_PX));
        let opaque = glyph.pixels().filter(|p| p.0[3] > 0).count();
        assert!(opaque > 50, "glyph has visible pixels, got {opaque}");
        assert!(
            glyph.pixels().all(|p| p.0[0] == 0 && p.0[1] == 0 && p.0[2] == 0),
            "template images must be pure black plus alpha"
        );
    }

    #[test]
    fn conversions_keep_dimensions() {
        let dot = status_dot(Status::Waiting);
        let _tray = tray_icon(&dot);
        let _menu = menu_icon(&dot);
    }
}
```

Add `mod icons;` to `src/main.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test icons 2>&1 | head`
Expected: compile errors.

- [ ] **Step 3: Write the implementation**

Insert above the test module:

```rust
use image::{Rgba, RgbaImage};

use crate::model::Status;

/// Height (and width, for square icons) of every bitmap in pixels.
pub const ICON_PX: u32 = 36;

const GREEN: [u8; 3] = [0x34, 0xC7, 0x59];
const RED: [u8; 3] = [0xFF, 0x3B, 0x30];
const YELLOW: [u8; 3] = [0xFF, 0xCC, 0x00];
const GREY: [u8; 3] = [0x8E, 0x8E, 0x93];
const BLACK: [u8; 3] = [0, 0, 0];

/// A 20 px coloured circle centred on a transparent 36 px canvas.
pub fn status_dot(status: Status) -> RgbaImage {
    let rgb = match status {
        Status::Ready => GREEN,
        Status::Blocked => RED,
        Status::Waiting => YELLOW,
        Status::Draft => GREY,
    };
    let mut img = RgbaImage::new(ICON_PX, ICON_PX);
    let centre = ICON_PX as f32 / 2.0;
    fill_circle(&mut img, centre, centre, 10.0, rgb);
    img
}

/// A monochrome pull-request glyph (two branch dots joined by a line, with a
/// merge arm) for use as a template image in the menu bar.
pub fn menubar_glyph() -> RgbaImage {
    let mut img = RgbaImage::new(ICON_PX, ICON_PX);
    // Left column: top dot, vertical line, bottom dot.
    ring(&mut img, 9.0, 8.0, 5.0, 2.5);
    fill_rect(&mut img, 8, 13, 2, 11, BLACK);
    ring(&mut img, 9.0, 28.0, 5.0, 2.5);
    // Right column: arm coming from the top-left, going down to a dot.
    fill_rect(&mut img, 14, 7, 9, 2, BLACK);
    fill_rect(&mut img, 26, 7, 2, 16, BLACK);
    ring(&mut img, 27.0, 28.0, 5.0, 2.5);
    // Small arrow head at the end of the arm.
    fill_rect(&mut img, 22, 4, 2, 2, BLACK);
    fill_rect(&mut img, 22, 10, 2, 2, BLACK);
    img
}

/// Anti-aliased filled circle.
pub fn fill_circle(img: &mut RgbaImage, cx: f32, cy: f32, radius: f32, rgb: [u8; 3]) {
    for (x, y, pixel) in img.enumerate_pixels_mut() {
        let dx = x as f32 + 0.5 - cx;
        let dy = y as f32 + 0.5 - cy;
        let coverage = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
        if coverage > 0.0 {
            blend(pixel, rgb, coverage);
        }
    }
}

/// Anti-aliased ring (circle outline) with the given stroke width.
fn ring(img: &mut RgbaImage, cx: f32, cy: f32, radius: f32, stroke: f32) {
    let outer = radius;
    let inner = radius - stroke;
    for (x, y, pixel) in img.enumerate_pixels_mut() {
        let dx = x as f32 + 0.5 - cx;
        let dy = y as f32 + 0.5 - cy;
        let d = (dx * dx + dy * dy).sqrt();
        let coverage = (outer + 0.5 - d).clamp(0.0, 1.0) * (d - inner + 0.5).clamp(0.0, 1.0);
        if coverage > 0.0 {
            blend(pixel, BLACK, coverage);
        }
    }
}

fn fill_rect(img: &mut RgbaImage, x0: u32, y0: u32, w: u32, h: u32, rgb: [u8; 3]) {
    for y in y0..(y0 + h).min(img.height()) {
        for x in x0..(x0 + w).min(img.width()) {
            *img.get_pixel_mut(x, y) = Rgba([rgb[0], rgb[1], rgb[2], 0xFF]);
        }
    }
}

/// Alpha-blend `rgb` at `coverage` over an existing pixel.
fn blend(pixel: &mut Rgba<u8>, rgb: [u8; 3], coverage: f32) {
    let alpha = (coverage * 255.0).round() as u8;
    if pixel.0[3] == 0 || alpha == 0xFF {
        *pixel = Rgba([rgb[0], rgb[1], rgb[2], alpha]);
    } else {
        pixel.0[3] = pixel.0[3].max(alpha);
    }
}

/// Convert to the status item icon type.
pub fn tray_icon(img: &RgbaImage) -> tray_icon::Icon {
    tray_icon::Icon::from_rgba(img.as_raw().clone(), img.width(), img.height())
        .expect("RGBA buffer matches its dimensions")
}

/// Convert to the menu item icon type.
pub fn menu_icon(img: &RgbaImage) -> tray_icon::menu::Icon {
    tray_icon::menu::Icon::from_rgba(img.as_raw().clone(), img.width(), img.height())
        .expect("RGBA buffer matches its dimensions")
}
```

Clippy pedantic will flag `x as f32` (`cast_precision_loss`) and `as u8` (`cast_possible_truncation`, `cast_sign_loss`). Add `#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]` at the top of `src/icons.rs` with the comment `// Pixel maths on 36 px canvases; the casts are exact.`

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test icons`
Expected: `test result: ok. 4 passed`

- [ ] **Step 5: Lint and commit**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings`

```bash
git add src/icons.rs src/main.rs
git commit -m "feat(icons): status dots and menu bar glyph drawn at runtime"
```

---

### Task 5: Avatars

**Files:**
- Create: `src/avatars.rs`
- Modify: `src/main.rs` (add `mod avatars;`)

**Interfaces:**
- Consumes: `icons::{ICON_PX, fill_circle}`.
- Produces: `avatars::AVATAR_PX: u32 = 32`, `avatars::MAX_AVATARS: usize = 5`, `avatars::AvatarCache` with `fn fetch_missing(&mut self, agent: &ureq::Agent, urls: impl IntoIterator<Item = String>) -> HashMap<String, RgbaImage>`, `avatars::placeholder() -> RgbaImage`, `avatars::circle_mask(RgbaImage) -> RgbaImage`, `avatars::compose_strip(&[&RgbaImage]) -> RgbaImage`, `avatars::decode(&[u8]) -> anyhow::Result<RgbaImage>`.

- [ ] **Step 1: Write the failing tests**

Create `src/avatars.rs`:

```rust
//! Reviewer avatars: download, circular mask, and side-by-side strips.

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn solid(rgb: [u8; 3]) -> RgbaImage {
        RgbaImage::from_pixel(AVATAR_PX, AVATAR_PX, Rgba([rgb[0], rgb[1], rgb[2], 0xFF]))
    }

    #[test]
    fn circle_mask_clears_corners_and_keeps_centre() {
        let masked = circle_mask(solid([10, 20, 30]));
        assert_eq!(masked.get_pixel(0, 0).0[3], 0);
        assert_eq!(masked.get_pixel(AVATAR_PX - 1, AVATAR_PX - 1).0[3], 0);
        assert_eq!(masked.get_pixel(AVATAR_PX / 2, AVATAR_PX / 2).0, [10, 20, 30, 0xFF]);
    }

    #[test]
    fn strip_dimensions_follow_avatar_count() {
        let a = solid([1, 1, 1]);
        let b = solid([2, 2, 2]);
        let strip = compose_strip(&[&a, &b]);
        assert_eq!(strip.height(), ICON_PX);
        assert_eq!(strip.width(), 2 * AVATAR_PX + GAP_PX);
        // First avatar starts at x = 0, second after avatar + gap; both vertically centred.
        let y = ICON_PX / 2;
        assert_eq!(strip.get_pixel(AVATAR_PX / 2, y).0, [1, 1, 1, 0xFF]);
        assert_eq!(strip.get_pixel(AVATAR_PX + GAP_PX + AVATAR_PX / 2, y).0, [2, 2, 2, 0xFF]);
        assert_eq!(strip.get_pixel(AVATAR_PX + GAP_PX / 2, y).0[3], 0, "gap is transparent");
    }

    #[test]
    fn strip_caps_at_max_avatars() {
        let a = solid([1, 1, 1]);
        let many: Vec<&RgbaImage> = std::iter::repeat_n(&a, MAX_AVATARS + 3).collect();
        let strip = compose_strip(&many);
        let n = MAX_AVATARS as u32;
        assert_eq!(strip.width(), n * AVATAR_PX + (n - 1) * GAP_PX);
    }

    #[test]
    fn empty_strip_is_a_single_transparent_column() {
        let strip = compose_strip(&[]);
        assert_eq!((strip.width(), strip.height()), (1, ICON_PX));
    }

    #[test]
    fn decode_resizes_and_masks_a_png() {
        let big = RgbaImage::from_pixel(64, 64, Rgba([200, 100, 50, 0xFF]));
        let mut bytes = std::io::Cursor::new(Vec::new());
        big.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let avatar = decode(bytes.get_ref()).unwrap();
        assert_eq!((avatar.width(), avatar.height()), (AVATAR_PX, AVATAR_PX));
        assert_eq!(avatar.get_pixel(AVATAR_PX / 2, AVATAR_PX / 2).0, [200, 100, 50, 0xFF]);
        assert_eq!(avatar.get_pixel(0, 0).0[3], 0);
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(decode(b"not an image").is_err());
    }

    #[test]
    fn placeholder_is_grey_and_round() {
        let p = placeholder();
        assert_eq!((p.width(), p.height()), (AVATAR_PX, AVATAR_PX));
        assert_eq!(p.get_pixel(AVATAR_PX / 2, AVATAR_PX / 2).0, [0x8E, 0x8E, 0x93, 0xFF]);
        assert_eq!(p.get_pixel(0, 0).0[3], 0);
    }
}
```

Add `mod avatars;` to `src/main.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test avatars 2>&1 | head`
Expected: compile errors.

- [ ] **Step 3: Write the implementation**

Insert above the test module:

```rust
use std::collections::HashMap;

use anyhow::{Context, Result};
use image::imageops::{self, FilterType};
use image::{Rgba, RgbaImage};
use ureq::Agent;

use crate::icons::{ICON_PX, fill_circle};

/// Diameter of one avatar in pixels (renders at 16 pt).
pub const AVATAR_PX: u32 = 32;
/// Horizontal gap between avatars in a strip.
pub const GAP_PX: u32 = 6;
/// Avatars shown per PR before the label falls back to `+N`.
pub const MAX_AVATARS: usize = 5;

const GREY: [u8; 3] = [0x8E, 0x8E, 0x93];

/// Process-lifetime cache of decoded, masked avatars keyed by URL.
#[derive(Debug, Default)]
pub struct AvatarCache {
    images: HashMap<String, RgbaImage>,
}

impl AvatarCache {
    /// Fetch every URL not yet cached and return only those new entries.
    ///
    /// Failures are logged and cached as a placeholder so they are not retried
    /// on every refresh. Empty URLs are ignored.
    pub fn fetch_missing(
        &mut self,
        agent: &Agent,
        urls: impl IntoIterator<Item = String>,
    ) -> HashMap<String, RgbaImage> {
        let mut fresh = HashMap::new();
        for url in urls {
            if url.is_empty() || self.images.contains_key(&url) || fresh.contains_key(&url) {
                continue;
            }
            let image = match fetch(agent, &url) {
                Ok(image) => image,
                Err(err) => {
                    log::warn!("avatar {url}: {err:#}");
                    placeholder()
                }
            };
            fresh.insert(url, image);
        }
        self.images.extend(fresh.iter().map(|(k, v)| (k.clone(), v.clone())));
        fresh
    }
}

fn fetch(agent: &Agent, url: &str) -> Result<RgbaImage> {
    let bytes = agent
        .get(url)
        .call()
        .context("request failed")?
        .body_mut()
        .read_to_vec()
        .context("reading body")?;
    decode(&bytes)
}

/// Decode any supported image, resize to [`AVATAR_PX`] and apply the mask.
pub fn decode(bytes: &[u8]) -> Result<RgbaImage> {
    let decoded = image::load_from_memory(bytes).context("not a supported image")?;
    let resized = imageops::resize(&decoded, AVATAR_PX, AVATAR_PX, FilterType::Triangle);
    Ok(circle_mask(resized))
}

/// Grey circle used when an avatar is missing or failed to load.
pub fn placeholder() -> RgbaImage {
    let mut img = RgbaImage::new(AVATAR_PX, AVATAR_PX);
    let centre = AVATAR_PX as f32 / 2.0;
    fill_circle(&mut img, centre, centre, centre, GREY);
    img
}

/// Keep only the pixels inside the inscribed circle, with a soft edge.
pub fn circle_mask(mut img: RgbaImage) -> RgbaImage {
    let radius = img.width().min(img.height()) as f32 / 2.0;
    let (cx, cy) = (img.width() as f32 / 2.0, img.height() as f32 / 2.0);
    for (x, y, pixel) in img.enumerate_pixels_mut() {
        let dx = x as f32 + 0.5 - cx;
        let dy = y as f32 + 0.5 - cy;
        let coverage = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
        let alpha = (f32::from(pixel.0[3]) * coverage).round() as u8;
        *pixel = Rgba([pixel.0[0], pixel.0[1], pixel.0[2], alpha]);
    }
    img
}

/// Lay avatars out left to right on a transparent [`ICON_PX`]-tall canvas.
///
/// At most [`MAX_AVATARS`] are drawn. An empty input yields a 1 px wide
/// transparent image so callers never have to special-case it.
pub fn compose_strip(avatars: &[&RgbaImage]) -> RgbaImage {
    let shown = &avatars[..avatars.len().min(MAX_AVATARS)];
    if shown.is_empty() {
        return RgbaImage::new(1, ICON_PX);
    }
    let n = shown.len() as u32;
    let width = n * AVATAR_PX + (n - 1) * GAP_PX;
    let mut strip = RgbaImage::new(width, ICON_PX);
    let y = i64::from((ICON_PX - AVATAR_PX) / 2);
    for (i, avatar) in shown.iter().enumerate() {
        let x = i64::from(i as u32 * (AVATAR_PX + GAP_PX));
        imageops::overlay(&mut strip, *avatar, x, y);
    }
    strip
}
```

Add `#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]` at the top of the file with the same comment as in `icons.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test avatars`
Expected: `test result: ok. 7 passed`

- [ ] **Step 5: Lint and commit**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings`

```bash
git add src/avatars.rs src/main.rs
git commit -m "feat(avatars): cache, circular mask and avatar strips"
```

---

### Task 6: Menu construction

**Files:**
- Create: `src/menu.rs`
- Modify: `src/main.rs` (add `mod menu;`)

**Interfaces:**
- Consumes: `model::{PullRequest, Reviewer, ReviewState, Snapshot, Status}`, `icons::{menu_icon, status_dot}`, `avatars::{compose_strip, placeholder, MAX_AVATARS}`.
- Produces: `menu::Action { OpenUrl(String), Refresh }`, `menu::BuiltMenu { menu: Menu, actions: HashMap<MenuId, Action> }`, `menu::build_menu(&Snapshot, &HashMap<String, RgbaImage>) -> anyhow::Result<BuiltMenu>`, `menu::tray_title(&Snapshot) -> Option<String>`, `menu::pr_label(&PullRequest) -> String`, `menu::reviewers_label(&[Reviewer]) -> String`, `menu::REFRESH_ID: &str`.

- [ ] **Step 1: Write the failing tests**

Create `src/menu.rs`:

```rust
//! Build the status item menu from a [`Snapshot`].
//!
//! `build_menu` touches AppKit and must run on the main thread. The label
//! helpers are pure and tested here.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CiState, ReviewDecision};

    fn pr(title: &str) -> PullRequest {
        PullRequest {
            repo: "acme/widgets".into(),
            number: 42,
            title: title.into(),
            url: "https://github.com/acme/widgets/pull/42".into(),
            is_draft: false,
            ci: CiState::Success,
            decision: ReviewDecision::Approved,
            reviewers: Vec::new(),
        }
    }

    fn reviewer(login: &str, state: ReviewState) -> Reviewer {
        Reviewer { login: login.into(), avatar_url: String::new(), state }
    }

    #[test]
    fn pr_label_has_repo_number_and_title() {
        assert_eq!(pr_label(&pr("Fix it")), "acme/widgets #42   Fix it");
    }

    #[test]
    fn pr_label_truncates_long_titles_on_char_boundaries() {
        let long = "ä".repeat(80);
        let label = pr_label(&pr(&long));
        let title = label.split("   ").nth(1).unwrap();
        assert_eq!(title.chars().count(), 60);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn reviewers_label_uses_state_symbols() {
        let label = reviewers_label(&[
            reviewer("alice", ReviewState::Approved),
            reviewer("bob", ReviewState::ChangesRequested),
            reviewer("carol", ReviewState::Commented),
            reviewer("dave", ReviewState::Pending),
        ]);
        assert_eq!(label, "alice ✓  bob ✗  carol 💬  dave ⏳");
    }

    #[test]
    fn reviewers_label_handles_none_and_overflow() {
        assert_eq!(reviewers_label(&[]), "no reviewers requested");
        let seven: Vec<Reviewer> = (0..7)
            .map(|i| reviewer(&format!("u{i}"), ReviewState::Pending))
            .collect();
        let label = reviewers_label(&seven);
        assert!(label.starts_with("u0 ⏳  u1 ⏳  u2 ⏳  u3 ⏳  u4 ⏳"));
        assert!(label.ends_with("  +2"));
    }

    #[test]
    fn tray_title_reflects_state() {
        let mut snap = Snapshot::default();
        assert_eq!(tray_title(&snap).as_deref(), Some("…"), "first load in flight");

        snap.updated_at = Some("12:00".into());
        assert_eq!(tray_title(&snap), None, "zero PRs shows no count");

        snap.prs = vec![pr("a"), pr("b")];
        assert_eq!(tray_title(&snap).as_deref(), Some("2"));

        snap.error = Some("boom".into());
        assert_eq!(tray_title(&snap).as_deref(), Some("!"));
    }
}
```

Add `mod menu;` to `src/main.rs`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test menu 2>&1 | head`
Expected: compile errors.

- [ ] **Step 3: Write the implementation**

Insert above the test module:

```rust
use std::collections::HashMap;

use anyhow::Result;
use image::RgbaImage;
use tray_icon::menu::accelerator::Accelerator;
use tray_icon::menu::{IconMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem};

use crate::avatars::{MAX_AVATARS, compose_strip, placeholder};
use crate::icons::{menu_icon, status_dot};
use crate::model::{PullRequest, ReviewState, Reviewer, Snapshot, Status};

/// Menu id of the "Refresh" item.
pub const REFRESH_ID: &str = "refresh";
const TITLE_MAX_CHARS: usize = 60;

/// What to do when a menu item is activated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    OpenUrl(String),
    Refresh,
}

/// A menu plus the lookup table from item id to action.
/// (`Menu` does not implement `Debug`, so neither does this.)
pub struct BuiltMenu {
    pub menu: Menu,
    pub actions: HashMap<MenuId, Action>,
}

/// Build the whole menu. Main thread only.
pub fn build_menu(snapshot: &Snapshot, avatars: &HashMap<String, RgbaImage>) -> Result<BuiltMenu> {
    let menu = Menu::new();
    let mut actions = HashMap::new();
    let fallback = placeholder();

    if let Some(error) = &snapshot.error {
        menu.append(&MenuItem::new(format!("⚠ {error}"), false, None))?;
        menu.append(&PredefinedMenuItem::separator())?;
    }

    if snapshot.prs.is_empty() && snapshot.updated_at.is_some() {
        menu.append(&MenuItem::new("No open pull requests 🎉", false, None))?;
    }

    for (index, pr) in snapshot.prs.iter().enumerate() {
        let id = MenuId::new(format!("pr-{index}"));
        let dot = menu_icon(&status_dot(Status::derive(pr)));
        menu.append(&IconMenuItem::with_id(id.clone(), pr_label(pr), true, Some(dot), None))?;
        actions.insert(id, Action::OpenUrl(pr.url.clone()));

        let images: Vec<&RgbaImage> = pr
            .reviewers
            .iter()
            .map(|r| avatars.get(&r.avatar_url).unwrap_or(&fallback))
            .collect();
        let strip = (!images.is_empty()).then(|| menu_icon(&compose_strip(&images)));
        menu.append(&IconMenuItem::new(reviewers_label(&pr.reviewers), false, strip, None))?;
    }

    if !snapshot.prs.is_empty() || snapshot.updated_at.is_some() {
        menu.append(&PredefinedMenuItem::separator())?;
    }

    let refresh_accel: Accelerator = "CmdOrCtrl+R".parse()?;
    menu.append(&MenuItem::with_id(REFRESH_ID, "Refresh", true, Some(refresh_accel)))?;
    actions.insert(MenuId::new(REFRESH_ID), Action::Refresh);

    let updated = snapshot
        .updated_at
        .as_ref()
        .map_or_else(|| "Loading…".to_owned(), |t| format!("Updated {t}"));
    menu.append(&MenuItem::new(updated, false, None))?;

    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&PredefinedMenuItem::quit(Some("Quit prtoolbar")))?;

    Ok(BuiltMenu { menu, actions })
}

/// Text next to the menu bar icon.
pub fn tray_title(snapshot: &Snapshot) -> Option<String> {
    if snapshot.error.is_some() {
        return Some("!".to_owned());
    }
    if snapshot.updated_at.is_none() {
        return Some("…".to_owned());
    }
    match snapshot.prs.len() {
        0 => None,
        n => Some(n.to_string()),
    }
}

/// `owner/repo #123   Title`, title capped at 60 characters.
pub fn pr_label(pr: &PullRequest) -> String {
    format!("{} #{}   {}", pr.repo, pr.number, truncate(&pr.title, TITLE_MAX_CHARS))
}

/// `alice ✓  bob ⏳`, or `no reviewers requested`, with `+N` past five.
pub fn reviewers_label(reviewers: &[Reviewer]) -> String {
    if reviewers.is_empty() {
        return "no reviewers requested".to_owned();
    }
    let mut parts: Vec<String> = reviewers
        .iter()
        .take(MAX_AVATARS)
        .map(|r| format!("{} {}", r.login, symbol(r.state)))
        .collect();
    if reviewers.len() > MAX_AVATARS {
        parts.push(format!("+{}", reviewers.len() - MAX_AVATARS));
    }
    parts.join("  ")
}

fn symbol(state: ReviewState) -> &'static str {
    match state {
        ReviewState::Approved => "✓",
        ReviewState::ChangesRequested => "✗",
        ReviewState::Commented => "💬",
        ReviewState::Pending => "⏳",
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max_chars - 1).collect();
    out.push('…');
    out
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test menu`
Expected: `test result: ok. 5 passed`

- [ ] **Step 5: Lint and commit**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings`

```bash
git add src/menu.rs src/main.rs
git commit -m "feat(menu): build status item menu from snapshot"
```

---

### Task 7: Events and worker thread

**Files:**
- Create: `src/events.rs`
- Create: `src/worker.rs`
- Modify: `src/main.rs` (add `mod events; mod worker;`)

**Interfaces:**
- Consumes: `auth::resolve_token`, `github::fetch_open_prs`, `avatars::AvatarCache`, `model::PullRequest`.
- Produces: `events::AppEvent { Menu(MenuEvent), Loaded(Vec<PullRequest>), Failed(String), Avatars(HashMap<String, RgbaImage>) }`, `worker::Command::Refresh`, `worker::spawn(EventLoopProxy<AppEvent>, Duration) -> mpsc::Sender<Command>`, `worker::interval_from_env(Option<&str>) -> Duration`, `worker::http_agent(Duration) -> ureq::Agent`.

- [ ] **Step 1: Create `src/events.rs`**

```rust
//! Messages delivered to the main thread's event loop.

use std::collections::HashMap;

use image::RgbaImage;
use tray_icon::menu::MenuEvent;

use crate::model::PullRequest;

/// Everything that can wake the main thread.
#[derive(Debug)]
pub enum AppEvent {
    /// A menu item was activated.
    Menu(MenuEvent),
    /// A refresh succeeded.
    Loaded(Vec<PullRequest>),
    /// A refresh failed; the message is shown in the menu.
    Failed(String),
    /// Newly downloaded avatars keyed by URL.
    Avatars(HashMap<String, RgbaImage>),
}
```

- [ ] **Step 2: Write the failing tests for the worker**

Create `src/worker.rs`:

```rust
//! Background refresh: fetch PRs, then avatars, and hand results to the UI.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_defaults_to_sixty_seconds() {
        assert_eq!(interval_from_env(None), Duration::from_secs(60));
    }

    #[test]
    fn interval_is_parsed_and_clamped() {
        assert_eq!(interval_from_env(Some("120")), Duration::from_secs(120));
        assert_eq!(interval_from_env(Some("3")), Duration::from_secs(15));
        assert_eq!(interval_from_env(Some("abc")), Duration::from_secs(60));
        assert_eq!(interval_from_env(Some("")), Duration::from_secs(60));
    }
}
```

Add `mod events;` and `mod worker;` to `src/main.rs`.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test worker 2>&1 | head`
Expected: compile error, `interval_from_env` not found.

- [ ] **Step 4: Write the implementation**

Insert above the test module in `src/worker.rs`:

```rust
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use tao::event_loop::EventLoopProxy;
use ureq::Agent;

use crate::avatars::AvatarCache;
use crate::events::AppEvent;
use crate::{auth, github};

const DEFAULT_INTERVAL: Duration = Duration::from_secs(60);
const MIN_INTERVAL: Duration = Duration::from_secs(15);
const GITHUB_TIMEOUT: Duration = Duration::from_secs(10);
const AVATAR_TIMEOUT: Duration = Duration::from_secs(5);

/// Requests the UI can send to the worker.
#[derive(Debug)]
pub enum Command {
    Refresh,
}

/// Start the refresh thread. Dropping the returned sender stops it after the
/// current cycle.
pub fn spawn(proxy: EventLoopProxy<AppEvent>, interval: Duration) -> Sender<Command> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("refresh".into())
        .spawn(move || run(&proxy, &rx, interval))
        .expect("spawn refresh thread");
    tx
}

/// Parse `PRTOOLBAR_INTERVAL_SECS`; invalid or missing values give the default.
pub fn interval_from_env(value: Option<&str>) -> Duration {
    value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map_or(DEFAULT_INTERVAL, |secs| Duration::from_secs(secs).max(MIN_INTERVAL))
}

/// An agent with a global timeout and our user agent. Non-2xx responses are
/// returned as responses, not errors, so callers can map status codes.
pub fn http_agent(timeout: Duration) -> Agent {
    Agent::config_builder()
        .timeout_global(Some(timeout))
        .user_agent(concat!("prtoolbar/", env!("CARGO_PKG_VERSION")))
        .http_status_as_error(false)
        .build()
        .new_agent()
}

fn run(proxy: &EventLoopProxy<AppEvent>, rx: &Receiver<Command>, interval: Duration) {
    let github = http_agent(GITHUB_TIMEOUT);
    let cdn = http_agent(AVATAR_TIMEOUT);
    let mut cache = AvatarCache::default();

    loop {
        let result = auth::resolve_token().and_then(|token| github::fetch_open_prs(&github, &token));
        match result {
            Ok(prs) => {
                let urls: Vec<String> = prs
                    .iter()
                    .flat_map(|pr| pr.reviewers.iter().map(|r| r.avatar_url.clone()))
                    .collect();
                if proxy.send_event(AppEvent::Loaded(prs)).is_err() {
                    return;
                }
                let fresh = cache.fetch_missing(&cdn, urls);
                if !fresh.is_empty() && proxy.send_event(AppEvent::Avatars(fresh)).is_err() {
                    return;
                }
            }
            Err(err) => {
                log::warn!("refresh failed: {err:#}");
                if proxy.send_event(AppEvent::Failed(format!("{err:#}"))).is_err() {
                    return;
                }
            }
        }

        match rx.recv_timeout(interval) {
            Ok(Command::Refresh) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test worker`
Expected: `test result: ok. 2 passed`

- [ ] **Step 6: Lint and commit**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings`

```bash
git add src/events.rs src/worker.rs src/main.rs
git commit -m "feat(worker): background refresh thread and app events"
```

---

### Task 8: Wire up `main.rs`

**Files:**
- Modify: `src/main.rs` (full rewrite)

**Interfaces:**
- Consumes: everything above.

- [ ] **Step 1: Rewrite `src/main.rs`**

```rust
//! prtoolbar — a macOS menu bar app that lists your open GitHub pull requests.
//!
//! The main thread owns the event loop, the status item and its menu. A
//! worker thread (see `worker.rs`) fetches data and posts [`AppEvent`]s here.

mod auth;
mod avatars;
mod events;
mod github;
mod icons;
mod menu;
mod model;
mod worker;

use std::collections::HashMap;
use std::sync::mpsc::Sender;

use anyhow::Result;
use image::RgbaImage;
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::menu::{MenuEvent, MenuId};
use tray_icon::{TrayIcon, TrayIconBuilder};

use crate::events::AppEvent;
use crate::menu::Action;
use crate::model::Snapshot;
use crate::worker::Command;

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let mut event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(AppEvent::Menu(event));
    }));

    let interval = worker::interval_from_env(std::env::var("PRTOOLBAR_INTERVAL_SECS").ok().as_deref());
    let worker = worker::spawn(event_loop.create_proxy(), interval);

    let tray = TrayIconBuilder::new()
        .with_icon(icons::tray_icon(&icons::menubar_glyph()))
        .with_icon_as_template(true)
        .with_tooltip("prtoolbar")
        .build()?;

    let mut app = App {
        tray,
        worker,
        snapshot: Snapshot::default(),
        avatars: HashMap::new(),
        actions: HashMap::new(),
    };
    app.rebuild();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        if let Event::UserEvent(event) = event {
            app.handle(event);
        }
    });
}

/// All main-thread state.
struct App {
    tray: TrayIcon,
    worker: Sender<Command>,
    snapshot: Snapshot,
    avatars: HashMap<String, RgbaImage>,
    actions: HashMap<MenuId, Action>,
}

impl App {
    fn handle(&mut self, event: AppEvent) {
        match event {
            AppEvent::Loaded(prs) => {
                self.snapshot.prs = prs;
                self.snapshot.error = None;
                self.snapshot.updated_at = Some(now_hhmm());
                self.rebuild();
            }
            AppEvent::Failed(message) => {
                self.snapshot.error = Some(message);
                self.rebuild();
            }
            AppEvent::Avatars(fresh) => {
                self.avatars.extend(fresh);
                self.rebuild();
            }
            AppEvent::Menu(menu_event) => self.activate(&menu_event.id),
        }
    }

    fn activate(&self, id: &MenuId) {
        match self.actions.get(id) {
            Some(Action::OpenUrl(url)) => {
                if let Err(err) = open::that_detached(url) {
                    log::warn!("open {url}: {err}");
                }
            }
            Some(Action::Refresh) => {
                let _ = self.worker.send(Command::Refresh);
            }
            None => log::debug!("unhandled menu id {id:?}"),
        }
    }

    /// Rebuild the menu and title from the current snapshot.
    fn rebuild(&mut self) {
        match menu::build_menu(&self.snapshot, &self.avatars) {
            Ok(built) => {
                self.tray.set_menu(Some(Box::new(built.menu)));
                self.actions = built.actions;
            }
            Err(err) => log::error!("building menu: {err:#}"),
        }
        self.tray.set_title(menu::tray_title(&self.snapshot));
    }
}

/// Local wall-clock time as `HH:MM`.
fn now_hhmm() -> String {
    jiff::Zoned::now().strftime("%H:%M").to_string()
}
```

Remove the `#![allow(dead_code)]` added in Task 1. If clippy still reports dead code, the item is genuinely unused and should be deleted rather than allowed.

- [ ] **Step 2: Build and lint**

Run: `cargo fmt --all && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: clean, all 34 tests pass (9 model, 4 github, 3 auth, 4 icons, 7 avatars, 5 menu, 2 worker).

- [ ] **Step 3: Manual smoke test**

Run: `RUST_LOG=prtoolbar=debug cargo run`

Check, in this order:
1. A pull-request glyph appears in the menu bar followed by `…`, then within a few seconds by a count (or nothing if you have no open PRs).
2. Clicking it opens a menu: coloured dots, one line per PR, a grey reviewer line under each with avatars once the second event lands.
3. Clicking a PR opens it in the default browser.
4. `Refresh` re-fetches (watch the debug log).
5. `Quit prtoolbar` exits the process.
6. Run once with `GITHUB_TOKEN=bad cargo run`: the title shows `!` and the menu's first line is `⚠ GitHub rejected the token (401)`.

If step 1 shows no icon at all, the most likely cause is the tray being created before the event loop started; move the `TrayIconBuilder` call (and `App` construction) into `Event::NewEvents(StartCause::Init)` inside the run closure, storing `app` in an `Option`.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire event loop, worker and menu together"
```

---

### Task 9: Finish

**Files:**
- Modify: `README.md` only if behaviour deviated from it during Task 8.

- [ ] **Step 1: Run the full CI set locally**

Run: `cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-features && cargo build --release`
Expected: all green.

- [ ] **Step 2: Optional bundle check**

Run: `cargo install cargo-bundle && cargo bundle --release && open target/release/bundle/osx/prtoolbar.app`
Expected: app launches with no Dock icon and finds the token via `/opt/homebrew/bin/gh`.

- [ ] **Step 3: Commit any README corrections and push**

```bash
git add README.md
git commit -m "docs: align README with shipped behaviour"
git push -u origin HEAD
```

Then open a PR against `main` using `superpowers:finishing-a-development-branch`.

---

## Self-review notes

- Spec coverage: model rules (T1), GraphQL + fixtures (T2), auth fallbacks incl. bundle `PATH` (T3), 36 px icons and system colours (T4), avatar strip / cap / placeholder (T5), menu layout, error line, empty state, `…`/`!`/count title (T6), two-event avatar flow, interval env, timeouts (T7), Accessory policy, open URL, `Updated HH:MM` (T8), CI/bundle (T0, T9).
- Deliberate deviations from the spec: none. The spec was amended in the same commit series to match the status rule and pixel sizes used here.
- Type names are consistent across tasks: `AppEvent::{Menu, Loaded, Failed, Avatars}`, `Command::Refresh`, `Action::{OpenUrl, Refresh}`, `BuiltMenu { menu, actions }`, `Snapshot { prs, updated_at, error }`.
