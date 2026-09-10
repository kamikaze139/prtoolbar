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
    /// Total open PRs matched by the search, which may exceed `prs.len()`
    /// past the 50-PR menu cap.
    pub total: usize,
    /// Local wall-clock time of the last successful fetch, `HH:MM`.
    pub updated_at: Option<String>,
    /// Message of the most recent failed fetch, cleared on success.
    pub error: Option<String>,
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
