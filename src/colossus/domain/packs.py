"""Pack manifests and installed pack records."""

from datetime import UTC, datetime
from pathlib import PurePosixPath
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator

PackCapability = Literal[
    "integrations",
    "skills",
    "tools",
    "binaries",
    "policies",
    "docs",
    "tests",
    "docker",
    "mcp_servers",
]
PackSourceKind = Literal["bundled", "local", "oci"]
PackTrustStatus = Literal["trusted", "untrusted"]
PackInstallStatus = Literal["enabled", "disabled"]
PackTrustKind = Literal["publisher", "key"]


def utc_now_iso() -> str:
    return datetime.now(UTC).isoformat()


class PackFileEntry(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    path: str
    sha256: str
    size: int = Field(ge=0)
    content_type: str = "application/octet-stream"

    @field_validator("path")
    @classmethod
    def _validate_path(cls, value: str) -> str:
        return _validate_relative_path(value)

    @field_validator("sha256")
    @classmethod
    def _validate_sha256(cls, value: str) -> str:
        normalized = value.strip().lower()
        if len(normalized) != 64 or any(ch not in "0123456789abcdef" for ch in normalized):
            raise ValueError("sha256 must be a 64-character lowercase hex digest")
        return normalized


class PackIntegrationRef(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    path: str

    @field_validator("path")
    @classmethod
    def _validate_path(cls, value: str) -> str:
        return _validate_relative_path(value)


class PackSkillRef(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    path: str

    @field_validator("path")
    @classmethod
    def _validate_path(cls, value: str) -> str:
        return _validate_relative_path(value)


class PackMcpServerRef(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    name: str
    command: str
    args: tuple[str, ...] = Field(default_factory=tuple)
    env_refs: dict[str, str] = Field(default_factory=dict)
    allowed_tools: tuple[str, ...] = Field(default_factory=tuple)

    @field_validator("name")
    @classmethod
    def _validate_name(cls, value: str) -> str:
        normalized = value.strip()
        if not normalized:
            raise ValueError("MCP server name is required")
        return normalized


class PackToolRef(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    name: str
    command: str
    args: tuple[str, ...] = Field(default_factory=tuple)
    env_refs: dict[str, str] = Field(default_factory=dict)
    permissions: tuple[str, ...] = Field(default_factory=tuple)

    @field_validator("name")
    @classmethod
    def _validate_name(cls, value: str) -> str:
        normalized = value.strip()
        if not normalized:
            raise ValueError("tool name is required")
        return normalized


class PackSignature(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    algorithm: Literal["ed25519"] = "ed25519"
    key_id: str
    signature: str


class PackManifest(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    format_version: int = 1
    name: str
    version: str
    description: str
    publisher: str
    license: str = ""
    homepage: str = ""
    capabilities: tuple[PackCapability, ...] = Field(default_factory=tuple)
    permissions: tuple[str, ...] = Field(default_factory=tuple)
    files: tuple[PackFileEntry, ...] = Field(default_factory=tuple)
    signatures: tuple[PackSignature, ...] = Field(default_factory=tuple)
    integrations: tuple[PackIntegrationRef, ...] = Field(default_factory=tuple)
    skills: tuple[PackSkillRef, ...] = Field(default_factory=tuple)
    tools: tuple[PackToolRef, ...] = Field(default_factory=tuple)
    mcp_servers: tuple[PackMcpServerRef, ...] = Field(default_factory=tuple)
    docs: tuple[str, ...] = Field(default_factory=tuple)
    tests: tuple[str, ...] = Field(default_factory=tuple)
    docker: tuple[str, ...] = Field(default_factory=tuple)
    binaries: tuple[str, ...] = Field(default_factory=tuple)
    dependencies: tuple[str, ...] = Field(default_factory=tuple)

    @field_validator("name")
    @classmethod
    def _validate_name(cls, value: str) -> str:
        normalized = value.strip().lower()
        if not normalized:
            raise ValueError("pack name is required")
        if any(ch.isspace() for ch in normalized):
            raise ValueError("pack name cannot contain whitespace")
        return normalized

    @field_validator("docs", "tests", "docker", "binaries")
    @classmethod
    def _validate_paths(cls, values: tuple[str, ...]) -> tuple[str, ...]:
        return tuple(_validate_relative_path(value) for value in values)


class InstalledPack(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    name: str
    version: str
    source_kind: PackSourceKind
    source: str
    manifest: PackManifest
    installed_path: str
    trust_status: PackTrustStatus
    status: PackInstallStatus = "enabled"
    installed_at: str = Field(default_factory=utc_now_iso)
    updated_at: str = Field(default_factory=utc_now_iso)


class PackStatusView(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    name: str
    version: str
    publisher: str
    source_kind: PackSourceKind
    trust_status: PackTrustStatus
    status: PackInstallStatus
    capabilities: tuple[PackCapability, ...] = Field(default_factory=tuple)
    integrations: tuple[str, ...] = Field(default_factory=tuple)
    skills: tuple[str, ...] = Field(default_factory=tuple)


class PackTrustRecord(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    kind: PackTrustKind
    value: str
    added_at: str = Field(default_factory=utc_now_iso)


class PackVerificationResult(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    name: str
    version: str
    source_kind: PackSourceKind
    trust_status: PackTrustStatus
    file_count: int
    capabilities: tuple[PackCapability, ...] = Field(default_factory=tuple)
    permissions: tuple[str, ...] = Field(default_factory=tuple)


def _validate_relative_path(value: str) -> str:
    normalized = value.strip()
    if not normalized:
        raise ValueError("path is required")
    path = PurePosixPath(normalized)
    if path.is_absolute() or ".." in path.parts:
        raise ValueError("path must be relative and stay inside the pack")
    return path.as_posix()
