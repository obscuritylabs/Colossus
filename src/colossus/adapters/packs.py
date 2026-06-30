"""Pack loading, verification, and installation adapters."""

from __future__ import annotations

import hashlib
import json
import shutil
import tarfile
import tempfile
from importlib import resources
from importlib.resources.abc import Traversable
from pathlib import Path
from typing import Any

from colossus.application.skill_loader import load_skill_from_directory
from colossus.domain.errors import ColossusError
from colossus.domain.integrations import IntegrationManifest
from colossus.domain.packs import (
    InstalledPack,
    PackManifest,
    PackSourceKind,
    PackTrustStatus,
    utc_now_iso,
)
from colossus.domain.skills import Skill
from colossus.ports.packs import LoadedPack, PackRepository

PACK_MANIFEST_NAME = "colossus.pack.json"
INSTALLED_PACK_MARKER = ".colossus-installed-pack.json"
PACK_LAYER_MEDIA_TYPES = {
    "application/vnd.colossus.pack.v1.tar",
    "application/vnd.colossus.pack.v1.tar+gzip",
    "application/vnd.oci.image.layer.v1.tar",
    "application/vnd.oci.image.layer.v1.tar+gzip",
}


class PackagePackRepository:
    """Read first-party packs bundled inside the Colossus package."""

    def __init__(self, package: str = "colossus.bundled_packs") -> None:
        self._package = package

    def list_packs(self) -> tuple[LoadedPack, ...]:
        root = resources.files(self._package)
        packs: list[LoadedPack] = []
        for child in root.iterdir():
            manifest_path = child / PACK_MANIFEST_NAME
            if child.is_dir() and manifest_path.is_file():
                manifest = PackManifest.model_validate_json(
                    manifest_path.read_text(encoding="utf-8")
                )
                packs.append(
                    LoadedPack(
                        manifest=manifest,
                        root=child,
                        source_kind="bundled",
                        source=f"package:{child.name}",
                        trust_status="trusted",
                    )
                )
        return tuple(sorted(packs, key=lambda pack: pack.manifest.name))


class InstalledPackRepository:
    """Read installed pack markers from a data-directory pack root."""

    def __init__(self, root: Path) -> None:
        self._root = root

    def list_packs(self) -> tuple[LoadedPack, ...]:
        if not self._root.exists():
            return ()
        packs: list[LoadedPack] = []
        for marker in self._root.glob("*/*/" + INSTALLED_PACK_MARKER):
            installed = InstalledPack.model_validate_json(marker.read_text(encoding="utf-8"))
            packs.append(
                LoadedPack(
                    manifest=installed.manifest,
                    root=marker.parent,
                    source_kind=installed.source_kind,
                    source=installed.source,
                    trust_status=installed.trust_status,
                    status=installed.status,
                )
            )
        return tuple(sorted(packs, key=lambda pack: (pack.manifest.name, pack.manifest.version)))


class PackSkillRepository:
    """Expose skills shipped by enabled bundled or installed packs."""

    def __init__(
        self,
        root: Path | None = None,
        *,
        bundled_repository: PackRepository | None = None,
    ) -> None:
        self._packs = InstalledPackRepository(root) if root is not None else None
        self._bundled_repository = bundled_repository

    def list_skills(self) -> tuple[Skill, ...]:
        skills: list[Skill] = []
        for pack in self._list_packs():
            if pack.status != "enabled":
                continue
            for ref in pack.manifest.skills:
                skill_root = _resource_child(pack.root, ref.path)
                skill = load_skill_from_directory(
                    skill_root,
                    source=f"pack:{pack.manifest.name}:{ref.path}",
                )
                if skill is not None:
                    skills.append(skill)
        return tuple(sorted(skills, key=lambda skill: skill.manifest.name))

    def get_skill(self, name: str) -> Skill | None:
        for skill in self.list_skills():
            if skill.manifest.name == name:
                return skill
        return None

    def _list_packs(self) -> tuple[LoadedPack, ...]:
        packs: list[LoadedPack] = []
        if self._bundled_repository is not None:
            packs.extend(self._bundled_repository.list_packs())
        if self._packs is not None:
            packs.extend(self._packs.list_packs())
        return tuple(packs)


