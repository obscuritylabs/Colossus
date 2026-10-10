#!/usr/bin/env python3
"""Stage the dedicated macOS CEF host and scoped-sandbox helper bundles.

This explicitly invoked source-development stager neither acquires Chromium nor
establishes publisher identity, OS containment, or a release capability.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import subprocess
import sys

import component
import stage_macos


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cef-root", type=Path, required=True)
    parser.add_argument("--native-build", type=Path, required=True)
    parser.add_argument("--executable", type=Path, required=True)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--platform", choices=("macosarm64", "macosx64"), required=True)
    options = parser.parse_args()
    try:
        if sys.platform != "darwin":
            raise component.ComponentError("macOS native host staging requires codesign and lipo")
        print(stage_macos.stage_app(
            options.cef_root, options.native_build, options.executable, options.app,
            options.platform, "com.obscuritylabs.colossus.native-browser-host.preview", host=True,
        ))
        return 0
    except (component.ComponentError, OSError, ValueError, subprocess.CalledProcessError):
        # Tool exceptions may include private paths; only a fixed category escapes.
        print("macOS native browser host staging failed", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
