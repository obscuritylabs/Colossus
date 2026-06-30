"""Agent specifications."""

from pydantic import BaseModel, ConfigDict, Field

DEFAULT_AGENT_MAX_TURNS = 24
MAX_AGENT_MAX_TURNS = 100


class AgentSpec(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    name: str
    instructions: str
    model: str = "default"
    tools: tuple[str, ...] = Field(default_factory=tuple)
    skills: tuple[str, ...] = Field(default_factory=tuple)
    subagents: tuple[str, ...] = Field(default_factory=tuple)
    max_turns: int = Field(default=DEFAULT_AGENT_MAX_TURNS, ge=1, le=MAX_AGENT_MAX_TURNS)
