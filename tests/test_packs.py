import hashlib
import json
import tarfile
from pathlib import Path

import pytest
from typer.testing import CliRunner

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.packs import (
    PackagePackRepository,
    PackInstaller,
    PackIntegrationManifestLoader,
    PackSkillRepository,
    write_installed_pack_marker,
)
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.application.integrations import IntegrationService
from colossus.application.packs import PackService
from colossus.application.skills import SkillResolver
from colossus.cli import app
from colossus.domain.errors import ColossusError
from colossus.domain.integrations import IntegrationAuthRequirement, IntegrationManifest
from colossus.domain.packs import PackManifest
from colossus.domain.skills import SkillManifest
from colossus.ports.credentials import CredentialMaterial
from colossus.ports.packs import LoadedPack


class EmptyCredentialBroker:
    def resolve(self, credential_ref: str) -> CredentialMaterial:
        raise ColossusError(f"Unexpected credential resolution: {credential_ref}")


def _pack_service(tmp_path: Path) -> PackService:
    return PackService(
        SQLiteStateStore(tmp_path / "state.sqlite3"),
        JsonlAuditSink(tmp_path / "audit.jsonl"),
        PackInstaller(tmp_path / "packs"),
        PackagePackRepository(),
        PackIntegrationManifestLoader(),
        marker_writer=write_installed_pack_marker,
    )


def _integration_service(tmp_path: Path, pack_service: PackService) -> IntegrationService:
    return IntegrationService(
        SQLiteStateStore(tmp_path / "state.sqlite3"),
        JsonlAuditSink(tmp_path / "audit.jsonl"),
        EmptyCredentialBroker(),
        pack_service=pack_service,
    )


def _write_demo_pack(
    root: Path,
    *,
    name: str = "demo-pack",
    publisher: str = "acme",
    include_skill: bool = False,
) -> Path:
    pack_root = root / name
    integration_dir = pack_root / "integrations"
    integration_dir.mkdir(parents=True)
    integration = IntegrationManifest(
        name="demo",
        title="Demo",
        description="Demo pack integration.",
        kind="native",
        auth=IntegrationAuthRequirement(type="none"),
        tools=(),
    )
    integration_path = integration_dir / "demo.json"
    integration_path.write_text(integration.model_dump_json(indent=2), encoding="utf-8")
    files = [_file_entry(pack_root, "integrations/demo.json")]
    skills: list[dict[str, str]] = []
    if include_skill:
        skill_dir = pack_root / "skills" / "demo-skill"
        skill_dir.mkdir(parents=True)
        (skill_dir / "manifest.json").write_text(
            SkillManifest(
                name="demo-skill",
                version="0.1.0",
                description="Demo pack skill.",
                required_tools=(),
            ).model_dump_json(indent=2),
            encoding="utf-8",
        )
        (skill_dir / "SKILL.md").write_text("Use the demo skill.", encoding="utf-8")
        files.append(_file_entry(pack_root, "skills/demo-skill/manifest.json"))
        files.append(_file_entry(pack_root, "skills/demo-skill/SKILL.md"))
        skills.append({"path": "skills/demo-skill"})
    manifest = {
        "format_version": 1,
        "name": name,
        "version": "0.1.0",
        "description": "Demo external pack.",
        "publisher": publisher,
        "license": "Apache-2.0",
        "capabilities": ["integrations", *([] if not include_skill else ["skills"])],
        "permissions": ["network"],
        "files": files,
        "integrations": [{"path": "integrations/demo.json"}],
        "skills": skills,
    }
    (pack_root / "colossus.pack.json").write_text(
        json.dumps(manifest, indent=2) + "\n",
        encoding="utf-8",
    )
    return pack_root


class StaticPackRepository:
    def __init__(self, packs: tuple[LoadedPack, ...]) -> None:
        self._packs = packs

    def list_packs(self) -> tuple[LoadedPack, ...]:
        return self._packs


def _file_entry(root: Path, rel_path: str) -> dict[str, object]:
    path = root / rel_path
    data = path.read_bytes()
    return {
        "path": rel_path,
        "sha256": hashlib.sha256(data).hexdigest(),
        "size": len(data),
        "content_type": "application/json" if path.suffix == ".json" else "text/markdown",
    }


