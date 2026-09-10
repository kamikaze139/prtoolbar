//! Find a GitHub token: `GITHUB_TOKEN`, then the `gh` CLI.

use std::process::Command;

use anyhow::{Result, anyhow};

/// Locations tried for `gh`. App bundles launched from Finder do not inherit
/// the shell `PATH`, so Homebrew's prefixes are tried explicitly.
const GH_CANDIDATES: &[&str] = &["gh", "/opt/homebrew/bin/gh", "/usr/local/bin/gh"];

/// Resolve a token from the environment, falling back to `gh auth token`.
pub fn resolve_token() -> Result<String> {
    resolve_from(std::env::var("GITHUB_TOKEN").ok(), gh_auth_token)
}

/// Pure core of [`resolve_token`], testable without touching the environment.
pub fn resolve_from(
    env_token: Option<String>,
    gh_lookup: impl FnOnce() -> Option<String>,
) -> Result<String> {
    if let Some(token) = env_token
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
    {
        return Ok(token);
    }
    gh_lookup().ok_or_else(|| anyhow!("No GitHub token: set GITHUB_TOKEN or run `gh auth login`"))
}

fn gh_auth_token() -> Option<String> {
    GH_CANDIDATES.iter().find_map(|gh| {
        let output = Command::new(gh).args(["auth", "token"]).output().ok()?;
        if !output.status.success() {
            return None;
        }
        let token = String::from_utf8(output.stdout).ok()?.trim().to_owned();
        (!token.is_empty()).then_some(token)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_token_wins() {
        let token = resolve_from(Some(" ghp_env ".into()), || Some("ghp_cli".into())).unwrap();
        assert_eq!(token, "ghp_env");
    }

    #[test]
    fn empty_env_falls_back_to_gh() {
        let token = resolve_from(Some("   ".into()), || Some("ghp_cli".into())).unwrap();
        assert_eq!(token, "ghp_cli");
    }

    #[test]
    fn missing_everything_is_a_clear_error() {
        let err = resolve_from(None, || None).unwrap_err();
        assert_eq!(
            err.to_string(),
            "No GitHub token: set GITHUB_TOKEN or run `gh auth login`"
        );
    }
}
