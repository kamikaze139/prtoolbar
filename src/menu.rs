//! Build the status item menu from a [`Snapshot`].
//!
//! `build_menu` touches `AppKit` and must run on the main thread. The label
//! helpers are pure and tested here.

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use image::{RgbaImage, imageops};
use tray_icon::menu::accelerator::Accelerator;
use tray_icon::menu::{IconMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem};

use crate::avatars::{MAX_AVATARS, compose_strip, placeholder};
use crate::icons::{ICON_PX, menu_icon, review_badge, status_dot};
use crate::model::{PullRequest, ReviewDecision, ReviewState, Reviewer, Snapshot, Status};

/// Menu id of the "Refresh" item.
pub const REFRESH_ID: &str = "refresh";
const TITLE_MAX_CHARS: usize = 44;

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

    for (repo, prs) in group_by_repo(&snapshot.prs) {
        menu.append(&MenuItem::new(
            format!("{repo} ({})", prs.len()),
            false,
            None,
        ))?;
        for pr in prs {
            let id = MenuId::new(&pr.url);
            menu.append(&IconMenuItem::with_id(
                id.clone(),
                pr_label(pr),
                true,
                Some(menu_icon(&pr_icon(pr, avatars, &fallback))),
                None,
            ))?;
            actions.insert(id, Action::OpenUrl(pr.url.clone()));
        }
    }

    if snapshot.total > snapshot.prs.len() {
        menu.append(&MenuItem::new(
            format!("Showing {} of {}", snapshot.prs.len(), snapshot.total),
            false,
            None,
        ))?;
    }

    if !snapshot.prs.is_empty() || snapshot.updated_at.is_some() {
        menu.append(&PredefinedMenuItem::separator())?;
    }

    let refresh_accel: Accelerator = "CmdOrCtrl+R".parse()?;
    menu.append(&MenuItem::with_id(
        REFRESH_ID,
        "Refresh",
        true,
        Some(refresh_accel),
    ))?;
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
        n if snapshot.total > n => Some(format!("{n}+")),
        n => Some(n.to_string()),
    }
}

/// Alphabetical repository sections, retaining newest-activity order within each.
fn group_by_repo(prs: &[PullRequest]) -> BTreeMap<&str, Vec<&PullRequest>> {
    let mut groups: BTreeMap<&str, Vec<&PullRequest>> = BTreeMap::new();
    for pr in prs {
        groups.entry(&pr.repo).or_default().push(pr);
    }
    groups
}

/// One image keeps the overall status dot and badged avatars on the PR row.
fn pr_icon(
    pr: &PullRequest,
    avatars: &HashMap<String, RgbaImage>,
    fallback: &RgbaImage,
) -> RgbaImage {
    let dot = status_dot(Status::derive(pr));
    if pr.reviewers.is_empty() {
        return dot;
    }
    let images: Vec<RgbaImage> = pr
        .reviewers
        .iter()
        .take(MAX_AVATARS)
        .map(|reviewer| {
            let mut avatar = avatars
                .get(&reviewer.avatar_url)
                .unwrap_or(fallback)
                .clone();
            let badge = review_badge(reviewer.state);
            let x = i64::from(avatar.width()) - i64::from(badge.width());
            let y = i64::from(avatar.height()) - i64::from(badge.height());
            imageops::overlay(&mut avatar, &badge, x, y);
            avatar
        })
        .collect();
    let strip = compose_strip(&images.iter().collect::<Vec<_>>());
    let mut icon = RgbaImage::new(ICON_PX + strip.width(), ICON_PX);
    imageops::overlay(&mut icon, &dot, 0, 0);
    imageops::overlay(&mut icon, &strip, i64::from(ICON_PX), 0);
    icon
}

/// PR number, aggregate review decision, short title and individual reviewers.
pub fn pr_label(pr: &PullRequest) -> String {
    let decision = match pr.decision {
        ReviewDecision::Approved => " ✓",
        ReviewDecision::ChangesRequested => " ✗",
        ReviewDecision::Required => " ⏳",
        ReviewDecision::None => "",
    };
    let mut label = format!(
        "#{}{decision}   {}",
        pr.number,
        truncate(&pr.title, TITLE_MAX_CHARS)
    );
    if !pr.reviewers.is_empty() {
        label.push_str("  ·  ");
        label.push_str(&reviewers_label(&pr.reviewers));
    }
    label
}

