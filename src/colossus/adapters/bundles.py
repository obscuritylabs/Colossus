"""Offline bundle verification."""

import hashlib
import json
from pathlib import Path

from colossus.domain.errors import BundleVerificationError


class ManifestBundleVerifier:
    def verify(self, bundle_path: Path) -> bool:
        manifest_path = bundle_path / "manifest.json"
        if not manifest_path.is_file():
            raise BundleVerificationError("Bundle is missing manifest.json")
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        files = manifest.get("files")
        if not isinstance(files, list):
            raise BundleVerificationError("Bundle manifest must contain a files list.")
        for item in files:
            if not isinstance(item, dict):
                raise BundleVerificationError("Bundle file entries must be objects.")
            rel_path = item.get("path")
            expected_sha = item.get("sha256")
            if not isinstance(rel_path, str) or not isinstance(expected_sha, str):
                raise BundleVerificationError("Bundle file entries require path and sha256.")
            actual_path = bundle_path / rel_path
            if not actual_path.is_file():
                raise BundleVerificationError(f"Bundle file missing: {rel_path}")
            actual_sha = hashlib.sha256(actual_path.read_bytes()).hexdigest()
            if actual_sha != expected_sha:
                raise BundleVerificationError(f"Bundle checksum mismatch: {rel_path}")
        return True
