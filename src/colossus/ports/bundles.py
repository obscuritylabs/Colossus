"""Offline bundle verification port."""

from pathlib import Path
from typing import Protocol


class BundleVerifier(Protocol):
    def verify(self, bundle_path: Path) -> bool:
        """Return whether a bundle verifies."""
        ...
