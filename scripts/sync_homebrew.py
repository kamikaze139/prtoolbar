#!/usr/bin/env python3
"""Run inside the public tap to copy the latest completed public app release.

Uses the tap's built-in GITHUB_TOKEN; no personal or cross-repository token.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import tempfile
from pathlib import Path

from publish_homebrew import publish, run, version_tuple
from render_cask import render


def should_update(current_cask: str | None, tag: str) -> bool:
    """True unless the tap already holds a newer release than this tag.

    The tag's own version is not a stop: the scripts rendering the cask come
    from the release, so re-running the same release is how a fix to the cask
    itself reaches users. main() compares the rendered cask and stops there
    when nothing actually changed.
    """
    if not tag.startswith("v"):
        raise ValueError("expected a stable vX.Y.Z tag")
    version = version_tuple(tag[1:])
    if current_cask is None:
        return True
    match = re.search(r'^\s*version "([^"]+)"', current_cask, re.MULTILINE)
    if not match:
        raise ValueError("existing cask has no stable version")
    return version >= version_tuple(match[1])


def notarization_status(metadata: dict, tag: str, archive: Path) -> bool:
    if metadata.get("tag") != tag or type(metadata.get("notarized")) is not bool:
        raise ValueError("release metadata must match the tag and contain a boolean notarized status")
    if metadata.get("sha256") != hashlib.sha256(archive.read_bytes()).hexdigest():
        raise ValueError("release metadata does not match the archive checksum")
    return metadata["notarized"]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True)
    parser.add_argument("--repository", required=True)
    args = parser.parse_args()
    tag = run("gh", "release", "view", "--repo", args.source, "--json", "tagName", "--jq", ".tagName")
    cask_path = Path("Casks/prtoolbar.rb")
    current_cask = cask_path.read_text() if cask_path.exists() else None
    if not should_update(current_cask, tag):
        print("The tap already contains a newer release.")
        return

    with tempfile.TemporaryDirectory(prefix="prtoolbar-sync-") as directory:
        dist = Path(directory)
        archive = f"prtoolbar-{tag}-universal-apple-darwin.zip"
        # Missing assets fail without changing the tap. The next scheduled run
        # retries if the source release was still building on this attempt.
        run("gh", "release", "download", tag, "--repo", args.source, "--dir", directory,
            "--pattern", archive, "--pattern", f"{archive}.sha256", "--pattern", "homebrew.json")
        metadata = json.loads((dist / "homebrew.json").read_text())
        notarized = notarization_status(metadata, tag, dist / archive)
        # The published cask, not the version, decides. That is what makes the
        # hourly run self-healing: a renderer fix in the same release rewrites
        # the cask here instead of waiting for someone to bump a version.
        if render(tag, args.repository, dist / archive, notarized=notarized) == current_cask:
            print("The tap already contains this release.")
            return
        publish(tag, args.repository, dist, notarized=notarized)


if __name__ == "__main__":
    main()
