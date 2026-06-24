"""Application service for provider diagnostics."""

from colossus.domain.events import ToolCallRequestedEvent
from colossus.domain.messages import UserMessage
from colossus.domain.providers import (
    ProviderCapability,
    ProviderModelInfo,
    ProviderReadiness,
    ProviderReadinessCheck,
)
from colossus.domain.requests import ModelRequest
from colossus.domain.tools import ToolPermission, ToolSpec
from colossus.ports.model_provider import ModelProvider


class ProviderDiagnostics:
    """Thin application wrapper around provider metadata and readiness probes."""

    def __init__(self, provider: ModelProvider) -> None:
        self._provider = provider

    def capabilities(self) -> tuple[ProviderCapability, ...]:
        return self._provider.capabilities()

    async def check_readiness(self) -> ProviderReadiness:
        return await self._provider.check_readiness()

    async def list_models(self) -> tuple[ProviderModelInfo, ...]:
        return await self._provider.list_models()

    async def probe_tool_calls(self, model: str) -> ProviderReadinessCheck:
        """Check whether this specific model emits normalized tool-call events."""
        probe_tool = ToolSpec(
            name="colossus_tool_probe",
            description="Probe whether a model can request a structured tool call.",
            input_schema={
                "type": "object",
                "properties": {"token": {"type": "string"}},
                "required": ["token"],
                "additionalProperties": False,
            },
            permissions=ToolPermission(
                filesystem="none",
                network="deny",
                approval_required=False,
                mutation=False,
                working_root_required=False,
                risk="low",
            ),
        )
        request = ModelRequest(
            model=model,
            instructions=(
                "You are checking tool-call support. Request the tool "
                "colossus_tool_probe with token set to probe-ok. Do not answer in text."
            ),
            messages=(
                UserMessage(
                    content=(
                        "Call colossus_tool_probe now with this JSON argument: "
                        '{"token":"probe-ok"}.'
                    )
                ),
            ),
            tools=(probe_tool,),
        )
        try:
            unexpected_tool: str | None = None
            unexpected_arguments = False
            matched = False
            async for event in self._provider.stream(request):
                if not isinstance(event, ToolCallRequestedEvent):
                    continue
                if event.name != "colossus_tool_probe":
                    unexpected_tool = event.name
                    continue
                if event.arguments.get("token") == "probe-ok":
                    matched = True
                    continue
                unexpected_arguments = True
        except Exception as exc:
            return ProviderReadinessCheck(
                name="model_tool_calls",
                status="fail",
                detail=f"Tool-call probe failed for {model}: {exc}",
            )
        if unexpected_tool is not None:
            return ProviderReadinessCheck(
                name="model_tool_calls",
                status="fail",
                detail=f"Model requested unexpected tool {unexpected_tool!r}.",
            )
        if matched:
            return ProviderReadinessCheck(
                name="model_tool_calls",
                status="pass",
                detail=f"Model {model} emitted a structured tool call.",
            )
        if unexpected_arguments:
            return ProviderReadinessCheck(
                name="model_tool_calls",
                status="fail",
                detail="Model emitted a tool call with unexpected arguments.",
            )
        return ProviderReadinessCheck(
            name="model_tool_calls",
            status="fail",
            detail=(
                f"Model {model} answered without a structured tool_call. "
                "It may still be useful for chat, but Colossus cannot execute tools from text."
            ),
        )
