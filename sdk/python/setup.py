"""Release guard: never build a wheel without the generated public API."""

from pathlib import Path
from runpy import run_path

from setuptools import setup
from setuptools.command.build_py import build_py
from setuptools.command.sdist import sdist

verify_generated = run_path(str(Path(__file__).resolve().with_name("build_support.py")))[
    "verify_generated"
]


class VerifiedBuildPy(build_py):
    def run(self) -> None:
        verify_generated()
        super().run()


class VerifiedSdist(sdist):
    def run(self) -> None:
        verify_generated()
        super().run()


setup(cmdclass={"build_py": VerifiedBuildPy, "sdist": VerifiedSdist})
