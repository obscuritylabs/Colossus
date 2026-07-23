#!/usr/bin/env python3

from __future__ import annotations

import os
import pathlib
import subprocess
import sys
import tarfile
import tempfile
import zipfile


def validate_wheel_names(names: set[str]) -> None:
    if not any(name.endswith(".dist-info/licenses/LICENSE") for name in names):
        raise AssertionError(names)
    if "colossus/api/v1alpha1/py.typed" not in names:
        raise AssertionError(names)
    if any(name.startswith("google/") for name in names):
        raise AssertionError(names)


def validate_sdist_names(names: set[str]) -> None:
    def contains(suffix: str) -> bool:
        return any(name.endswith(suffix) for name in names)

    if not contains("/generated-output.sha256"):
        raise AssertionError(names)
    if not contains("/generated/colossus/api/v1alpha1/agent_run_pb2.py"):
        raise AssertionError(names)
    if not contains("/generated/colossus/api/v1alpha1/py.typed"):
        raise AssertionError(names)
    if any("/generated/google/" in name for name in names):
        raise AssertionError(names)


def one_artifact(dist: pathlib.Path, pattern: str) -> pathlib.Path:
    artifacts = list(dist.glob(pattern))
    if len(artifacts) != 1:
        raise AssertionError(f"expected one {pattern} artifact, found {artifacts}")
    return artifacts[0]


def main() -> None:
    dist = pathlib.Path("dist")
    wheel = one_artifact(dist, "*.whl")
    with zipfile.ZipFile(wheel) as archive:
        validate_wheel_names(set(archive.namelist()))

    sdist = one_artifact(dist, "*.tar.gz")
    with tarfile.open(sdist) as archive:
        validate_sdist_names(set(archive.getnames()))

    with tempfile.TemporaryDirectory(prefix="colossus-python-wheel.") as smoke_root:
        subprocess.run(  # noqa: S603 - arguments are fixed and the wheel is locally built.
            [
                sys.executable,
                "-m",
                "pip",
                "install",
                "--disable-pip-version-check",
                "--no-deps",
                "--no-index",
                "--target",
                smoke_root,
                str(wheel),
            ],
            check=True,
        )
        smoke_env = os.environ.copy()
        smoke_env["PYTHONPATH"] = smoke_root
        subprocess.run(  # noqa: S603 - executes this exact interpreter with fixed code.
            [
                sys.executable,
                "-c",
                """
from colossus.api.v1alpha1.agent_run_pb2 import RunResult, RunStateChanged, RunUpdate
from colossus_sdk import decode_colossus_rpc_error, is_terminal_run_update
from google.rpc.status_pb2 import Status

assert callable(decode_colossus_rpc_error)
assert Status(code=3).code == 3
assert is_terminal_run_update(RunUpdate(result=RunResult()))
assert not is_terminal_run_update(RunUpdate(state=RunStateChanged()))
""",
            ],
            check=True,
            env=smoke_env,
        )


if __name__ == "__main__":
    main()
