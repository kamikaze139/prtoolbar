//! Text helpers for the tray title and error messages. The app has one popup.

use crate::model::Snapshot;

/// Hover text for the menu bar icon.
///
/// The menu bar shows the glyph alone — no count, no badge — so this is the
/// only place the numbers appear outside the popup.
pub fn tray_tooltip(snapshot: &Snapshot) -> String {
    if let Some(error) = &snapshot.error {
        return format!("Refresh failed: {error}");
    }
    let Some(updated) = &snapshot.updated_at else {
        return "Loading pull requests…".to_owned();
    };
    let shown = snapshot.prs.len();
    let count = match (shown, snapshot.total) {
        (0, _) => "No open pull requests".to_owned(),
        (1, 1) => "1 open pull request".to_owned(),
        // Past the display cap the list is shorter than the true total.
        (n, total) if total > n => format!("{n} of {total} open pull requests"),
        (n, _) => format!("{n} open pull requests"),
    };
    format!("{count} · Updated {updated}")
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

    fn two_prs() -> Vec<crate::model::PullRequest> {
        crate::github::parse_response(include_str!("../tests/fixtures/search_response.json"))
            .unwrap()
            .0
    }

    #[test]
    fn tooltip_carries_the_count_the_menu_bar_no_longer_shows() {
        let mut snapshot = Snapshot::default();
        snapshot.loaded(two_prs(), 2, "12:00".into());
        assert_eq!(
            tray_tooltip(&snapshot),
            "2 open pull requests · Updated 12:00"
        );
    }

    #[test]
    fn tooltip_reports_the_true_total_past_the_display_cap() {
        let mut snapshot = Snapshot::default();
        snapshot.loaded(two_prs(), 137, "12:00".into());
        assert_eq!(
            tray_tooltip(&snapshot),
            "2 of 137 open pull requests · Updated 12:00"
        );
    }

    #[test]
    fn tooltip_covers_loading_empty_singular_and_failed() {
        let mut snapshot = Snapshot::default();
        assert_eq!(tray_tooltip(&snapshot), "Loading pull requests…");

        snapshot.loaded(Vec::new(), 0, "12:00".into());
        assert_eq!(
            tray_tooltip(&snapshot),
            "No open pull requests · Updated 12:00"
        );

        snapshot.loaded(two_prs().into_iter().take(1).collect(), 1, "12:00".into());
        assert_eq!(
            tray_tooltip(&snapshot),
            "1 open pull request · Updated 12:00"
        );

        snapshot.failed("GitHub rejected the token (401)".into());
        assert_eq!(
            tray_tooltip(&snapshot),
            "Refresh failed: GitHub rejected the token (401)"
        );
    }

    #[test]
    fn truncate_keeps_unicode_and_only_shortens_past_the_limit() {
        assert_eq!(truncate("äbc", 3), "äbc");
        assert_eq!(truncate("äbcd", 3), "äb…");
        assert_eq!(truncate("anything", 1), "…");
    }
}
