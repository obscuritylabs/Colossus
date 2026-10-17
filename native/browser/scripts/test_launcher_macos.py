"""Exercise development launcher command and environment handoff without builds."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


@unittest.skipUnless(sys.platform == "darwin", "launcher requires Apple signing tools")
class MacosLauncherTests(unittest.TestCase):
    def test_launcher_selects_embedded_assets_and_preserves_build_separation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            repository = root / "checkout"
            scripts = repository / "scripts"
            scripts.mkdir(parents=True)
            (repository / "apps/desktop").mkdir(parents=True)
            (repository / ".local").mkdir()
            original = Path(__file__).resolve().parents[3] / "scripts/desktop-chromium-preview"
            shutil.copy2(original, scripts / original.name)
            shutil.copy2(original.parent / "development-launch.mjs", scripts / "development-launch.mjs")
            launch_wrapper = scripts / "open-desktop-chromium-preview"
            wrapper_source = (original.parent / launch_wrapper.name).read_text()
            self.assertIn("open_command=/usr/bin/open\n", wrapper_source)
            # Keep the real wrapper's argument/environment logic while replacing
            # only its OS executable, so the fixture cannot activate a real app.
            launch_wrapper.write_text(wrapper_source.replace("open_command=/usr/bin/open\n",
                                                            "open_command=$FIXTURE_OPEN\n"))
            launch_wrapper.chmod(0o755)
            commands = root / "commands"
            commands.mkdir()
            log = root / "commands.jsonl"
            shim = commands / "shim"
            shim.write_text(f"#!{sys.executable}\n" + r'''
import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
argv = sys.argv[1:]
repo = pathlib.Path(os.environ["FIXTURE_REPOSITORY"])
with open(os.environ["FIXTURE_LOG"], "a") as output:
    output.write(json.dumps({"name": name, "argv": argv,
        "target": os.environ.get("CARGO_TARGET_DIR"),
        "cef_root": os.environ.get("COLOSSUS_CEF_ROOT"),
        "native": os.environ.get("COLOSSUS_CEF_NATIVE_LIB_DIR"),
        "malloc": os.environ.get("MallocNanoZone"),
        "raw_development_key": "COLOSSUS_DEV_JOURNAL_KEY" in os.environ,
        "raw_journal_key": "COLOSSUS_JOURNAL_KEY" in os.environ,
        "authority_selector": "COLOSSUS_DEVELOPMENT_CREDENTIAL_AUTHORITY" in os.environ}) + "\n")
if name == "uname":
    print("Darwin" if argv == ["-s"] else "arm64")
elif name == "rustc":
    print("aarch64-apple-darwin")
elif name == "node" and (argv[1:3] == ["--", "python3"] or
                        any(value.endswith("/open-desktop-chromium-preview") for value in argv)):
    # Use the real environment seam for Python provisioning and OS handoff.
    os.execv(os.environ["FIXTURE_NODE"], [os.environ["FIXTURE_NODE"], *argv])
elif name == "python3" and "fetch" in argv:
    print(repo / ".local/cef-cache/sources/pinned")
elif name == "node" and "prepare" in argv:
    binary = repo / "apps/desktop/src-tauri/binaries/colossus-aarch64-apple-darwin"
    binary.parent.mkdir(parents=True, exist_ok=True)
    binary.write_bytes(pathlib.Path(sys.argv[0]).read_bytes())
    binary.chmod(0o755)
elif name.startswith("colossus-"):
    print(json.dumps({"active": True, "authorityPath": os.environ["FIXTURE_AUTHORITY"]}))
elif name == "node" and "build" in argv:
    binary = pathlib.Path(os.environ["CARGO_TARGET_DIR"]) / "debug/colossus-desktop"
    binary.parent.mkdir(parents=True, exist_ok=True)
    binary.write_bytes(b"compiled Desktop")
elif name == "python3" and "--app" in argv:
    app = pathlib.Path(argv[argv.index("--app") + 1])
    binary = app / "Contents/MacOS/colossus-desktop"
    binary.parent.mkdir(parents=True)
    binary.write_bytes(b"staged Desktop")
    print(binary)
''')
            shim.chmod(0o755)
            for name in ("uname", "rustc", "python3", "node", "npm", "cargo", "cmake", "open-fixture"):
                (commands / name).symlink_to(shim.name)
            environment = dict(os.environ)
            environment.update({"PATH": str(commands) + ":/usr/bin:/bin",
                                "FIXTURE_REPOSITORY": str(repository), "FIXTURE_LOG": str(log),
                                "FIXTURE_NODE": shutil.which("node"),
                                "FIXTURE_OPEN": str(commands / "open-fixture"),
                                "FIXTURE_AUTHORITY": str(root / "active-authority"),
                                "CARGO_TARGET_DIR": ".local/runtime-target",
                                "COLOSSUS_CEF_NATIVE_LIB_DIR": ".local/review-native",
                                "COLOSSUS_HOME": str(root / "private-home"),
                                "COLOSSUS_DEV_JOURNAL_KEY": "synthetic-development-key",
                                "COLOSSUS_JOURNAL_KEY": "synthetic-journal-key",
                                "COLOSSUS_DEVELOPMENT_CREDENTIAL_AUTHORITY": str(root / "synthetic-authority")})
            result = subprocess.run(["/bin/sh", str(scripts / original.name), "--dev-credentials"],
                                    env=environment, capture_output=True, text=True, timeout=20)
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            prepare = next(call for call in calls if call["name"] == "node" and "prepare" in call["argv"])
            self.assertEqual(prepare["target"], ".local/runtime-target")
            build = next(call for call in calls if call["name"] == "node" and "build" in call["argv"])
            self.assertEqual(build["argv"][2:], ["npm", "run", "tauri", "--", "build", "--debug",
                                                "--no-bundle", "--features", "embedded-chromium-preview",
                                                "--", "--locked"])
            self.assertEqual(build["target"], str(repository / ".local/runtime-target/desktop-acceptance"))
            self.assertEqual(build["native"], str(repository / ".local/review-native"))
            stage = next(call for call in calls if call["name"] == "python3" and "--app" in call["argv"])
            self.assertIn(str(repository / ".local/review-native"), stage["argv"])
            self.assertIn("--dictation-resources", stage["argv"])
            self.assertIn(str(repository / "apps/desktop/src-tauri/dictation-assets"), stage["argv"])
            python_calls = [call for call in calls if call["name"] == "python3"]
            self.assertEqual(len(python_calls), 2)
            self.assertTrue(all(not call["raw_development_key"] and not call["raw_journal_key"]
                                and not call["authority_selector"] for call in python_calls))
            launch = calls[-1]
            self.assertEqual(launch["name"], "open-fixture")
            self.assertEqual(launch["malloc"], "0")
            self.assertEqual(launch["argv"][:2], ["-n", "-W"])
            self.assertTrue(launch["argv"][2].endswith("Colossus Chromium Preview.app"))
            self.assertIn("COLOSSUS_HOME=" + str(root / "private-home"), launch["argv"])
            self.assertIn("COLOSSUS_DEVELOPMENT_CREDENTIAL_AUTHORITY=" + str(root / "active-authority"),
                          launch["argv"])
            self.assertIn("MallocNanoZone=0", launch["argv"])
            self.assertFalse(launch["raw_development_key"])
            self.assertFalse(launch["raw_journal_key"])
            self.assertTrue(launch["authority_selector"])
            status = next(call for call in calls if call["name"].startswith("colossus-"))
            self.assertFalse(status["raw_development_key"])
            self.assertFalse(status["raw_journal_key"])
            self.assertFalse(status["authority_selector"])
            for flag in ("--stdout", "--stderr"):
                path = Path(launch["argv"][launch["argv"].index(flag) + 1])
                self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            # Ordinary preview launches cannot forward an ambient authority
            # selector merely because one existed in the invoking shell.
            log.unlink()
            result = subprocess.run(["/bin/sh", str(scripts / original.name)], env=environment,
                                    capture_output=True, text=True, timeout=20)
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            launch = calls[-1]
            self.assertEqual(launch["name"], "open-fixture")
            self.assertIn("COLOSSUS_HOME=" + str(root / "private-home"), launch["argv"])
            self.assertFalse(any(value.startswith("COLOSSUS_DEVELOPMENT_CREDENTIAL_AUTHORITY=")
                                 for value in launch["argv"]))
            self.assertFalse(launch["authority_selector"])
            self.assertFalse(launch["raw_development_key"])
            self.assertFalse(launch["raw_journal_key"])
            self.assertFalse(any(call["name"].startswith("colossus-") for call in calls))


if __name__ == "__main__":
    unittest.main()
