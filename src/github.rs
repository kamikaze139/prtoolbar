//! GitHub GraphQL client: one query, its DTOs, and the mapping into `model`.

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;
use ureq::Agent;

use crate::model::{
    CiState, PullRequest, ReviewDecision, ReviewState, Reviewer, StackMembership, merge_reviewers,
};

const ENDPOINT: &str = "https://api.github.com/graphql";

/// Open PRs authored by the token owner, newest activity first.
const QUERY: &str = r#"
query {
  search(query: "is:pr is:open author:@me archived:false sort:updated-desc", type: ISSUE, first: 50) {
    issueCount
    nodes {
      ... on PullRequest {
        number title url isDraft
        repository { nameWithOwner }
        reviewDecision
        stackEntry { position stack { number size } }
        commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
        reviewRequests(first: 10) { nodes { requestedReviewer {
          ... on User { login avatarUrl(size: 64) }
          ... on Team { name avatarUrl(size: 64) } } } }
        latestReviews(first: 10) { nodes { state author { login avatarUrl(size: 64) } } }
      }
    }
  }
}
"#;

/// Fetch the user's open pull requests.
///
/// The agent must be configured with `http_status_as_error(false)` so that
/// non-2xx responses reach the status mapping below.
pub fn fetch_open_prs(agent: &Agent, token: &str) -> Result<(Vec<PullRequest>, u64)> {
    let mut response = agent
        .post(ENDPOINT)
        .header("Authorization", format!("bearer {token}"))
        .send_json(serde_json::json!({ "query": QUERY }))
        .context("GitHub request failed")?;

    if let Some(err) = status_error(response.status().as_u16()) {
        return Err(err);
    }

    let body = response
        .body_mut()
        .read_to_string()
        .context("reading GitHub response")?;
    parse_response(&body)
}

/// Map an HTTP status code to the error shown in the menu, or `None` for 200.
fn status_error(status: u16) -> Option<anyhow::Error> {
    match status {
        200 => None,
        401 => Some(anyhow!("GitHub rejected the token (401)")),
        403 | 429 => Some(anyhow!("GitHub rate limited (HTTP {status})")),
        other => Some(anyhow!("GitHub HTTP {other}")),
    }
}

