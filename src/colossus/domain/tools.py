"""Tool specifications and execution results."""

from typing import Literal

from pydantic import BaseModel, ConfigDict, Field


class ToolPermission(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    filesystem: Literal["none", "read", "write"] = "none"
    network: Literal["deny", "allow"] = "deny"
    approval_required: bool = False
    mutation: bool = False
    working_root_required: bool = True
    risk: Literal["low", "medium", "high"] = "low"


class ToolSpec(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    name: str
    description: str
    input_schema: dict[str, object]
    output_schema: dict[str, object] | None = None
    permissions: ToolPermission = Field(default_factory=ToolPermission)
    timeout_seconds: float = 30.0
    max_output_bytes: int = 32_768


class ToolCall(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    call_id: str
    name: str
    arguments: dict[str, object] = Field(default_factory=dict)


class ToolResult(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    call_id: str
    name: str
    output: str
    exit_code: int = 0
