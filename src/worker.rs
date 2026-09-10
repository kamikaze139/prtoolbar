//! Background refresh: fetch PRs, then avatars, and hand results to the UI.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use tao::event_loop::EventLoopProxy;
use ureq::Agent;

use crate::avatars::AvatarCache;
use crate::events::AppEvent;
use crate::{auth, github};

const DEFAULT_INTERVAL: Duration = Duration::from_secs(60);
const MIN_INTERVAL: Duration = Duration::from_secs(15);
const GITHUB_TIMEOUT: Duration = Duration::from_secs(10);
const AVATAR_TIMEOUT: Duration = Duration::from_secs(5);

/// Requests the UI can send to the worker.
#[derive(Debug)]
pub enum Command {
    Refresh,
}

/// Start the refresh thread. Dropping the returned sender stops it after the
/// current cycle.
pub fn spawn(proxy: EventLoopProxy<AppEvent>, interval: Duration) -> Sender<Command> {
    let (tx, rx) = mpsc::channel();
    thread::Builder::new()
        .name("refresh".into())
        .spawn(move || run(&proxy, &rx, interval))
        .expect("spawn refresh thread");
    tx
}

/// Parse `PRTOOLBAR_INTERVAL_SECS`; invalid or missing values give the default.
pub fn interval_from_env(value: Option<&str>) -> Duration {
    value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map_or(DEFAULT_INTERVAL, |secs| {
            Duration::from_secs(secs).max(MIN_INTERVAL)
        })
}

/// An agent with a global timeout and our user agent. Non-2xx responses are
/// returned as responses, not errors, so callers can map status codes.
pub fn http_agent(timeout: Duration) -> Agent {
    Agent::config_builder()
        .timeout_global(Some(timeout))
        .user_agent(concat!("prtoolbar/", env!("CARGO_PKG_VERSION")))
        .http_status_as_error(false)
        .build()
        .new_agent()
}

fn run(proxy: &EventLoopProxy<AppEvent>, rx: &Receiver<Command>, interval: Duration) {
    let github = http_agent(GITHUB_TIMEOUT);
    let cdn = http_agent(AVATAR_TIMEOUT);
    let mut cache = AvatarCache::default();

    loop {
        let result =
            auth::resolve_token().and_then(|token| github::fetch_open_prs(&github, &token));
        match result {
            Ok(prs) => {
                let urls: Vec<String> = prs
                    .iter()
                    .flat_map(|pr| pr.reviewers.iter().map(|r| r.avatar_url.clone()))
                    .collect();
                if proxy.send_event(AppEvent::Loaded(prs)).is_err() {
                    return;
                }
                let fresh = cache.fetch_missing(&cdn, urls);
                if !fresh.is_empty() && proxy.send_event(AppEvent::Avatars(fresh)).is_err() {
                    return;
                }
            }
            Err(err) => {
                log::warn!("refresh failed: {err:#}");
                if proxy
                    .send_event(AppEvent::Failed(format!("{err:#}")))
                    .is_err()
                {
                    return;
                }
            }
        }

        match rx.recv_timeout(interval) {
            Ok(Command::Refresh) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_defaults_to_sixty_seconds() {
        assert_eq!(interval_from_env(None), Duration::from_secs(60));
    }

    #[test]
    fn interval_is_parsed_and_clamped() {
        assert_eq!(interval_from_env(Some("120")), Duration::from_secs(120));
        assert_eq!(interval_from_env(Some("3")), Duration::from_secs(15));
        assert_eq!(interval_from_env(Some("abc")), Duration::from_secs(60));
        assert_eq!(interval_from_env(Some("")), Duration::from_secs(60));
    }
}
