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

    def commit_fixture(self):
        (self.workspace / "source.txt").write_text("fixture source\n", encoding="utf-8")
        for arguments in (
            ["add", "."],
            ["-c", "user.name=Review tests", "-c", "user.email=review@example.invalid",
             "-c", "commit.gpgSign=false", "commit", "--quiet", "-m", "Test fixture"],
        ):
            subprocess.run(
                [shutil.which("git"), *arguments], cwd=self.workspace,
                check=True, capture_output=True, timeout=10,
            )

    def linked_worktree(self, name):
        checkout = self.root / name
        subprocess.run(
            [shutil.which("git"), "worktree", "add", "--quiet", "-b", name, str(checkout)],
            cwd=self.workspace, check=True, capture_output=True, timeout=10,
        )
        return checkout

    def run_helper(self, review_id="review-1", workspace=None):
        return subprocess.run(
            [sys.executable, str(INITIALIZER), "--workspace", str(workspace or self.workspace), "--review-id", review_id],
            env=self.environment, text=True, capture_output=True, check=False,
        )

    def success(self, review_id="review-1", workspace=None):
        result = self.run_helper(review_id, workspace)
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

    def test_linked_worktrees_share_primary_notes_and_record_requested_workspace(self):
        self.git_repository(ignore=True)
        self.commit_fixture()
        first = self.linked_worktree("first-worktree")
        scope = first / "component"
        scope.mkdir()
        result = self.success(workspace=scope)
        review = Path(result["review_dir"])
        self.assertEqual(review, self.workspace / ".local/security-reviews/review-1")
        self.assertEqual(result["workspace"], str(scope))
        self.assertEqual(result["storage_checkout"], str(self.workspace))
        brief = (review / "brief.md").read_text()
        self.assertIn(f"Requested workspace: {json.dumps(str(scope))}", brief)
        self.assertIn(f"Storage checkout: {json.dumps(str(self.workspace))}", brief)
        checkpoint = review / "checkpoint.md"
        checkpoint.write_text("retained worktree evidence\n", encoding="utf-8")
        second = self.linked_worktree("second-worktree")
        repeated = self.run_helper(workspace=second)
        self.assertNotEqual(repeated.returncode, 0)
        self.assertIn("already exists", repeated.stderr)
        self.assertEqual(checkpoint.read_text(), "retained worktree evidence\n")
        self.assertFalse((first / ".local").exists())
        self.assertFalse((second / ".local").exists())

    def test_linked_ignore_rules_do_not_authorize_unignored_primary_storage(self):
        self.git_repository()
        self.commit_fixture()
        linked = self.linked_worktree("linked-worktree")
        (linked / ".gitignore").write_text("/.local/\n", encoding="utf-8")
        result = self.success(workspace=linked)
        self.assertEqual(result["storage"], "temporary")
        self.assertTrue(Path(result["review_dir"]).is_relative_to(self.temp_root))
        self.assertFalse((self.workspace / ".local").exists())
        self.assertFalse((linked / ".local").exists())

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
