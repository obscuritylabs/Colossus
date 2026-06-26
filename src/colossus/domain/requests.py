"""Requests and results for agent runs."""

from pydantic import BaseModel, ConfigDict, Field

from colossus.domain.agents import AgentSpec
from colossus.domain.messages import Message
from colossus.domain.tools import ToolSpec


class ModelRequest(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    model: str
    instructions: str
    messages: tuple[Message, ...]
    tools: tuple[ToolSpec, ...] = Field(default_factory=tuple)


class AgentRunRequest(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    prompt: str
    agent: AgentSpec
    session_id: str | None = None
    plan_id: str | None = None
    skill_mode_enabled: bool = True
    active_skills: tuple[str, ...] = Field(default_factory=tuple)


class AgentRunResult(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    run_id: str
    final_output: str
    events_recorded: int
    session_id: str | None = None
