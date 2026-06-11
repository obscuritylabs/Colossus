"""Typed user preference models."""

from typing import Literal

from pydantic import BaseModel, ConfigDict

EventDisplayPreference = Literal["compact", "verbose", "off"]
TranscriptStylePreference = Literal["comfortable", "compact"]


class ReplPreferences(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    theme: str = "default"
    multiline: bool = False
    stream_model_output: bool = True
    events_mode: EventDisplayPreference = "compact"
    show_reasoning: bool = True
    transcript_style: TranscriptStylePreference = "comfortable"
