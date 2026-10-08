"""Behavioral checks for the portable security-review scratch initializer."""

import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest


INITIALIZER = (
    Path(__file__).resolve().parents[2]
    / "bundled-plugins/colossus/skills/security-review/scripts/init_review.py"
)


class ReviewWorkspaceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.workspace = self.root / "checkout with spaces"
        self.workspace.mkdir()
        self.temp_root = self.root / "temporary"
        self.temp_root.mkdir()
        self.environment = dict(os.environ, TMPDIR=str(self.temp_root), TEMP=str(self.temp_root), TMP=str(self.temp_root))

    def git_repository(self, ignore=False):
        git = shutil.which("git")
        if git is None:
            self.skipTest("Git is unavailable")
        subprocess.run(
            [git, "-c", "init.templateDir=", "init", "--quiet", str(self.workspace)],
            check=True, capture_output=True,
        )
        if ignore:
            (self.workspace / ".gitignore").write_text("/.local/\n", encoding="utf-8")

    def run_helper(self, review_id="review-1"):
        return subprocess.run(
            [sys.executable, str(INITIALIZER), "--workspace", str(self.workspace), "--review-id", review_id],
            env=self.environment, text=True, capture_output=True, check=False,
        )

    def success(self, review_id="review-1"):
        result = self.run_helper(review_id)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_ignored_checkout_gets_private_notes_without_source_or_config_changes(self):
        self.git_repository(ignore=True)
        source = self.workspace / "source.txt"
        source.write_text("unchanged source\n", encoding="utf-8")
        config = (self.workspace / ".git/config").read_bytes()
        result = self.success()
        review = Path(result["review_dir"])
        self.assertEqual(review, self.workspace / ".local/security-reviews/review-1")
        self.assertEqual(result["storage"], "workspace")
        self.assertEqual(source.read_text(), "unchanged source\n")
        self.assertEqual((self.workspace / ".git/config").read_bytes(), config)
        self.assertEqual((self.workspace / ".gitignore").read_text(), "/.local/\n")
        self.assertEqual(json.loads((review / "coverage.json").read_text())["status"], "not_started")
        self.assertEqual(json.loads((review / "findings.json").read_text()), {"candidates": []})
        self.assertFalse((review / "report.md").exists())
        if os.name == "posix":
            self.assertEqual(stat.S_IMODE(review.stat().st_mode), 0o700)
            self.assertEqual(stat.S_IMODE((review / "brief.md").stat().st_mode), 0o600)

    def test_unignored_repository_uses_temporary_storage_without_editing_ignore_rules(self):
        self.git_repository()
        result = self.success()
        self.assertEqual(result["storage"], "temporary")
        self.assertTrue(Path(result["review_dir"]).is_relative_to(self.temp_root))
        self.assertFalse((self.workspace / ".local").exists())
        self.assertFalse((self.workspace / ".gitignore").exists())

    def test_non_git_directory_has_a_working_fallback(self):
        result = self.success()
        self.assertEqual(result["storage"], "temporary")
        self.assertTrue((Path(result["review_dir"]) / "checkpoint.md").is_file())

    def test_existing_review_is_preserved_and_not_reinitialized(self):
        self.git_repository(ignore=True)
        review = Path(self.success()["review_dir"])
        note = review / "checkpoint.md"
        note.write_text("existing evidence\n", encoding="utf-8")
        result = self.run_helper()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("already exists", result.stderr)
        self.assertEqual(note.read_text(), "existing evidence\n")

    def test_traversal_ids_are_rejected_before_writes(self):
        result = self.run_helper("../../outside")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.workspace / ".local").exists())
        self.assertEqual(list(self.temp_root.iterdir()), [])

    def test_linked_scratch_parents_do_not_redirect_writes(self):
        self.git_repository(ignore=True)
        outside = self.root / "outside"
        outside.mkdir()
        try:
            (self.workspace / ".local").symlink_to(outside, target_is_directory=True)
        except OSError:
            self.skipTest("Directory symlinks are unavailable")
        result = self.success()
        self.assertEqual(result["storage"], "temporary")
        self.assertEqual(list(outside.iterdir()), [])

    def test_non_directory_scratch_parent_uses_safe_fallback(self):
        self.git_repository(ignore=True)
        (self.workspace / ".local").write_text("preserve me", encoding="utf-8")
        result = self.success()
        self.assertEqual(result["storage"], "temporary")
        self.assertEqual((self.workspace / ".local").read_text(), "preserve me")


if __name__ == "__main__":
    unittest.main()