@pytest.mark.asyncio
async def test_bundled_first_party_packs_provide_integrations(tmp_path: Path) -> None:
    pack_service = _pack_service(tmp_path)
    integration_service = _integration_service(tmp_path, pack_service)

    packs = await pack_service.list_statuses()
    statuses = await integration_service.list_statuses()

    assert {"github", "opensearch", "searxng"}.issubset({pack.name for pack in packs})
    assert {"github", "opensearch", "searxng"}.issubset(
        {status.name for status in statuses}
    )


@pytest.mark.asyncio
async def test_unsigned_pack_install_is_blocked_unless_explicitly_allowed(
    tmp_path: Path,
) -> None:
    pack_dir = _write_demo_pack(tmp_path / "source")
    service = _pack_service(tmp_path)

    result = await service.verify(pack_dir)
    with pytest.raises(ColossusError, match="unsigned or untrusted"):
        await service.install(pack_dir)
    installed = await service.install(pack_dir, allow_untrusted=True)

    assert result.trust_status == "untrusted"
    assert installed.trust_status == "untrusted"
    assert (tmp_path / "packs" / "demo-pack" / "0.1.0").is_dir()
    audit_text = (tmp_path / "audit.jsonl").read_text(encoding="utf-8")
    assert "pack.installed" in audit_text
    assert "allow_untrusted" in audit_text


@pytest.mark.asyncio
async def test_trusted_publisher_can_install_pack_without_override(tmp_path: Path) -> None:
    pack_dir = _write_demo_pack(tmp_path / "source", publisher="trusted-pub")
    service = _pack_service(tmp_path)

    await service.add_trust("trusted-pub")
    installed = await service.install(pack_dir)

    assert installed.trust_status == "trusted"


@pytest.mark.asyncio
async def test_pack_integration_is_available_before_connection(tmp_path: Path) -> None:
    pack_dir = _write_demo_pack(tmp_path / "source")
    pack_service = _pack_service(tmp_path)
    await pack_service.install(pack_dir, allow_untrusted=True)
    integration_service = _integration_service(tmp_path, pack_service)

    statuses = await integration_service.list_statuses()

    demo = next(status for status in statuses if status.name == "demo")
    assert demo.status == "available"
    assert demo.tools == ()


@pytest.mark.asyncio
async def test_installed_pack_skills_are_discoverable(tmp_path: Path) -> None:
    pack_dir = _write_demo_pack(tmp_path / "source", include_skill=True)
    service = _pack_service(tmp_path)
    await service.install(pack_dir, allow_untrusted=True)
    resolver = SkillResolver((PackSkillRepository(tmp_path / "packs"),))

    skill = resolver.get_skill("demo-skill")

    assert skill is not None
    assert skill.instructions == "Use the demo skill."
    assert skill.source == "pack:demo-pack:skills/demo-skill"


def test_bundled_pack_skills_are_discoverable(tmp_path: Path) -> None:
    pack_dir = _write_demo_pack(tmp_path / "source", include_skill=True)
    manifest = PackManifest.model_validate_json(
        (pack_dir / "colossus.pack.json").read_text(encoding="utf-8")
    )
    repository = StaticPackRepository(
        (
            LoadedPack(
                manifest=manifest,
                root=pack_dir,
                source_kind="bundled",
                source="package:demo-pack",
                trust_status="trusted",
            ),
        )
    )
    resolver = SkillResolver((PackSkillRepository(bundled_repository=repository),))

    skill = resolver.get_skill("demo-skill")

    assert skill is not None
    assert skill.source == "pack:demo-pack:skills/demo-skill"


@pytest.mark.asyncio
async def test_disabled_installed_pack_skills_are_not_discoverable(tmp_path: Path) -> None:
    pack_dir = _write_demo_pack(tmp_path / "source", include_skill=True)
    service = _pack_service(tmp_path)
    await service.install(pack_dir, allow_untrusted=True)
    await service.disable("demo-pack")
    resolver = SkillResolver((PackSkillRepository(tmp_path / "packs"),))

    assert resolver.get_skill("demo-skill") is None


def test_pack_verification_requires_declared_nested_skill_files(tmp_path: Path) -> None:
    pack_dir = _write_demo_pack(tmp_path / "source", include_skill=True)
    references = pack_dir / "skills" / "demo-skill" / "references"
    references.mkdir()
    (references / "note.md").write_text("extra reference", encoding="utf-8")

    with pytest.raises(ColossusError, match="not declared"):
        PackInstaller(tmp_path / "packs").verify_source(pack_dir)


