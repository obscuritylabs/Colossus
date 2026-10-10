import importlib.util
from pathlib import Path
import shutil
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("mac_network_policies", Path(__file__).with_name("mac_network_policies.py"))
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)
SOURCE = Path(__file__).resolve().parents[1] / "mac" / "sandbox"


class PolicyPins(unittest.TestCase):
    def test_exact_policies_generate_c_literals(self):
        output = MODULE.generate(SOURCE)
        self.assertIn("colossus_policy_common_sha256", output)
        self.assertIn("colossus_policy_renderer_sha256", output)
        self.assertIn("colossus_policy_network_sha256", output)

    def test_changed_policy_fails_closed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "policies"
            shutil.copytree(SOURCE, root)
            with (root / "renderer.sb").open("ab") as target:
                target.write(b"\n(allow network*)\n")
            with self.assertRaises(ValueError):
                MODULE.generate(root)

    def test_symlink_policy_fails_closed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "policies"
            shutil.copytree(SOURCE, root)
            (root / "renderer.sb").unlink()
            (root / "renderer.sb").symlink_to(SOURCE / "renderer.sb")
            with self.assertRaises(ValueError):
                MODULE.generate(root)


if __name__ == "__main__":
    unittest.main()
