//! prtoolbar — native macOS menu bar pull requests.

mod auth;
mod avatars;
mod dismissal;
mod events;
mod github;
mod icons;
mod menu;
mod model;
mod native;
mod tree;
mod worker;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    native::run()
}
