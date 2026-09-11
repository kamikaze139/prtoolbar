//! Pure domain types and the rules that turn GitHub data into a status.
//! No I/O lives here, so everything is unit-testable.

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

/// Native GitHub stack membership. Position is one-based, starting at the base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackMembership {
    /// Stack number, unique within the repository.
    pub number: u64,
    pub position: u64,
    /// Full stack size, including PRs outside the currently fetched list.
    pub size: u64,
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
    pub stack: Option<StackMembership>,
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
/// Current requests come first and take precedence over earlier submitted
/// reviews: asking someone to review again makes their review pending.
pub fn merge_reviewers(requested: Vec<Reviewer>, reviewed: Vec<Reviewer>) -> Vec<Reviewer> {
    let mut out: Vec<Reviewer> = Vec::with_capacity(requested.len() + reviewed.len());
    for reviewer in requested.into_iter().chain(reviewed) {
        if !out.iter().any(|existing| existing.login == reviewer.login) {
            out.push(reviewer);
        }
    }
    out
}

/// Everything the main thread needs to render the menu.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub prs: Vec<PullRequest>,
    /// Total open PRs matched by the search, which may exceed `prs.len()`
    /// past the 50-PR menu cap.
    pub total: usize,
    /// Local wall-clock time of the last successful fetch, `HH:MM`.
    pub updated_at: Option<String>,
    /// Message of the most recent failed fetch, cleared on success.
    pub error: Option<String>,
}

impl Snapshot {
    /// Apply a successful fetch: the new list replaces the old one, and any
    /// error from an earlier cycle is cleared.
    pub fn loaded(&mut self, prs: Vec<PullRequest>, total: usize, at: String) {
        self.prs = prs;
        self.total = total;
        self.updated_at = Some(at);
        self.error = None;
    }

    /// Apply a failed fetch: show the message but keep the last good list, so
    /// a transient network blip does not empty the menu.
    pub fn failed(&mut self, message: String) {
        self.error = Some(message);
    }
}

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
            stack: None,
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
            Status::derive(&pr(
                false,
                CiState::Success,
                ReviewDecision::ChangesRequested
            )),
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
                ("bob", ReviewState::Pending),
                ("carol", ReviewState::Commented),
            ]
        );
    }

    #[test]
    fn a_successful_fetch_replaces_the_list_and_clears_the_error() {
        let mut snapshot = Snapshot {
            prs: vec![pr(false, CiState::Success, ReviewDecision::Approved)],
            total: 1,
            updated_at: Some("11:00".into()),
            error: Some("boom".into()),
        };

        snapshot.loaded(Vec::new(), 0, "12:00".into());

        assert!(
            snapshot.prs.is_empty(),
            "the new list wins, even when empty"
        );
        assert_eq!(snapshot.total, 0);
        assert_eq!(snapshot.updated_at.as_deref(), Some("12:00"));
        assert_eq!(snapshot.error, None, "a success clears the previous error");
    }

    #[test]
    fn a_failed_fetch_keeps_the_last_good_list_and_timestamp() {
        let mut snapshot = Snapshot::default();
        snapshot.loaded(
            vec![pr(false, CiState::Success, ReviewDecision::Approved)],
            9,
            "12:00".into(),
        );

        snapshot.failed("GitHub rejected the token (401)".into());

        assert_eq!(snapshot.prs.len(), 1, "the stale list stays on screen");
        assert_eq!(snapshot.total, 9);
        assert_eq!(snapshot.updated_at.as_deref(), Some("12:00"));
        assert_eq!(
            snapshot.error.as_deref(),
            Some("GitHub rejected the token (401)")
        );
    }

    #[test]
    fn rerequested_review_is_pending_despite_a_previous_review() {
        let requested = vec![reviewer("bob", ReviewState::Pending)];
        let reviewed = vec![reviewer("bob", ReviewState::ChangesRequested)];
        let merged = merge_reviewers(requested, reviewed);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].state, ReviewState::Pending);
    }
}
