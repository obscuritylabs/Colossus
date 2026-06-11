"""Provider-neutral conversation messages."""

from typing import Literal

from pydantic import BaseModel, ConfigDict, Field

from colossus.domain.tools import ToolCall


class UserMessage(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    role: Literal["user"] = "user"
    content: str


class AssistantMessage(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    role: Literal["assistant"] = "assistant"
    content: str
    tool_calls: tuple[ToolCall, ...] = Field(default_factory=tuple)


class ToolResultMessage(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    role: Literal["tool"] = "tool"
    call_id: str
    name: str
    content: str


Message = UserMessage | AssistantMessage | ToolResultMessage


class ConversationTurn(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    messages: tuple[Message, ...] = Field(default_factory=tuple)
