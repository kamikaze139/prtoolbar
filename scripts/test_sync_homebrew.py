"""Validate public-release metadata before the tap publishes anything."""

import hashlib
import tempfile
import unittest
from pathlib import Path

from test_render_cask import TEST_TMP

from sync_homebrew import notarization_status, should_update


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

    def test_sync_skips_current_and_older_releases(self):
        self.assertTrue(should_update(None, "v0.1.0"))
        self.assertTrue(should_update('  version "0.9.0"', "v0.10.0"))
        self.assertFalse(should_update('  version "0.10.0"', "v0.10.0"))
        self.assertFalse(should_update('  version "0.10.0"', "v0.9.0"))
        for tag in ["v0.1.0-rc.1", "0.1.0", "v../bad"]:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                should_update(None, tag)


if __name__ == "__main__":
    unittest.main()
