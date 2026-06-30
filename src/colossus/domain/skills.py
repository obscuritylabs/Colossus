"""Skill manifests and loaded skills."""

from pydantic import BaseModel, ConfigDict, Field


class SkillManifest(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    name: str
    version: str
    description: str
    triggers: tuple[str, ...] = Field(default_factory=tuple)
    required_tools: tuple[str, ...] = Field(default_factory=tuple)
    permissions: tuple[str, ...] = Field(default_factory=tuple)
    offline_compatible: bool = True


class Skill(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    manifest: SkillManifest
    instructions: str
    source: str
    resource_root: str | None = None
