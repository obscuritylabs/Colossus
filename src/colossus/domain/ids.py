"""Strong identifiers used across the harness."""

from typing import NewType

AgentName = NewType("AgentName", str)
RunId = NewType("RunId", str)
SkillName = NewType("SkillName", str)
ToolCallId = NewType("ToolCallId", str)
ToolName = NewType("ToolName", str)
