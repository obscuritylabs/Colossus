"""Pack repository and installer ports."""

from dataclasses import dataclass
from importlib.resources.abc import Traversable
from pathlib import Path
from typing import Protocol

from colossus.domain.integrations import IntegrationManifest
from colossus.domain.packs import (
    InstalledPack,
    PackInstallStatus,
    PackManifest,
    PackSourceKind,
    PackTrustStatus,
)


@dataclass(frozen=True)
class LoadedPack:
    manifest: PackManifest
    root: Path | Traversable
    source_kind: PackSourceKind
    source: str
    trust_status: PackTrustStatus
    status: PackInstallStatus = "enabled"


class PackRepository(Protocol):
    def list_packs(self) -> tuple[LoadedPack, ...]:
        """List available packs."""
        ...


class PackInstaller(Protocol):
    def verify_source(self, source: Path) -> tuple[PackManifest, PackSourceKind]:
        """Verify a pack source without installing it."""
        ...

    def install_source(
        self,
        source: Path,
        *,
        trust_status: PackTrustStatus,
    ) -> InstalledPack:
        """Install a verified pack source."""
        ...


class PackIntegrationLoader(Protocol):
    def integration_manifests_from_pack(
        self,
        pack: LoadedPack,
    ) -> tuple[IntegrationManifest, ...]:
        """Load integration manifests from a pack."""
        ...