class PackInstaller:
    def __init__(self, install_root: Path) -> None:
        self._install_root = install_root

    def verify_source(self, source: Path) -> tuple[PackManifest, PackSourceKind]:
        with tempfile.TemporaryDirectory(
            prefix="pack-verify-",
            dir=_tmp_parent(self._install_root),
        ) as tmp:
            root, kind = _materialize_source(source, Path(tmp))
            manifest = verify_pack_directory(root)
            return manifest, kind

    def install_source(
        self,
        source: Path,
        *,
        trust_status: PackTrustStatus,
    ) -> InstalledPack:
        self._install_root.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(
            prefix="pack-install-",
            dir=_tmp_parent(self._install_root),
        ) as tmp:
            root, kind = _materialize_source(source, Path(tmp))
            manifest = verify_pack_directory(root)
            destination = self._install_root / manifest.name / manifest.version
            if destination.exists():
                shutil.rmtree(destination)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copytree(root, destination, symlinks=False)
        now = utc_now_iso()
        installed = InstalledPack(
            name=manifest.name,
            version=manifest.version,
            source_kind=kind,
            source=str(source),
            manifest=manifest,
            installed_path=str(destination),
            trust_status=trust_status,
            status="enabled",
            installed_at=now,
            updated_at=now,
        )
        write_installed_pack_marker(installed)
        return installed


def verify_pack_directory(root: Path) -> PackManifest:
    manifest_path = root / PACK_MANIFEST_NAME
    if not manifest_path.is_file():
        raise ColossusError(f"Pack is missing {PACK_MANIFEST_NAME}")
    manifest = PackManifest.model_validate_json(manifest_path.read_text(encoding="utf-8"))
    for file_entry in manifest.files:
        actual_path = _safe_path(root, file_entry.path)
        if actual_path.is_symlink():
            raise ColossusError(f"Pack file must not be a symlink: {file_entry.path}")
        if not actual_path.is_file():
            raise ColossusError(f"Pack file missing: {file_entry.path}")
        if actual_path.stat().st_size != file_entry.size:
            raise ColossusError(f"Pack file size mismatch: {file_entry.path}")
        actual_sha = hashlib.sha256(actual_path.read_bytes()).hexdigest()
        if actual_sha != file_entry.sha256:
            raise ColossusError(f"Pack checksum mismatch: {file_entry.path}")
    _validate_pack_references(root, manifest)
    return manifest


def integration_manifests_from_pack(pack: LoadedPack) -> tuple[IntegrationManifest, ...]:
    manifests: list[IntegrationManifest] = []
    for ref in pack.manifest.integrations:
        path = _resource_child(pack.root, ref.path)
        manifests.append(IntegrationManifest.model_validate_json(path.read_text(encoding="utf-8")))
    return tuple(manifests)


class PackIntegrationManifestLoader:
    def integration_manifests_from_pack(
        self,
        pack: LoadedPack,
    ) -> tuple[IntegrationManifest, ...]:
        return integration_manifests_from_pack(pack)


def installed_pack_from_marker(path: Path) -> InstalledPack:
    return InstalledPack.model_validate_json(path.read_text(encoding="utf-8"))


def write_installed_pack_marker(installed: InstalledPack) -> None:
    path = Path(installed.installed_path)
    path.mkdir(parents=True, exist_ok=True)
    (path / INSTALLED_PACK_MARKER).write_text(installed.model_dump_json(indent=2), encoding="utf-8")


def _materialize_source(source: Path, workspace: Path) -> tuple[Path, PackSourceKind]:
    if not source.exists():
        raise ColossusError(f"Pack source does not exist: {source}")
    if source.is_dir() and (source / PACK_MANIFEST_NAME).is_file():
        return source, "local"
    if source.is_dir() and (source / "oci-layout").is_file() and (source / "index.json").is_file():
        return _extract_oci_layout(source, workspace), "oci"
    raise ColossusError(
        "Pack source must be a pack directory or local OCI layout directory."
    )


def _extract_oci_layout(source: Path, workspace: Path) -> Path:
    index = _json_file(source / "index.json")
    manifests = index.get("manifests")
    if not isinstance(manifests, list) or not manifests:
        raise ColossusError("OCI pack layout index.json must contain manifests.")
    descriptor = _mapping(manifests[0])
    manifest = _json_file(_oci_blob_path(source, _digest(descriptor)))
    layers = manifest.get("layers")
    if not isinstance(layers, list) or not layers:
        raise ColossusError("OCI pack manifest must contain at least one layer.")
    layer = next(
        (
            _mapping(item)
            for item in layers
            if _mapping(item).get("mediaType") in PACK_LAYER_MEDIA_TYPES
        ),
        _mapping(layers[0]),
    )
    layer_path = _oci_blob_path(source, _digest(layer))
    extract_root = workspace / "oci-pack"
    extract_root.mkdir(parents=True, exist_ok=True)
    with tarfile.open(layer_path, mode="r:*") as archive:
        for member in archive.getmembers():
            target = _safe_path(extract_root, member.name)
            if member.isdir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            if not member.isfile():
                raise ColossusError(f"OCI pack layer contains unsupported entry: {member.name}")
            target.parent.mkdir(parents=True, exist_ok=True)
            fileobj = archive.extractfile(member)
            if fileobj is None:
                raise ColossusError(f"OCI pack layer entry cannot be read: {member.name}")
            with target.open("wb") as output:
                shutil.copyfileobj(fileobj, output)
    if (extract_root / PACK_MANIFEST_NAME).is_file():
        return extract_root
    children = [child for child in extract_root.iterdir() if child.is_dir()]
    if len(children) == 1 and (children[0] / PACK_MANIFEST_NAME).is_file():
        return children[0]
    raise ColossusError(f"OCI pack layer is missing {PACK_MANIFEST_NAME}")


