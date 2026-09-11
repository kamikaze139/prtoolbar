"""Release integrity checks: cask URLs, archive checksums and input validation."""
import hashlib
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from render_cask import render

TEST_TMP = Path(__file__).resolve().parents[1] / "target" / "release-tooling-tests"
TEST_TMP.mkdir(parents=True, exist_ok=True)


class CaskTests(unittest.TestCase):
    def test_cask_points_to_distribution_repo_and_hashes_actual_archive(self):
        with tempfile.TemporaryDirectory(dir=TEST_TMP) as directory:
            archive = Path(directory) / "prtoolbar-v0.2.0-universal-apple-darwin.zip"
            archive.write_bytes(b"exact notarized archive")
            cask = render("v0.2.0", "someone/homebrew-tap", archive)
            self.assertIn(hashlib.sha256(archive.read_bytes()).hexdigest(), cask)
            self.assertIn('version "0.2.0"', cask)
            self.assertIn("someone/homebrew-tap/releases/download/v#{version}", cask)
            self.assertIn('depends_on formula: "gh"', cask)
            self.assertIn('app "prtoolbar.app"', cask)
            self.assertNotIn("no_check", cask)

    def test_unnotarized_builds_are_released_from_quarantine_on_install(self):
        # Ad-hoc signatures have a bare cdhash for a designated requirement, so it changes
        # every build and Homebrew can never inherit the user's Gatekeeper approval across an
        # upgrade. Without this the dialog returns on every single release, not just the first.
        with tempfile.TemporaryDirectory(dir=TEST_TMP) as directory:
            archive = Path(directory) / "prtoolbar-v0.2.0-universal-apple-darwin.zip"
            archive.write_bytes(b"release archive")
            cask = render("v0.2.0", "someone/homebrew-tap", archive)
            self.assertIn("postflight do", cask)
            self.assertIn('"/usr/bin/xattr"', cask)
            self.assertIn('"com.apple.quarantine"', cask)
            self.assertIn('"#{appdir}/prtoolbar.app"', cask)
            self.assertIn("not notarized by Apple", cask)
            # Saying otherwise would send people to a dialog they will never see.
            self.assertNotIn("Open Anyway", cask)

    def test_notarized_builds_leave_gatekeeper_untouched(self):
        with tempfile.TemporaryDirectory(dir=TEST_TMP) as directory:
            archive = Path(directory) / "prtoolbar-v0.2.0-universal-apple-darwin.zip"
            archive.write_bytes(b"release archive")
            cask = render("v0.2.0", "someone/homebrew-tap", archive, notarized=True)
            self.assertNotIn("postflight", cask)
            self.assertNotIn("xattr", cask)
            self.assertNotIn("quarantine", cask)
            self.assertNotIn("Open Anyway", cask)

    def test_release_notes_describe_the_actual_notarization_status(self):
        from publish_homebrew import release_notes
        self.assertIn("not notarized by Apple", release_notes("someone/tap", notarized=False))
        self.assertIn("Open Anyway", release_notes("someone/tap", notarized=False))
        self.assertNotIn("Open Anyway", release_notes("someone/tap", notarized=True))
        self.assertIn("notarized by Apple", release_notes("someone/tap", notarized=True))

    def test_rejects_prereleases_and_wrong_artifact_before_generating_a_cask(self):
        archive = Path("prtoolbar-v0.2.0-universal-apple-darwin.zip")
        for tag in ["v0.2.0-rc.1", "0.2.0", 'v0.2.0"', "v01.2.0"]:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                render(tag, "someone/homebrew-tap", archive)
        with self.assertRaises(ValueError):
            render("v0.3.0", "someone/homebrew-tap", archive)
        with self.assertRaises(ValueError):
            render("v0.2.0", 'someone/repo#{system("bad")}', archive)


class UpdateOrderTests(unittest.TestCase):
    def test_an_old_release_cannot_downgrade_the_tap(self):
        from publish_homebrew import is_downgrade
        self.assertTrue(is_downgrade('  version "0.10.0"', "0.9.9"))
        self.assertFalse(is_downgrade('  version "0.9.9"', "0.10.0"))
        self.assertFalse(is_downgrade('  version "0.10.0"', "0.10.0"))
        with self.assertRaises(ValueError):
            is_downgrade('version :latest', "0.10.0")


class PublicationGateTests(unittest.TestCase):
    def test_bad_checksum_never_reaches_github(self):
        from publish_homebrew import publish
        with tempfile.TemporaryDirectory(dir=TEST_TMP) as directory:
            dist = Path(directory)
            archive = dist / "prtoolbar-v0.2.0-universal-apple-darwin.zip"
            archive.write_bytes(b"release archive")
            archive.with_suffix(".zip.sha256").write_text("bad checksum")
            with patch.dict("os.environ", {"GH_TOKEN": "test-token"}), patch("publish_homebrew.run") as gh:
                with self.assertRaisesRegex(ValueError, "checksum"):
                    publish("v0.2.0", "someone/homebrew-tap", dist)
                gh.assert_not_called()

    def test_private_destination_is_rejected_before_any_publication(self):
        from publish_homebrew import publish
        with tempfile.TemporaryDirectory(dir=TEST_TMP) as directory:
            dist = Path(directory)
            archive = dist / "prtoolbar-v0.2.0-universal-apple-darwin.zip"
            archive.write_bytes(b"release archive")
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            archive.with_suffix(".zip.sha256").write_text(f"{digest}  {archive.name}\n")
            with patch.dict("os.environ", {"GH_TOKEN": "test-token"}), patch(
                "publish_homebrew.run", return_value='{"isPrivate": true, "defaultBranchRef": {"name": "main"}}'
            ) as gh:
                with self.assertRaisesRegex(ValueError, "public tap"):
                    publish("v0.2.0", "someone/homebrew-tap", dist)
                self.assertEqual(gh.call_count, 1)  # Only the read-only repository query.


if __name__ == "__main__":
    unittest.main()
