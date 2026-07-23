import importlib.util
import pathlib
import unittest

SCRIPT = pathlib.Path(__file__).parents[1] / "check_package.py"
SPEC = importlib.util.spec_from_file_location("check_python_package", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load the Python package validator")
PACKAGE_CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGE_CHECK)


class PackageValidationTests(unittest.TestCase):
    def test_accepts_the_intended_wheel_surface(self) -> None:
        PACKAGE_CHECK.validate_wheel_names(
            {
                "colossus_sdk-0.1.0.dist-info/licenses/LICENSE",
                "colossus/api/v1alpha1/py.typed",
            }
        )

    def test_rejects_conflicting_google_modules(self) -> None:
        with self.assertRaises(AssertionError):
            PACKAGE_CHECK.validate_wheel_names(
                {
                    "colossus_sdk-0.1.0.dist-info/licenses/LICENSE",
                    "colossus/api/v1alpha1/py.typed",
                    "google/rpc/status_pb2.py",
                }
            )

    def test_accepts_the_intended_source_distribution(self) -> None:
        PACKAGE_CHECK.validate_sdist_names(
            {
                "colossus_sdk/generated-output.sha256",
                "colossus_sdk/generated/colossus/api/v1alpha1/agent_run_pb2.py",
                "colossus_sdk/generated/colossus/api/v1alpha1/py.typed",
            }
        )


if __name__ == "__main__":
    unittest.main()
