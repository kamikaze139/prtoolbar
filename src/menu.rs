//! Build the status item menu from a [`Snapshot`].
//!
//! `build_menu` touches `AppKit` and must run on the main thread. The label
//! helpers are pure and tested here.

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

    for pr in &snapshot.prs {
        let id = MenuId::new(&pr.url);
        let dot = menu_icon(&status_dot(Status::derive(pr)));
        menu.append(&IconMenuItem::with_id(
            id.clone(),
            pr_label(pr),
            true,
            Some(dot),
            None,
        ))?;
        actions.insert(id, Action::OpenUrl(pr.url.clone()));

        let images: Vec<&RgbaImage> = pr
            .reviewers
            .iter()
            .map(|r| avatars.get(&r.avatar_url).unwrap_or(&fallback))
            .collect();
        let strip = (!images.is_empty()).then(|| menu_icon(&compose_strip(&images)));
        menu.append(&IconMenuItem::new(
            reviewers_label(&pr.reviewers),
            false,
            strip,
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
        n => Some(n.to_string()),
    }
}

/// `owner/repo #123   Title`, title capped at 60 characters.
pub fn pr_label(pr: &PullRequest) -> String {
    format!(
        "{} #{}   {}",
        pr.repo,
        pr.number,
        truncate(&pr.title, TITLE_MAX_CHARS)
    )
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
        assert_eq!(
            tray_title(&snap).as_deref(),
            Some("…"),
            "first load in flight"
        );

        snap.updated_at = Some("12:00".into());
        assert_eq!(tray_title(&snap), None, "zero PRs shows no count");

        snap.prs = vec![pr("a"), pr("b")];
        assert_eq!(tray_title(&snap).as_deref(), Some("2"));

        snap.error = Some("boom".into());
        assert_eq!(tray_title(&snap).as_deref(), Some("!"));
    }
}
