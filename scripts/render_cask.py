#!/usr/bin/env python3
"""Generate the Homebrew cask from the exact release archive being published."""

import argparse
import hashlib
import re
from pathlib import Path


def render(tag: str, repository: str, archive: Path, *, notarized: bool = False) -> str:
    if not re.fullmatch(r"v(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", tag):
        raise ValueError("Homebrew requires a stable vX.Y.Z release tag")
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9-]*/[A-Za-z0-9_.-]+", repository):
        raise ValueError("repository must be a GitHub owner/repository")
    expected = f"prtoolbar-{tag}-universal-apple-darwin.zip"
    if archive.name != expected:
        raise ValueError(f"archive must be named {expected}")
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    notice = "" if notarized else (
        "\n    This build is not notarized by Apple. If macOS blocks the first launch,\n"
        "    open System Settings > Privacy & Security > Open Anyway after trying\n"
        "    to open the app.\n"
    )
    return f'''cask "prtoolbar" do
  version "{tag[1:]}"
  sha256 "{digest}"

  url "https://github.com/{repository}/releases/download/v#{{version}}/prtoolbar-v#{{version}}-universal-apple-darwin.zip"
  name "prtoolbar"
  desc "Menu bar app for GitHub pull requests, reviews, and stacks"
  homepage "https://github.com/{repository}"

  depends_on formula: "gh"
  depends_on macos: :monterey

  app "prtoolbar.app"

  uninstall quit: "de.kaminski.prtoolbar"

  caveats <<~EOS
    Sign in to GitHub once, then open the menu bar app:
      gh auth login --hostname github.com
      open -a prtoolbar

    Already signed in with gh? Just open the app.
{notice}  EOS
end
'''


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--notarized", choices=("true", "false"), default="false")
    args = parser.parse_args()
    cask = render(args.tag, args.repository, args.archive, notarized=args.notarized == "true")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(cask, encoding="utf-8")


if __name__ == "__main__":
    main()