/// Turn a raw GraphQL response body into pull requests and the total count.
pub fn parse_response(body: &str) -> Result<(Vec<PullRequest>, u64)> {
    let parsed: GraphQlResponse =
        serde_json::from_str(body).context("GitHub returned unexpected JSON")?;

    if let Some(errors) = parsed.errors.filter(|errors| !errors.is_empty()) {
        let messages: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
        bail!("GitHub: {}", messages.join("; "));
    }

    let data = parsed
        .data
        .ok_or_else(|| anyhow!("GitHub returned no data"))?;
    let total = data.search.issue_count;
    let prs = data
        .search
        .nodes
        .into_iter()
        .filter_map(|node| match serde_json::from_value::<PrNode>(node) {
            Ok(node) => Some(PullRequest::from(node)),
            Err(err) => {
                log::warn!("skipping search node: {err}");
                None
            }
        })
        .collect();
    Ok((prs, total))
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
    search: SearchResult,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchResult {
    #[serde(default)]
    issue_count: u64,
    nodes: Vec<serde_json::Value>,
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
    stack_entry: Option<StackEntry>,
}

#[derive(Debug, Deserialize)]
struct StackEntry {
    position: u64,
    stack: Option<Stack>,
}

#[derive(Debug, Deserialize)]
struct Stack {
    number: u64,
    size: u64,
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
            stack: node.stack_entry.and_then(|entry| {
                entry.stack.map(|stack| StackMembership {
                    number: stack.number,
                    position: entry.position,
                    size: stack.size,
                })
            }),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CiState, ReviewDecision, ReviewState};

    const FIXTURE: &str = include_str!("../tests/fixtures/search_response.json");

    #[test]
    fn stack_metadata_preserves_the_full_stack_size() {
        let mut response: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        response["data"]["search"]["nodes"][0]["stackEntry"] = serde_json::json!({
            "position": 4, "stack": { "number": 7, "size": 20 }
        });
        let (prs, _) = parse_response(&response.to_string()).unwrap();
        assert_eq!(
            prs[0].stack,
            Some(StackMembership {
                number: 7,
                position: 4,
                size: 20
            })
        );
        assert_eq!(prs[1].stack, None);

        response["data"]["search"]["nodes"][0]["stackEntry"] = serde_json::json!({
            "position": 4, "stack": null
        });
        let (prs, _) = parse_response(&response.to_string()).unwrap();
        assert_eq!(prs[0].stack, None);
    }

    #[test]
    fn parses_fixture_into_pull_requests() {
        let (prs, total) = parse_response(FIXTURE).expect("fixture parses");
        assert_eq!(prs.len(), 2);
        assert_eq!(total, 2);

        let first = &prs[0];
        assert_eq!(first.repo, "acme/widgets");
        assert_eq!(first.number, 42);
        assert_eq!(first.url, "https://github.com/acme/widgets/pull/42");
        assert!(!first.is_draft);
        assert_eq!(first.ci, CiState::Success);
        assert_eq!(first.decision, ReviewDecision::Approved);

        let reviewers: Vec<(&str, ReviewState)> = first
            .reviewers
            .iter()
            .map(|r| (r.login.as_str(), r.state))
            .collect();
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
    fn a_malformed_node_is_skipped_not_fatal() {
        let (prs, _) = parse_response(r#"{"data":{"search":{"nodes":[{}]}}}"#)
            .expect("malformed node is skipped, not an error");
        assert_eq!(prs, vec![]);
    }

    #[test]
    fn no_nodes_is_an_empty_list() {
        let (prs, _) = parse_response(r#"{"data":{"search":{"nodes":[]}}}"#).unwrap();
        assert_eq!(prs, vec![]);
    }

    #[test]
    fn ci_state_mapping() {
        assert_eq!(ci_state("SUCCESS"), CiState::Success);
        assert_eq!(ci_state("FAILURE"), CiState::Failure);
        assert_eq!(ci_state("ERROR"), CiState::Failure);
        assert_eq!(ci_state("PENDING"), CiState::Pending);
        assert_eq!(ci_state("EXPECTED"), CiState::Pending);
    }

    #[test]
    fn review_decision_mapping() {
        assert_eq!(review_decision(Some("APPROVED")), ReviewDecision::Approved);
        assert_eq!(
            review_decision(Some("CHANGES_REQUESTED")),
            ReviewDecision::ChangesRequested
        );
        assert_eq!(
            review_decision(Some("REVIEW_REQUIRED")),
            ReviewDecision::Required
        );
        assert_eq!(review_decision(None), ReviewDecision::None);
        assert_eq!(
            review_decision(Some("SOMETHING_NEW")),
            ReviewDecision::None,
            "an unknown decision must not block the PR"
        );
    }

    #[test]
    fn only_submitted_reviews_become_reviewer_states() {
        assert_eq!(review_state("APPROVED"), Some(ReviewState::Approved));
        assert_eq!(
            review_state("CHANGES_REQUESTED"),
            Some(ReviewState::ChangesRequested)
        );
        assert_eq!(review_state("COMMENTED"), Some(ReviewState::Commented));
        assert_eq!(review_state("DISMISSED"), None, "dismissed is not shown");
        assert_eq!(review_state("PENDING"), None, "unsubmitted is not shown");
    }

    #[test]
    fn the_total_is_the_search_count_not_the_node_count() {
        let body = r#"{"data":{"search":{"issueCount":137,"nodes":[]}}}"#;
        let (prs, total) = parse_response(body).unwrap();
        assert!(prs.is_empty());
        assert_eq!(total, 137, "the count past the 50-PR cap stays truthful");
    }

    #[test]
    fn status_error_mapping() {
        assert!(status_error(200).is_none());
        assert_eq!(
            status_error(401).unwrap().to_string(),
            "GitHub rejected the token (401)"
        );
        assert_eq!(
            status_error(403).unwrap().to_string(),
            "GitHub rate limited (HTTP 403)"
        );
        assert_eq!(
            status_error(429).unwrap().to_string(),
            "GitHub rate limited (HTTP 429)"
        );
        assert_eq!(status_error(500).unwrap().to_string(), "GitHub HTTP 500");
    }
}
