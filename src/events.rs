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
