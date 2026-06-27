"""Integration manifests, connections, and auth metadata."""

from datetime import UTC, datetime
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field, field_validator

from colossus.domain.tools import ToolPermission

IntegrationKind = Literal["native", "openapi", "mcp"]
IntegrationAuthType = Literal[
    "none",
    "api_key",
    "bearer",
    "oauth2_authorization_code",
    "service_account",
]
IntegrationConnectionStatus = Literal["connected", "pending_auth", "disconnected"]


def utc_now_iso() -> str:
    return datetime.now(UTC).isoformat()


class IntegrationAuthRequirement(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    type: IntegrationAuthType = "none"
    scopes: tuple[str, ...] = Field(default_factory=tuple)
    header: str = "Authorization"
    scheme: str | None = "Bearer"


class IntegrationToolManifest(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    name: str
    description: str
    input_schema: dict[str, object]
    output_schema: dict[str, object] | None = None
    permissions: ToolPermission = Field(default_factory=ToolPermission)
    timeout_seconds: float = 30.0
    max_output_bytes: int = 32_768
    operation_id: str | None = None
    method: str | None = None
    path: str | None = None

    @field_validator("name")
    @classmethod
    def _validate_name(cls, value: str) -> str:
        if not value.strip():
            raise ValueError("integration tool name is required")
        return value.strip()


class IntegrationManifest(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    name: str
    title: str
    description: str
    kind: IntegrationKind
    auth: IntegrationAuthRequirement = Field(default_factory=IntegrationAuthRequirement)
    tools: tuple[IntegrationToolManifest, ...] = Field(default_factory=tuple)
    metadata: dict[str, object] = Field(default_factory=dict)

    @field_validator("name")
    @classmethod
    def _validate_name(cls, value: str) -> str:
        normalized = value.strip().lower()
        if not normalized:
            raise ValueError("integration name is required")
        if any(character.isspace() for character in normalized):
            raise ValueError("integration name cannot contain whitespace")
        return normalized


class IntegrationConnection(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    name: str
    kind: IntegrationKind
    status: IntegrationConnectionStatus
    credential_ref: str | None = None
    scopes: tuple[str, ...] = Field(default_factory=tuple)
    manifest: IntegrationManifest
    config: dict[str, object] = Field(default_factory=dict)
    connected_at: str = Field(default_factory=utc_now_iso)
    updated_at: str = Field(default_factory=utc_now_iso)


class IntegrationStatusView(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    name: str
    title: str
    kind: IntegrationKind
    status: IntegrationConnectionStatus | Literal["available"]
    auth_type: IntegrationAuthType
    credential_ref: str | None = None
    scopes: tuple[str, ...] = Field(default_factory=tuple)
    tools: tuple[str, ...] = Field(default_factory=tuple)
