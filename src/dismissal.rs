//! Event policy for a tray popover. The anchor is handled on mouse-up only.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickTarget {
    Popover,
    TrayIcon,
    Elsewhere,
}

pub fn dismiss_on_mouse_down(shown: bool, target: ClickTarget) -> bool {
    shown && target == ClickTarget::Elsewhere
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outside_click_closes_even_when_the_popover_never_gained_focus() {
        assert!(dismiss_on_mouse_down(true, ClickTarget::Elsewhere));
        assert!(!dismiss_on_mouse_down(false, ClickTarget::Elsewhere));
    }

    #[test]
    fn anchor_mouse_down_must_not_close_before_the_mouse_up_toggle() {
        assert!(!dismiss_on_mouse_down(true, ClickTarget::TrayIcon));
        // Visibility remains true until the button action toggles it to false.
        assert!(!dismiss_on_mouse_down(true, ClickTarget::Popover));
    }
}
