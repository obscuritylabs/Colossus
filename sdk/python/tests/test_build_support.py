from __future__ import annotations

import shutil
import tempfile
import unittest
from pathlib import Path

from build_support import (
    ROOT,
    source_input_digest,
    source_input_paths,
    verify_source_inputs,
)


class SourceInputGuardTests(unittest.TestCase):
    def test_standalone_source_distribution_does_not_require_repository_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as temporary_directory:
            standalone_root = Path(temporary_directory) / "obscuritylabs_colossus_sdk-0.10.3"
            standalone_root.mkdir()
            verify_source_inputs(standalone_root)

    def test_source_checkout_rejects_stale_schema_inputs(self) -> None:
        repository_root = ROOT.parents[1]
        with tempfile.TemporaryDirectory() as temporary_directory:
            copied_repository = Path(temporary_directory)
            for source in source_input_paths(repository_root):
                destination = copied_repository / source.relative_to(repository_root)
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, destination)

            input_manifest = copied_repository / "sdk/generated-inputs.sha256"
            input_manifest.write_text(
                f"{source_input_digest(copied_repository)}\n",
                encoding="ascii",
            )
            copied_python_root = copied_repository / "sdk/python"
            verify_source_inputs(copied_python_root)

            schema = next((copied_repository / "api").rglob("*.proto"))
            schema.write_bytes(schema.read_bytes() + b"\n// stale generated binding test\n")
            with self.assertRaisesRegex(RuntimeError, "schema/tool inputs"):
                verify_source_inputs(copied_python_root)


if __name__ == "__main__":
    unittest.main()
