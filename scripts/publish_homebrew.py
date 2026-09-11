#!/usr/bin/env python3
"""Publish build artifacts to an existing public Homebrew tap.

Called by sync_homebrew.py with the build's notarization status. Requires GH_TOKEN with
Contents: write on the tap. No source code or private release notes are copied.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import tempfile
from pathlib import Path

from render_cask import render


def run(*args: str, cwd: Path | None = None) -> str:
    return subprocess.check_output(args, cwd=cwd, text=True).strip()


def version_tuple(value: str) -> tuple[int, int, int]:
    if not re.fullmatch(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", value):
        raise ValueError(f"expected a stable X.Y.Z version, got {value!r}")
    return tuple(int(part) for part in value.split("."))


def is_downgrade(current_cask: str, version: str) -> bool:
    match = re.search(r'^\s*version "([^"]+)"', current_cask, re.MULTILINE)
    if not match:
        raise ValueError("existing cask has no version; refusing to overwrite it")
    return version_tuple(match[1]) > version_tuple(version)


def release_notes(tap: str, *, notarized: bool) -> str:
    status = (
        "Signed with Developer ID and notarized by Apple.\n"
        if notarized else
        "This build is not notarized by Apple. If macOS blocks the first launch, "
        "open **System Settings → Privacy & Security → Open Anyway** after "
        "attempting to open the app.\n"
    )
    return (
        "Native macOS menu bar app for GitHub pull requests, reviews and stacks.\n\n"
        "Apple silicon and Intel; macOS 12 or newer.\n\n"
        f"```sh\nbrew install --cask {tap}/prtoolbar\n"
        "gh auth login --hostname github.com\nopen -a prtoolbar\n```\n\n"
        "Alternatively, download the ZIP below and move prtoolbar.app to Applications.\n\n"
        + status
    )


def publish(tag: str, repository: str, dist: Path, *, notarized: bool = False) -> None:
    archive = dist.resolve() / f"prtoolbar-{tag}-universal-apple-darwin.zip"
    cask = render(tag, repository, archive, notarized=notarized)
    owner, repo_name = repository.split("/")
    if not repo_name.startswith("homebrew-") or repo_name == "homebrew-":
        raise ValueError("tap repository must be named owner/homebrew-NAME")
    if not os.environ.get("GH_TOKEN"):
        raise ValueError("GH_TOKEN needs Contents: write on the tap")
    checksum = archive.with_suffix(archive.suffix + ".sha256")
    expected_hash = re.search(r'sha256 "([0-9a-f]{64})"', cask)[1]
    if checksum.read_text().split() != [expected_hash, archive.name]:
        raise ValueError("release checksum does not match the archive")

    info = json.loads(run("gh", "repo", "view", repository, "--json", "isPrivate,defaultBranchRef"))
    if info["isPrivate"]:
        raise ValueError("Homebrew downloads need a public tap; source can stay private")
    if not info["defaultBranchRef"]:
        raise ValueError("initialize the tap repository with a README before publishing")
    branch = info["defaultBranchRef"]["name"]
    tap = f"{owner}/{repo_name.removeprefix('homebrew-')}"

    with tempfile.TemporaryDirectory(prefix="prtoolbar-homebrew-") as directory:
        checkout = Path(directory) / "tap"
        run("gh", "auth", "setup-git")
        run("gh", "repo", "clone", repository, str(checkout), "--", "--depth=1", "--branch", branch)
        cask_path = checkout / "Casks" / "prtoolbar.rb"
        if cask_path.exists() and is_downgrade(cask_path.read_text(), tag[1:]):
            print("The tap already contains a newer version; skipping this older release.")
            return

        notes = Path(directory) / "notes.md"
        notes.write_text(release_notes(tap, notarized=notarized), encoding="utf-8")
        exists = subprocess.run(
            ["gh", "release", "view", tag, "--repo", repository],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False,
        ).returncode == 0
        if not exists:
            run("gh", "release", "create", tag, "--repo", repository, "--target", branch,
                "--title", f"prtoolbar {tag}", "--notes-file", str(notes), "--draft")
        run("gh", "release", "upload", tag, str(archive), str(checksum),
            "--repo", repository, "--clobber")
        run("gh", "release", "edit", tag, "--repo", repository, "--draft=false", "--latest",
            "--notes-file", str(notes))

        # Publish the cask only after its download is live. Stage only our own files.
        cask_path.parent.mkdir(parents=True, exist_ok=True)
        cask_path.write_text(cask, encoding="utf-8")
        run("git", "add", "Casks/prtoolbar.rb", cwd=checkout)
        if subprocess.run(["git", "diff", "--cached", "--quiet"], cwd=checkout, check=False).returncode == 0:
            print("Homebrew cask is already up to date.")
            return
        run("git", "config", "user.name", "github-actions[bot]", cwd=checkout)
        run("git", "config", "user.email", "41898282+github-actions[bot]@users.noreply.github.com", cwd=checkout)
        run("git", "commit", "-m", f"chore: update prtoolbar to {tag[1:]}", cwd=checkout)
        run("git", "push", "origin", f"HEAD:{branch}", cwd=checkout)
        print(f"Published: brew install --cask {tap}/prtoolbar")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--dist", required=True, type=Path)
    parser.add_argument("--notarized", choices=("true", "false"), default="false")
    args = parser.parse_args()
    publish(args.tag, args.repository, args.dist, notarized=args.notarized == "true")


if __name__ == "__main__":
    main()
