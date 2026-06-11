import hashlib
import json

import pytest

from colossus.adapters.bundles import ManifestBundleVerifier
from colossus.domain.errors import BundleVerificationError


def test_manifest_bundle_verifier_accepts_matching_hash(tmp_path) -> None:
    payload = tmp_path / "artifact.txt"
    payload.write_text("hello", encoding="utf-8")
    (tmp_path / "manifest.json").write_text(
        json.dumps(
            {
                "files": [
                    {
                        "path": "artifact.txt",
                        "sha256": hashlib.sha256(b"hello").hexdigest(),
                    }
                ]
            }
        ),
        encoding="utf-8",
    )

    assert ManifestBundleVerifier().verify(tmp_path) is True


def test_manifest_bundle_verifier_rejects_missing_manifest(tmp_path) -> None:
    with pytest.raises(BundleVerificationError, match="missing manifest"):
        ManifestBundleVerifier().verify(tmp_path)


@pytest.mark.parametrize(
    "manifest, message",
    [
        ({"files": "artifact.txt"}, "files list"),
        ({"files": ["artifact.txt"]}, "entries must be objects"),
        ({"files": [{"path": "artifact.txt"}]}, "require path and sha256"),
        ({"files": [{"path": 7, "sha256": "abc"}]}, "require path and sha256"),
    ],
)
def test_manifest_bundle_verifier_rejects_malformed_file_entries(
    tmp_path, manifest, message
) -> None:
    (tmp_path / "manifest.json").write_text(json.dumps(manifest), encoding="utf-8")

    with pytest.raises(BundleVerificationError, match=message):
        ManifestBundleVerifier().verify(tmp_path)


def test_manifest_bundle_verifier_rejects_missing_declared_file(tmp_path) -> None:
    (tmp_path / "manifest.json").write_text(
        json.dumps({"files": [{"path": "missing.txt", "sha256": "abc"}]}),
        encoding="utf-8",
    )

    with pytest.raises(BundleVerificationError, match="file missing"):
        ManifestBundleVerifier().verify(tmp_path)


def test_manifest_bundle_verifier_rejects_checksum_mismatch(tmp_path) -> None:
    (tmp_path / "artifact.txt").write_text("actual", encoding="utf-8")
    (tmp_path / "manifest.json").write_text(
        json.dumps(
            {
                "files": [
                    {
                        "path": "artifact.txt",
                        "sha256": hashlib.sha256(b"expected").hexdigest(),
                    }
                ]
            }
        ),
        encoding="utf-8",
    )

    with pytest.raises(BundleVerificationError, match="checksum mismatch"):
        ManifestBundleVerifier().verify(tmp_path)
