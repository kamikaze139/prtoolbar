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

    let interval =
        worker::interval_from_env(std::env::var("PRTOOLBAR_INTERVAL_SECS").ok().as_deref());
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
            AppEvent::Loaded { prs, total } => {
                self.snapshot.prs = prs;
                self.snapshot.total = total;
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