def _safe_path(root: Path, rel_path: str) -> Path:
    root_resolved = root.resolve()
    target = (root / rel_path).resolve()
    if target != root_resolved and root_resolved not in target.parents:
        raise ColossusError(f"Pack path escapes the pack root: {rel_path}")
    return target


def _resource_child(root: Path | Traversable, path: str) -> Path | Traversable:
    child: Path | Traversable = root
    for part in Path(path).parts:
        child = child / part
    return child


def _validate_pack_references(root: Path, manifest: PackManifest) -> None:
    declared = {file_entry.path for file_entry in manifest.files}
    for integration_ref in manifest.integrations:
        _require_declared_file(root, integration_ref.path, declared)
    for skill_ref in manifest.skills:
        skill_root = _safe_path(root, skill_ref.path)
        if not skill_root.is_dir():
            raise ColossusError(f"Pack skill missing: {skill_ref.path}")
        skill = load_skill_from_directory(
            skill_root,
            source=f"pack:{manifest.name}:{skill_ref.path}",
        )
        if skill is None:
            raise ColossusError(f"Pack skill is missing SKILL.md: {skill_ref.path}")
        for skill_file in skill_root.rglob("*"):
            if skill_file.is_file():
                rel_path = skill_file.relative_to(root).as_posix()
                if rel_path not in declared:
                    raise ColossusError(f"Pack skill file is not declared: {rel_path}")
    for referenced_path in (*manifest.docs, *manifest.tests, *manifest.docker, *manifest.binaries):
        _require_declared_file(root, referenced_path, declared)
    for tool in manifest.tools:
        if not tool.permissions:
            raise ColossusError(f"Pack tool must declare permissions: {tool.name}")
        _require_declared_command(root, tool.command, declared)
    for server in manifest.mcp_servers:
        _require_declared_command(root, server.command, declared)


def _require_declared_file(root: Path, rel_path: str, declared: set[str]) -> None:
    if rel_path not in declared:
        raise ColossusError(f"Pack referenced file is not declared: {rel_path}")
    path = _safe_path(root, rel_path)
    if not path.is_file():
        raise ColossusError(f"Pack referenced file missing: {rel_path}")


def _require_declared_command(root: Path, command: str, declared: set[str]) -> None:
    command_path = command.removeprefix("./")
    if command.startswith(("/", ".")) or "/" in command:
        _require_declared_file(root, command_path, declared)


def _oci_blob_path(source: Path, digest: str) -> Path:
    algorithm, _, value = digest.partition(":")
    if algorithm != "sha256" or not value:
        raise ColossusError("OCI pack digests must use sha256.")
    return source / "blobs" / "sha256" / value


def _digest(descriptor: dict[str, object]) -> str:
    value = descriptor.get("digest")
    if not isinstance(value, str):
        raise ColossusError("OCI pack descriptor is missing digest.")
    return value


def _json_file(path: Path) -> dict[str, object]:
    if not path.is_file():
        raise ColossusError(f"Pack JSON file is missing: {path}")
    data = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(data, dict):
        raise ColossusError(f"Pack JSON file must contain an object: {path}")
    return data


def _mapping(value: object) -> dict[str, object]:
    return value if isinstance(value, dict) else {}


def _tmp_parent(path: Path) -> str | None:
    path.mkdir(parents=True, exist_ok=True)
    return str(path.parent)


def manifest_file_entry(path: Path, rel_path: str, content_type: str) -> dict[str, Any]:
    actual = path / rel_path
    return {
        "path": rel_path,
        "sha256": hashlib.sha256(actual.read_bytes()).hexdigest(),
        "size": actual.stat().st_size,
        "content_type": content_type,
    }
