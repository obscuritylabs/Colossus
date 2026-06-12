"""OpenAI Responses API adapter."""

import json
from collections.abc import AsyncIterator
from pathlib import Path

import httpx

from colossus.adapters.model_catalog import extract_model_infos
from colossus.adapters.tool_name_codec import ToolNameCodec
from colossus.domain.events import (
    FinalOutputEvent,
    ModelDeltaEvent,
    RunEvent,
    ToolCallRequestedEvent,
)
from colossus.domain.messages import AssistantMessage, Message, ToolResultMessage, UserMessage
from colossus.domain.providers import (
    ProviderCapability,
    ProviderModelInfo,
    ProviderReadiness,
    ProviderReadinessCheck,
)
from colossus.domain.requests import ModelRequest
from colossus.domain.tools import ToolSpec


class OpenAIResponsesProvider:
    name = "openai-responses"

    def __init__(
        self,
        *,
        api_key: str,
        base_url: str = "https://api.openai.com/v1",
        timeout_seconds: float = 120.0,
        ca_bundle: Path | None = None,
        transport: httpx.AsyncBaseTransport | None = None,
    ) -> None:
        self._api_key = api_key
        self._base_url = base_url.rstrip("/")
        self._timeout_seconds = timeout_seconds
        self._ca_bundle = ca_bundle
        self._transport = transport

    @property
    def base_url(self) -> str:
        return self._base_url

    @property
    def ca_bundle(self) -> Path | None:
        return self._ca_bundle

    def capabilities(self) -> tuple[ProviderCapability, ...]:
        return (
            ProviderCapability(
                name="responses_api",
                supported=True,
                detail="Uses the OpenAI Responses API.",
            ),
            ProviderCapability(
                name="tool_calls",
                supported=True,
                detail="Supports function and custom tool call normalization.",
            ),
        )

    async def check_readiness(self) -> ProviderReadiness:
        if not self._api_key:
            return ProviderReadiness(
                provider=self.name,
                ready=False,
                checks=(
                    ProviderReadinessCheck(
                        name="api_key",
                        status="fail",
                        detail="API key is not configured.",
                    ),
                ),
            )
        try:
            await self.list_models()
        except httpx.HTTPStatusError as exc:
            return ProviderReadiness(
                provider=self.name,
                ready=False,
                checks=(
                    ProviderReadinessCheck(
                        name="models_endpoint",
                        status="fail",
                        detail=f"HTTP {exc.response.status_code} from {self._base_url}/models.",
                    ),
                ),
            )
        except httpx.RequestError as exc:
            return ProviderReadiness(
                provider=self.name,
                ready=False,
                checks=(
                    ProviderReadinessCheck(
                        name="models_endpoint",
                        status="fail",
                        detail=f"Could not reach {self._base_url}/models: {exc}.",
                    ),
                ),
            )
        return ProviderReadiness(
            provider=self.name,
            ready=True,
            checks=(
                ProviderReadinessCheck(
                    name="models_endpoint",
                    status="pass",
                    detail=f"Reached {self._base_url}/models.",
                ),
            ),
        )

    async def list_models(self) -> tuple[ProviderModelInfo, ...]:
        response = await self._get_models_response()
        response.raise_for_status()
        return extract_model_infos(response.json())

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        tool_name_codec = ToolNameCodec.from_tools(request.tools)
        payload = {
            "model": request.model,
            "instructions": request.instructions,
            "input": _messages_to_responses_input(request.messages, tool_name_codec),
            "tools": [
                _tool_to_responses_tool(tool, tool_name_codec) for tool in request.tools
            ],
            "store": False,
        }
        async with httpx.AsyncClient(
            timeout=self._timeout_seconds,
            verify=str(self._ca_bundle) if self._ca_bundle else True,
            transport=self._transport,
        ) as client:
            response = await client.post(
                f"{self._base_url}/responses",
                headers={"Authorization": f"Bearer {self._api_key}"},
                json=payload,
            )
            response.raise_for_status()
            data = response.json()

        output_text: list[str] = []
        for item in data.get("output", []):
            item_type = item.get("type")
            if item_type == "message":
                text = _extract_message_text(item)
                if text:
                    output_text.append(text)
                    yield ModelDeltaEvent(text=text)
            elif item_type == "function_call":
                yield ToolCallRequestedEvent(
                    call_id=str(item["call_id"]),
                    name=tool_name_codec.decode(str(item["name"])),
                    arguments=json.loads(str(item.get("arguments") or "{}")),
                )
            elif item_type == "custom_tool_call":
                yield ToolCallRequestedEvent(
                    call_id=str(item["call_id"]),
                    name=tool_name_codec.decode(str(item["name"])),
                    arguments={"input": str(item.get("input", ""))},
                )
        if output_text:
            yield FinalOutputEvent(text="".join(output_text))

    async def _get_models_response(self) -> httpx.Response:
        async with httpx.AsyncClient(
            timeout=self._timeout_seconds,
            verify=str(self._ca_bundle) if self._ca_bundle else True,
            transport=self._transport,
        ) as client:
            return await client.get(
                f"{self._base_url}/models",
                headers={"Authorization": f"Bearer {self._api_key}"},
            )


def _messages_to_responses_input(
    messages: tuple[Message, ...],
    tool_name_codec: ToolNameCodec,
) -> list[dict[str, object]]:
    items: list[dict[str, object]] = []
    for message in messages:
        items.extend(_message_to_responses_input(message, tool_name_codec))
    return items


def _message_to_responses_input(
    message: Message,
    tool_name_codec: ToolNameCodec,
) -> list[dict[str, object]]:
    if isinstance(message, UserMessage):
        return [{"role": "user", "content": message.content}]
    if isinstance(message, AssistantMessage):
        items: list[dict[str, object]] = []
        if message.content:
            items.append({"role": "assistant", "content": message.content})
        for call in message.tool_calls:
            items.append(
                {
                    "type": "function_call",
                    "call_id": call.call_id,
                    "name": tool_name_codec.encode(call.name),
                    "arguments": json.dumps(call.arguments),
                }
            )
        if items:
            return items
        return [{"role": "assistant", "content": ""}]
    if isinstance(message, ToolResultMessage):
        return [
            {
                "type": "function_call_output",
                "call_id": message.call_id,
                "output": message.content,
            }
        ]
    raise TypeError(f"Unsupported message: {message!r}")


def _tool_to_responses_tool(
    tool: ToolSpec,
    tool_name_codec: ToolNameCodec,
) -> dict[str, object]:
    return {
        "type": "function",
        "name": tool_name_codec.encode(tool.name),
        "description": tool.description,
        "parameters": tool.input_schema,
        "strict": True,
    }


def _extract_message_text(item: dict[str, object]) -> str:
    content = item.get("content")
    if not isinstance(content, list):
        return ""
    chunks: list[str] = []
    for entry in content:
        if isinstance(entry, dict) and entry.get("type") in {"output_text", "text"}:
            text = entry.get("text")
            if isinstance(text, str):
                chunks.append(text)
    return "".join(chunks)