def test_pack_verification_requires_binary_refs_to_be_hash_listed(tmp_path: Path) -> None:
    pack_dir = _write_demo_pack(tmp_path / "source")
    manifest = json.loads((pack_dir / "colossus.pack.json").read_text(encoding="utf-8"))
    manifest["capabilities"].append("binaries")
    manifest["binaries"] = ["bin/demo"]
    (pack_dir / "colossus.pack.json").write_text(
        json.dumps(manifest, indent=2) + "\n",
        encoding="utf-8",
    )

    with pytest.raises(ColossusError, match="referenced file is not declared"):
        PackInstaller(tmp_path / "packs").verify_source(pack_dir)


def test_pack_verification_requires_executable_tool_permissions(tmp_path: Path) -> None:
    pack_dir = _write_demo_pack(tmp_path / "source")
    bin_dir = pack_dir / "bin"
    bin_dir.mkdir()
    (bin_dir / "demo").write_text("#!/bin/sh\n", encoding="utf-8")
    manifest = json.loads((pack_dir / "colossus.pack.json").read_text(encoding="utf-8"))
    manifest["capabilities"].append("tools")
    manifest["files"].append(_file_entry(pack_dir, "bin/demo"))
    manifest["tools"] = [{"name": "demo.tool", "command": "bin/demo"}]
    (pack_dir / "colossus.pack.json").write_text(
        json.dumps(manifest, indent=2) + "\n",
        encoding="utf-8",
    )

    with pytest.raises(ColossusError, match="must declare permissions"):
        PackInstaller(tmp_path / "packs").verify_source(pack_dir)


@pytest.mark.asyncio
async def test_local_oci_layout_pack_verifies(tmp_path: Path) -> None:
    pack_dir = _write_demo_pack(tmp_path / "source")
    oci_dir = _write_oci_layout(tmp_path / "oci", pack_dir)
    service = _pack_service(tmp_path)

    result = await service.verify(oci_dir)

    assert result.name == "demo-pack"
    assert result.source_kind == "oci"


def _write_oci_layout(root: Path, pack_dir: Path) -> Path:
    blobs = root / "blobs" / "sha256"
    blobs.mkdir(parents=True)
    layer_path = root / "layer.tar"
    with tarfile.open(layer_path, "w") as archive:
        for path in pack_dir.rglob("*"):
            if path.is_file():
                archive.add(path, arcname=str(path.relative_to(pack_dir)))
    layer_data = layer_path.read_bytes()
    layer_digest = hashlib.sha256(layer_data).hexdigest()
    (blobs / layer_digest).write_bytes(layer_data)
    manifest = {
        "schemaVersion": 2,
        "layers": [
            {
                "mediaType": "application/vnd.colossus.pack.v1.tar",
                "digest": f"sha256:{layer_digest}",
                "size": len(layer_data),
            }
        ],
    }
    manifest_data = json.dumps(manifest).encode()
    manifest_digest = hashlib.sha256(manifest_data).hexdigest()
    (blobs / manifest_digest).write_bytes(manifest_data)
    (root / "oci-layout").write_text('{"imageLayoutVersion":"1.0.0"}\n', encoding="utf-8")
    (root / "index.json").write_text(
        json.dumps(
            {
                "schemaVersion": 2,
                "manifests": [
                    {
                        "mediaType": "application/vnd.oci.image.manifest.v1+json",
                        "digest": f"sha256:{manifest_digest}",
                        "size": len(manifest_data),
                    }
                ],
            }
        )
        + "\n",
        encoding="utf-8",
    )
    layer_path.unlink()
    return root


def test_cli_pack_list_install_and_show(tmp_path: Path) -> None:
    source = _write_demo_pack(tmp_path / "source")
    runner = CliRunner()

    listed = runner.invoke(app, ["packs", "list"])
    blocked = runner.invoke(app, ["packs", "install", str(source)])
    validated = runner.invoke(app, ["packs", "validate", str(source)])
    installed = runner.invoke(
        app,
        ["packs", "install", str(source), "--allow-untrusted"],
    )
    shown = runner.invoke(app, ["packs", "show", "demo-pack"])

    assert listed.exit_code == 0
    assert "github" in listed.stdout
    assert blocked.exit_code == 1
    assert "unsigned or untrusted" in blocked.stdout
    assert validated.exit_code == 0
    assert "Pack is valid" in validated.stdout
    assert installed.exit_code == 0
    assert "Installed pack demo-pack" in installed.stdout
    assert shown.exit_code == 0
    assert "demo-pack" in shown.stdout
