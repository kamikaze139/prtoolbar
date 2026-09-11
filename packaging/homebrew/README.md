# prtoolbar for macOS

A native menu bar app for your GitHub pull requests, reviews and stacks.
Supports Apple silicon and Intel Macs running macOS 12 or newer.

## Install

With [Homebrew](https://brew.sh) installed:

```sh
brew install --cask kamikaze139/tap/prtoolbar
```

This installs the app in Applications and installs GitHub CLI if needed.
Sign in once, then launch the app:

```sh
gh auth login --hostname github.com
open -a prtoolbar
```

Already signed in with GitHub CLI? Skip the login step. Look for the PR icon
in your menu bar. Either mouse button opens the list. Click a PR to open GitHub;
click elsewhere or press Escape to close the popup.

If macOS blocks the first launch of an unnotarized build, try opening the app,
then go to **System Settings → Privacy & Security → Open Anyway**.

## Update or remove

```sh
brew upgrade --cask kamikaze139/tap/prtoolbar
open -a prtoolbar
```

```sh
brew uninstall --cask prtoolbar
```

## Without Homebrew

Download the universal ZIP from [Releases](https://github.com/kamikaze139/homebrew-tap/releases/latest),
unzip it and move `prtoolbar.app` to Applications. Install [GitHub CLI](https://cli.github.com)
and sign in with `gh auth login --hostname github.com`, then open the app.

Releases use ad-hoc signing by default. Apple Developer ID signing and
notarization are optional; each release's notes state its notarization status.
The source repository is maintained separately; this repository contains the
installer definition and public downloads.

## Maintenance

The **Update prtoolbar** workflow publishes the latest public
[kamikaze139/prtoolbar](https://github.com/kamikaze139/prtoolbar) release. That
repository starts it as soon as a release finishes; an hourly schedule catches
anything the notification missed, and Run workflow in Actions publishes on
demand. It uses this repository's built-in GitHub Actions token to do the
publishing; no personal access token is required here.
