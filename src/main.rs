//! prtoolbar — a macOS menu bar app that lists your open GitHub pull requests.
//!
//! This is the initial skeleton: it shows a tray icon with a Quit item and
//! proves the threading model (worker thread -> event loop proxy). See
//! `docs/superpowers/plans/` for the implementation plan.

#![allow(dead_code)] // Removed in Task 8 once every module is wired up.

mod auth;
mod avatars;
mod github;
mod icons;
mod menu;
mod model;

use std::thread;
use std::time::Duration;

use anyhow::Result;
use tao::event::Event;
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::menu::{Menu, MenuEvent, PredefinedMenuItem};
use tray_icon::{Icon, TrayIconBuilder};

/// Events delivered to the main thread.
#[derive(Debug)]
enum AppEvent {
    /// A menu item was activated.
    Menu(MenuEvent),
    /// The worker thread ticked.
    Tick,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let mut event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(AppEvent::Menu(event));
    }));

    let proxy = event_loop.create_proxy();
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(60));
            if proxy.send_event(AppEvent::Tick).is_err() {
                break;
            }
        }
    });

    let menu = Menu::new();
    let quit = PredefinedMenuItem::quit(Some("Quit prtoolbar"));
    menu.append(&quit)?;

    let tray = TrayIconBuilder::new()
        .with_icon(placeholder_icon())
        .with_icon_as_template(true)
        .with_title("…")
        .with_menu(Box::new(menu))
        .build()?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        // Keep the tray alive for the lifetime of the loop.
        let _ = &tray;
        match event {
            Event::UserEvent(AppEvent::Menu(menu_event)) => log::debug!("menu: {menu_event:?}"),
            Event::UserEvent(AppEvent::Tick) => log::debug!("tick"),
            _ => {}
        }
    });
}

/// A solid 32x32 square used until real icons exist.
fn placeholder_icon() -> Icon {
    const SIZE: u32 = 32;
    let rgba = [0, 0, 0, 255].repeat((SIZE * SIZE) as usize);
    Icon::from_rgba(rgba, SIZE, SIZE).expect("placeholder icon dimensions are valid")
}
