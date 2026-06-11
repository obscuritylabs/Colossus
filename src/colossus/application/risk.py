"""Model-assisted tool risk assessment."""

import json
import re
from typing import Any

from pydantic import ValidationError

from colossus.application.model_router import ModelRouter
from colossus.domain.events import FinalOutputEvent, ModelDeltaEvent
from colossus.domain.messages import UserMessage
from colossus.domain.policy import PolicyDecision
from colossus.domain.requests import ModelRequest
from colossus.domain.risk import RiskAssessment
from colossus.domain.tools import ToolCall, ToolSpec

JsonObject = dict[str, Any]

RISK_INSTRUCTIONS = (
    "You are Colossus command-risk evaluator. Review the proposed tool call for "
    "operational and security risk. Return only compact JSON with keys: "
    "risk_level, summary, concerns, recommended_decision. recommended_decision must be "
    "one of allow, requires_approval, deny. Do not include markdown."
)


class RiskAssessmentService:
    def __init__(
        self,
        router: ModelRouter,
        *,
        role: str = "risk_evaluator",
        max_payload_chars: int = 4_000,
    ) -> None:
        self._router = router
        self._role = role
        self._max_payload_chars = max_payload_chars

    async def assess_tool_call(
        self,
        spec: ToolSpec,
        call: ToolCall,
        deterministic_decision: PolicyDecision,
    ) -> RiskAssessment | None:
        route = self._router.resolve(self._role)
        prompt = json.dumps(
            {
                "tool": call.name,
                "arguments": _redact(call.arguments),
                "permissions": spec.permissions.model_dump(mode="json"),
                "timeout_seconds": spec.timeout_seconds,
                "max_output_bytes": spec.max_output_bytes,
                "deterministic_policy": deterministic_decision.model_dump(mode="json"),
            },
            sort_keys=True,
        )
        prompt = _truncate(prompt, self._max_payload_chars)
        try:
            chunks: list[str] = []
            async for event in self._router.stream(
                self._role,
                ModelRequest(
                    model=route.profile.model,
                    instructions=RISK_INSTRUCTIONS,
                    messages=(UserMessage(content=prompt),),
                    tools=(),
                ),
            ):
                if isinstance(event, ModelDeltaEvent) or (
                    isinstance(event, FinalOutputEvent) and not chunks
                ):
                    chunks.append(event.text)
            payload = _extract_json("".join(chunks))
            assessment = RiskAssessment.model_validate(
                {
                    **payload,
                    "tool": call.name,
                    "model_role": route.role,
                    "profile_name": route.profile_name,
                }
            )
        except (json.JSONDecodeError, KeyError, TypeError, ValueError, ValidationError):
            return None
        return assessment


def _extract_json(value: str) -> JsonObject:
    stripped = value.strip()
    if stripped.startswith("```"):
        stripped = re.sub(r"^```(?:json)?\s*", "", stripped)
        stripped = re.sub(r"\s*```$", "", stripped)
    parsed = json.loads(stripped)
    if not isinstance(parsed, dict):
        raise TypeError("Risk assessment response must be a JSON object.")
    return parsed


def _redact(value: object) -> object:
    if isinstance(value, dict):
        return {str(key): _redact_mapping_value(str(key), item) for key, item in value.items()}
    if isinstance(value, list):
        return [_redact(item) for item in value]
    if isinstance(value, tuple):
        return tuple(_redact(item) for item in value)
    if isinstance(value, str):
        return _redact_string(value)
    return value


def _redact_mapping_value(key: str, value: object) -> object:
    if _SENSITIVE_KEY_PATTERN.search(key):
        return "[REDACTED]"
    return _redact(value)


def _redact_string(value: str) -> str:
    return _SENSITIVE_INLINE_PATTERN.sub(r"\1=[REDACTED]", value)


def _truncate(value: str, limit: int) -> str:
    if len(value) <= limit:
        return value
    return f"{value[: max(0, limit - 3)]}..."


_SENSITIVE_KEY_PATTERN = re.compile(r"(api[_-]?key|token|secret|password|credential)", re.I)
_SENSITIVE_INLINE_PATTERN = re.compile(
    r"\b(api[_-]?key|token|secret|password|credential)=([^\s]+)",
    re.I,
)
