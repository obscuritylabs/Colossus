"""No-engine tests for fixture serving and bounded native process execution."""

import os
from pathlib import Path
import sys
import tempfile
import threading
import unittest
from urllib.error import HTTPError
from urllib.request import urlopen
from unittest.mock import patch

import component
import run_probe


@unittest.skipUnless(sys.platform == "linux", "initial probe is Linux-only")
class ProbeTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.fixture = self.root / "fixture.html"
        self.fixture.write_text("<!doctype html><title>fixture</title>")

    def tearDown(self):
        self.temporary.cleanup()

    def stage(self, program):
        root = self.root / "component"
        root.mkdir()
        executable = root / run_probe.PROBE_EXECUTABLE
        executable.write_text(f"#!{sys.executable}\n" + program)
        executable.chmod(0o755)
        component.installed_inventory(root, "linux64", run_probe.PROBE_EXECUTABLE)
        return root

    def test_fixture_server_serves_only_the_selected_html(self):
        server = run_probe.fixture_server(self.fixture)
        thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.01})
        thread.start()
        try:
            base = f"http://127.0.0.1:{server.server_address[1]}"
            with urlopen(base + "/fixture.html", timeout=2) as response:
                self.assertEqual(response.read(), self.fixture.read_bytes())
            for path in ("/", "/../AGENTS.md", "/component", "/fixture.html?other"):
                with self.subTest(path=path), self.assertRaises(HTTPError) as error:
                    urlopen(base + path, timeout=2)
                self.assertEqual(error.exception.code, 404)
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=1)

    def test_success_requires_all_native_evidence_and_removes_display(self):
        root = self.stage(
            "import os,sys,urllib.request\n"
            "assert 'DISPLAY' not in os.environ and 'WAYLAND_DISPLAY' not in os.environ\n"
            "assert os.path.abspath('.') != os.path.dirname(os.path.abspath(sys.argv[0]))\n"
            "url=sys.argv[1].removeprefix('--fixture-url=')\n"
            "assert b'fixture' in urllib.request.urlopen(url).read()\n"
            "print('devtools_command=1 success=1 bounded_bytes=20')\n"
            "print('native_fixture=passed png_bytes=40')\n"
            "print('native_negative_controls=passed')\n"
            "print('native_offscreen_pixels=passed native_presentation_input=passed native_presentation_revocation=passed')\n"
        )
        with patch.dict(os.environ, {"DISPLAY": ":99", "WAYLAND_DISPLAY": "wayland-test"}):
            result = run_probe.run_probe(root, fixture=self.fixture)
        self.assertEqual(result.returncode, 0)
        self.assertIn("native_fixture=passed", result.stdout)

    def test_modified_component_is_rejected_before_execution(self):
        root = self.stage("print('must not execute')\n")
        (root / run_probe.PROBE_EXECUTABLE).write_text("changed")
        with self.assertRaises(component.ComponentError):
            run_probe.run_probe(root, fixture=self.fixture)

    def test_missing_manifest_is_rejected(self):
        root = self.root / "missing"
        root.mkdir()
        with self.assertRaises(component.ComponentError):
            run_probe.run_probe(root, fixture=self.fixture)

    def test_zero_exit_without_markers_is_not_acceptance(self):
        root = self.stage("print('native_fixture=passed')\n")
        with self.assertRaises(run_probe.ProbeError):
            run_probe.run_probe(root, fixture=self.fixture)

    def test_combined_output_bound_is_enforced(self):
        root = self.stage("import sys\nsys.stdout.write('x'*70000)\nsys.stdout.flush()\n")
        with self.assertRaisesRegex(run_probe.ProbeError, "64 KiB"):
            run_probe.run_probe(root, fixture=self.fixture)

    def test_deadline_is_enforced(self):
        root = self.stage("import time\ntime.sleep(10)\n")
        with self.assertRaisesRegex(run_probe.ProbeError, "deadline"):
            run_probe.run_probe(root, fixture=self.fixture, timeout=0.1)


if __name__ == "__main__":
    unittest.main()
