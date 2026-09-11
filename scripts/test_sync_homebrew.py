"""Validate public-release metadata before the tap publishes anything."""

import hashlib
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from test_render_cask import TEST_TMP

import sync_homebrew
from render_cask import render
from sync_homebrew import notarization_status, should_update

ARCHIVE_BYTES = b"release archive"
TAP = "someone/homebrew-tap"


def fake_run(*args: str) -> str:
    """Stand in for the two gh calls main() makes, publishing v0.2.0."""
    if args[:3] == ("gh", "release", "view"):
        return "v0.2.0"
    if args[:3] == ("gh", "release", "download"):
        dist = Path(args[args.index("--dir") + 1])
        archive = dist / "prtoolbar-v0.2.0-universal-apple-darwin.zip"
        archive.write_bytes(ARCHIVE_BYTES)
        digest = hashlib.sha256(ARCHIVE_BYTES).hexdigest()
        archive.with_suffix(".zip.sha256").write_text(f"{digest}  {archive.name}\n")
        (dist / "homebrew.json").write_text(
            json.dumps({"tag": "v0.2.0", "notarized": False, "sha256": digest})
        )
        return ""
    raise AssertionError(f"unexpected command: {args}")


def sync_in(tap_checkout: Path) -> mock.Mock:
    """Run main() against a throwaway tap checkout; returns the publish mock."""
    argv = ["sync_homebrew.py", "--source", "kamikaze139/prtoolbar", "--repository", TAP]
    previous = Path.cwd()
    os.chdir(tap_checkout)
    try:
        with mock.patch.object(sync_homebrew, "run", fake_run), \
             mock.patch.object(sync_homebrew, "publish") as publish, \
             mock.patch.object(sys, "argv", argv):
            sync_homebrew.main()
        return publish
    finally:
        os.chdir(previous)


class SyncTests(unittest.TestCase):
    def test_only_matching_metadata_bound_to_the_archive_is_accepted(self):
        with tempfile.TemporaryDirectory(dir=TEST_TMP) as directory:
            archive = Path(directory) / "release.zip"
            archive.write_bytes(b"release archive")
            metadata = {
                "tag": "v0.1.0", "notarized": False,
                "sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
            }
            self.assertFalse(notarization_status(metadata, "v0.1.0", archive))
            self.assertTrue(notarization_status(dict(metadata, notarized=True), "v0.1.0", archive))
            for change in [
                {"tag": "v0.2.0"}, {"notarized": "false"},
                {"notarized": None}, {"sha256": "0" * 64},
            ]:
                with self.subTest(change=change), self.assertRaises(ValueError):
                    notarization_status(dict(metadata, **change), "v0.1.0", archive)
            archive.write_bytes(b"a concurrent rebuild of the same tag")
            with self.assertRaises(ValueError):
                notarization_status(metadata, "v0.1.0", archive)

    def test_sync_skips_older_releases_but_reconsiders_the_current_one(self):
        # The current version is not a stop: a fix to the renderer ships in the
        # same release and has to be able to rewrite an already published cask.
        self.assertTrue(should_update(None, "v0.1.0"))
        self.assertTrue(should_update('  version "0.9.0"', "v0.10.0"))
        self.assertTrue(should_update('  version "0.10.0"', "v0.10.0"))
        self.assertFalse(should_update('  version "0.10.0"', "v0.9.0"))
        for tag in ["v0.1.0-rc.1", "0.1.0", "v../bad"]:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                should_update(None, tag)

    def test_an_unchanged_cask_is_left_alone(self):
        with tempfile.TemporaryDirectory(dir=TEST_TMP) as directory:
            checkout = Path(directory)
            archive = checkout / "prtoolbar-v0.2.0-universal-apple-darwin.zip"
            archive.write_bytes(ARCHIVE_BYTES)
            (checkout / "Casks").mkdir()
            (checkout / "Casks/prtoolbar.rb").write_text(render("v0.2.0", TAP, archive))
            archive.unlink()
            sync_in(checkout).assert_not_called()

    def test_a_drifted_cask_is_republished_without_a_version_bump(self):
        # v0.1.3 shipped a cask the tap rendered with a stale copy of the
        # renderer, so the quarantine fix reached nobody and no version bump
        # would have helped. Publishing must key off the cask, not the version.
        with tempfile.TemporaryDirectory(dir=TEST_TMP) as directory:
            checkout = Path(directory)
            (checkout / "Casks").mkdir()
            (checkout / "Casks/prtoolbar.rb").write_text('cask "prtoolbar" do\n  version "0.2.0"\nend\n')
            publish = sync_in(checkout)
            publish.assert_called_once()
            self.assertEqual(publish.call_args.args[:2], ("v0.2.0", TAP))
            self.assertIs(publish.call_args.kwargs["notarized"], False)

    def test_the_tap_workflow_runs_the_release_s_own_publishing_scripts(self):
        # The tap was bootstrapped with copies of these scripts and nothing kept
        # them in step, so every renderer fix stopped at this repository's edge.
        workflow = (Path(__file__).resolve().parents[1] / "packaging/homebrew/update.yml").read_text()
        self.assertNotIn("python3 scripts/sync_homebrew.py", workflow)
        self.assertIn('"$RUNNER_TEMP/scripts/sync_homebrew.py"', workflow)
        self.assertIn("contents/scripts/$script?ref=$tag", workflow)
        for script in ["render_cask.py", "publish_homebrew.py", "sync_homebrew.py"]:
            with self.subTest(script=script):
                self.assertIn(script, workflow)


if __name__ == "__main__":
    unittest.main()
