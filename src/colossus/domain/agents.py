"""Agent specifications."""

from pydantic import BaseModel, ConfigDict, Field


class AgentSpec(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    name: str
    instructions: str
    model: str = "default"
    tools: tuple[str, ...] = Field(default_factory=tuple)
    skills: tuple[str, ...] = Field(default_factory=tuple)
    subagents: tuple[str, ...] = Field(default_factory=tuple)
    max_turns: int = 8