/// `alice ✓  bob ⏳`, or empty when nobody is assigned, with `+N` past five.
pub fn reviewers_label(reviewers: &[Reviewer]) -> String {
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

pub(crate) fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max_chars - 1).collect();
    out.push('…');
    out
}

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
        Reviewer {
            login: login.into(),
            avatar_url: String::new(),
            state,
        }
    }

    #[test]
    fn pr_label_omits_repo_and_empty_review_summary() {
        assert_eq!(pr_label(&pr("Fix it")), "#42 ✓   Fix it");
    }

    #[test]
    fn pr_label_truncates_long_titles_on_char_boundaries() {
        let long = "ä".repeat(80);
        let label = pr_label(&pr(&long));
        let title = label.split("   ").nth(1).unwrap();
        assert_eq!(title.chars().count(), 44);
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
        assert_eq!(reviewers_label(&[]), "");
        let seven: Vec<Reviewer> = (0..7)
            .map(|i| reviewer(&format!("u{i}"), ReviewState::Pending))
            .collect();
        let label = reviewers_label(&seven);
        assert_eq!(label, "u0 ⏳  u1 ⏳  u2 ⏳  u3 ⏳  u4 ⏳  +2");
    }

    #[test]
    fn pr_label_keeps_reviewer_names_and_states_inline() {
        let mut pull_request = pr("Fix it");
        pull_request.reviewers = vec![
            reviewer("alice", ReviewState::Approved),
            reviewer("bob", ReviewState::Approved),
            reviewer("carol", ReviewState::Pending),
        ];
        assert_eq!(
            pr_label(&pull_request),
            "#42 ✓   Fix it  ·  alice ✓  bob ✓  carol ⏳"
        );
    }

    #[test]
    fn pr_label_shows_review_decision_independently_of_ci() {
        let mut pull_request = pr("Fix it");
        pull_request.ci = CiState::Failure;
        for (decision, expected) in [
            (ReviewDecision::Approved, "#42 ✓   Fix it"),
            (ReviewDecision::ChangesRequested, "#42 ✗   Fix it"),
            (ReviewDecision::Required, "#42 ⏳   Fix it"),
            (ReviewDecision::None, "#42   Fix it"),
        ] {
            pull_request.decision = decision;
            assert_eq!(pr_label(&pull_request), expected);
        }
    }

    #[test]
    fn groups_interleaved_repos_without_losing_prs_or_changing_recency_order() {
        let mut first = pr("Newest widget");
        first.repo = "zebra/widgets".into();
        let second = pr("Other owner's widget");
        let mut third = pr("Older widget");
        third.repo = "zebra/widgets".into();
        let prs = vec![first, second, third];
        let groups = group_by_repo(&prs);
        assert_eq!(
            groups.keys().copied().collect::<Vec<_>>(),
            ["acme/widgets", "zebra/widgets"]
        );
        assert_eq!(groups["acme/widgets"], [&prs[1]]);
        assert_eq!(groups["zebra/widgets"], [&prs[0], &prs[2]]);
        assert!(group_by_repo(&[]).is_empty());
    }

    #[test]
    fn tray_title_reflects_state() {
        let mut snap = Snapshot::default();
        assert_eq!(
            tray_title(&snap).as_deref(),
            Some("…"),
            "first load in flight"
        );

        snap.updated_at = Some("12:00".into());
        assert_eq!(tray_title(&snap), None, "zero PRs shows no count");

        snap.prs = vec![pr("a"), pr("b")];
        assert_eq!(tray_title(&snap).as_deref(), Some("2"));

        snap.total = 7;
        assert_eq!(
            tray_title(&snap).as_deref(),
            Some("2+"),
            "total past the cap shows a plus"
        );
        snap.total = snap.prs.len();

        snap.error = Some("boom".into());
        assert_eq!(tray_title(&snap).as_deref(), Some("!"));
    }
}
