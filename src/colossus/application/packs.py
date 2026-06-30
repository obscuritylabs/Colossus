"""Pack installation, trust, and discovery service."""

from collections.abc import Callable
from pathlib import Path

from colossus.domain.errors import ColossusError
from colossus.domain.integrations import IntegrationManifest
from colossus.domain.packs import (
    InstalledPack,
    PackInstallStatus,
    PackManifest,
    PackStatusView,
    PackTrustRecord,
    PackTrustStatus,
    PackVerificationResult,
    utc_now_iso,
)
from colossus.ports.audit import AuditSink
from colossus.ports.packs import LoadedPack, PackInstaller, PackIntegrationLoader, PackRepository
from colossus.ports.state import StateStore


class PackService:
    def __init__(
        self,
        state_store: StateStore,
        audit_sink: AuditSink,
        installer: PackInstaller,
        bundled_repository: PackRepository,
        integration_loader: PackIntegrationLoader,
        *,
        marker_writer: Callable[[InstalledPack], None] | None = None,
    ) -> None:
        self._state_store = state_store
        self._audit_sink = audit_sink
        self._installer = installer
        self._bundled_repository = bundled_repository
        self._integration_loader = integration_loader
        self._marker_writer = marker_writer

    async def list_statuses(self) -> tuple[PackStatusView, ...]:
        packs = [self._status_for_loaded(pack) for pack in self._bundled_repository.list_packs()]
        for installed in await self._state_store.list_installed_packs():
            packs.append(self._status_for_installed(installed))
        return tuple(sorted(packs, key=lambda pack: (pack.name, pack.version)))

    async def get_pack(self, name: str) -> InstalledPack | LoadedPack:
        normalized = _normalize_name(name)
        for pack in self._bundled_repository.list_packs():
            if pack.manifest.name == normalized:
                return pack
        installed = await self._state_store.get_installed_pack(normalized)
        if installed is None:
            raise ColossusError(f"Unknown pack: {name}")
        return installed

    async def verify(self, source: Path) -> PackVerificationResult:
        manifest, source_kind = self._installer.verify_source(source)
        trust_status = await self._trust_status(manifest)
        return PackVerificationResult(
            name=manifest.name,
            version=manifest.version,
            source_kind=source_kind,
            trust_status=trust_status,
            file_count=len(manifest.files),
            capabilities=manifest.capabilities,
            permissions=manifest.permissions,
        )

    async def install(self, source: Path, *, allow_untrusted: bool = False) -> InstalledPack:
        manifest, _source_kind = self._installer.verify_source(source)
        trust_status = await self._trust_status(manifest)
        if trust_status != "trusted" and not allow_untrusted:
            raise ColossusError(
                "Pack is unsigned or untrusted. Re-run with --allow-untrusted to install it."
            )
        installed = self._installer.install_source(source, trust_status=trust_status)
        await self._state_store.save_installed_pack(installed)
        await self._audit_sink.record(
            "user",
            "pack.installed",
            _audit_pack_details(installed, allow_untrusted=allow_untrusted),
        )
        return installed

    async def uninstall(self, name: str) -> None:
        normalized = _normalize_name(name)
        installed = await self._state_store.get_installed_pack(normalized)
        if installed is None:
            raise ColossusError(f"Pack is not installed: {name}")
        path = Path(installed.installed_path)
        if path.exists():
            import shutil

            shutil.rmtree(path)
        await self._state_store.delete_installed_pack(normalized)
        await self._audit_sink.record(
            "user",
            "pack.uninstalled",
            {"name": installed.name, "version": installed.version},
        )

    async def enable(self, name: str) -> InstalledPack:
        return await self._set_status(name, "enabled")

    async def disable(self, name: str) -> InstalledPack:
        return await self._set_status(name, "disabled")

    async def list_trust(self) -> tuple[PackTrustRecord, ...]:
        return await self._state_store.list_pack_trust_records()

    async def add_trust(self, value: str) -> PackTrustRecord:
        normalized = value.strip()
        if not normalized:
            raise ColossusError("Trust value is required.")
        if normalized.startswith("key:"):
            record = PackTrustRecord(kind="key", value=normalized.removeprefix("key:"))
        else:
            record = PackTrustRecord(kind="publisher", value=normalized)
        await self._state_store.save_pack_trust_record(record)
        await self._audit_sink.record(
            "user",
            "pack.trust_added",
            {"kind": record.kind, "value": record.value},
        )
        return record

    async def integration_manifests(self) -> tuple[IntegrationManifest, ...]:
        manifests: dict[str, IntegrationManifest] = {}
        for pack in self._bundled_repository.list_packs():
            if pack.status == "enabled":
                for manifest in self._integration_loader.integration_manifests_from_pack(pack):
                    manifests.setdefault(manifest.name, manifest)
        for installed in await self._state_store.list_installed_packs():
            if installed.status != "enabled":
                continue
            pack = LoadedPack(
                manifest=installed.manifest,
                root=Path(installed.installed_path),
                source_kind=installed.source_kind,
                source=installed.source,
                trust_status=installed.trust_status,
                status=installed.status,
            )
            for manifest in self._integration_loader.integration_manifests_from_pack(pack):
                manifests.setdefault(manifest.name, manifest)
        return tuple(sorted(manifests.values(), key=lambda manifest: manifest.name))

    async def _set_status(self, name: str, status: PackInstallStatus) -> InstalledPack:
        normalized = _normalize_name(name)
        installed = await self._state_store.get_installed_pack(normalized)
        if installed is None:
            raise ColossusError(f"Pack is not installed: {name}")
        updated = installed.model_copy(update={"status": status, "updated_at": utc_now_iso()})
        await self._state_store.save_installed_pack(updated)
        if self._marker_writer is not None:
            self._marker_writer(updated)
        await self._audit_sink.record(
            "user",
            f"pack.{status}",
            {"name": updated.name, "version": updated.version},
        )
        return updated

    async def _trust_status(self, manifest: PackManifest) -> PackTrustStatus:
        records = await self._state_store.list_pack_trust_records()
        publishers = {record.value for record in records if record.kind == "publisher"}
        keys = {record.value for record in records if record.kind == "key"}
        if manifest.publisher in publishers:
            return "trusted"
        if any(signature.key_id in keys for signature in manifest.signatures):
            return "trusted"
        return "untrusted"

    def _status_for_loaded(self, pack: LoadedPack) -> PackStatusView:
        integrations = tuple(
            manifest.name
            for manifest in self._integration_loader.integration_manifests_from_pack(pack)
        )
        return PackStatusView(
            name=pack.manifest.name,
            version=pack.manifest.version,
            publisher=pack.manifest.publisher,
            source_kind=pack.source_kind,
            trust_status=pack.trust_status,
            status="enabled",
            capabilities=pack.manifest.capabilities,
            integrations=integrations,
            skills=tuple(ref.path for ref in pack.manifest.skills),
        )

    def _status_for_installed(self, pack: InstalledPack) -> PackStatusView:
        return PackStatusView(
            name=pack.name,
            version=pack.version,
            publisher=pack.manifest.publisher,
            source_kind=pack.source_kind,
            trust_status=pack.trust_status,
            status=pack.status,
            capabilities=pack.manifest.capabilities,
            integrations=tuple(ref.path for ref in pack.manifest.integrations),
            skills=tuple(ref.path for ref in pack.manifest.skills),
        )


def _audit_pack_details(installed: InstalledPack, *, allow_untrusted: bool) -> dict[str, object]:
    return {
        "name": installed.name,
        "version": installed.version,
        "source_kind": installed.source_kind,
        "trust_status": installed.trust_status,
        "status": installed.status,
        "allow_untrusted": allow_untrusted,
        "capabilities": list(installed.manifest.capabilities),
        "permissions": list(installed.manifest.permissions),
    }


def _normalize_name(name: str) -> str:
    normalized = name.strip().lower()
    if not normalized:
        raise ColossusError("Pack name is required.")
    return normalized
