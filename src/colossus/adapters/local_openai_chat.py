"""Local OpenAI-compatible chat-completions adapter."""

import json
from collections.abc import AsyncIterator
from dataclasses import dataclass, field
from pathlib import Path

import httpx

from colossus.domain.errors import ProviderError
from colossus.domain.events import (
    FinalOutputEvent,
    ModelDeltaEvent,
    ReasoningSummaryEvent,
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


class LocalOpenAIChatProvider:
    name = "local-openai-chat"

    def __init__(
        self,
        *,
        base_url: str,
        api_key: str = "local",
        timeout_seconds: float = 120.0,
        ca_bundle: Path | None = None,
        transport: httpx.AsyncBaseTransport | None = None,
    ) -> None:
        self._base_url = base_url.rstrip("/")
        self._api_key = api_key
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
                name="chat_completions",
                supported=True,
                detail="Uses an OpenAI-compatible chat completions endpoint.",
            ),
            ProviderCapability(
                name="tool_calls",
                supported=True,
                detail="Supports OpenAI-compatible function tool calls.",
            ),
        )

    async def check_readiness(self) -> ProviderReadiness:
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
        return _extract_model_infos(response.json())

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        messages: list[dict[str, object]] = [
            {"role": "system", "content": request.instructions},
            *[_message_to_chat_message(message) for message in request.messages],
        ]
        payload: dict[str, object] = {
            "model": request.model,
            "messages": messages,
            "tools": [_tool_to_chat_tool(tool) for tool in request.tools],
        }
        async with httpx.AsyncClient(
            timeout=self._timeout_seconds,
            verify=str(self._ca_bundle) if self._ca_bundle else True,
            transport=self._transport,
        ) as client:
            try:
                async for event in self._stream_chat_completion(client, payload):
                    yield event
                return
            except _StreamingFallbackRequired:
                async for event in self._non_stream_chat_completion(client, payload):
                    yield event

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

    async def _stream_chat_completion(
        self,
        client: httpx.AsyncClient,
        payload: dict[str, object],
    ) -> AsyncIterator[RunEvent]:
        streamed_payload = {**payload, "stream": True}
        async with client.stream(
            "POST",
            f"{self._base_url}/chat/completions",
            headers={"Authorization": f"Bearer {self._api_key}"},
            json=streamed_payload,
        ) as response:
            try:
                response.raise_for_status()
            except httpx.HTTPStatusError:
                raise _StreamingFallbackRequired from None
            content_type = response.headers.get("content-type", "")
            if "text/event-stream" not in content_type:
                body = await response.aread()
                async for event in _events_from_chat_completion(json.loads(body.decode())):
                    yield event
                return

            output_text: list[str] = []
            tool_calls: dict[int, _StreamToolCall] = {}
            async for item in _stream_json_items(response):
                async for event in _events_from_stream_chunk(item, tool_calls, output_text):
                    yield event

            for event in _tool_call_events(tool_calls):
                yield event
            if output_text:
                yield FinalOutputEvent(text="".join(output_text))

    async def _non_stream_chat_completion(
        self,
        client: httpx.AsyncClient,
        payload: dict[str, object],
    ) -> AsyncIterator[RunEvent]:
        response = await client.post(
            f"{self._base_url}/chat/completions",
            headers={"Authorization": f"Bearer {self._api_key}"},
            json=payload,
        )
        try:
            response.raise_for_status()
        except httpx.HTTPStatusError as exc:
            raise ProviderError(_http_error_detail(exc)) from exc
        async for event in _events_from_chat_completion(response.json()):
            yield event


def _message_to_chat_message(message: Message) -> dict[str, object]:
    if isinstance(message, UserMessage):
        return {"role": "user", "content": message.content}
    if isinstance(message, AssistantMessage):
        if message.tool_calls:
            return {
                "role": "assistant",
                "content": message.content or None,
                "tool_calls": [
                    {
                        "id": call.call_id,
                        "type": "function",
                        "function": {
                            "name": call.name,
                            "arguments": json.dumps(call.arguments),
                        },
                    }
                    for call in message.tool_calls
                ],
            }
        return {"role": "assistant", "content": message.content}
    if isinstance(message, ToolResultMessage):
        return {"role": "tool", "tool_call_id": message.call_id, "content": message.content}
    raise TypeError(f"Unsupported message: {message!r}")


def _tool_to_chat_tool(tool: ToolSpec) -> dict[str, object]:
    return {
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.input_schema,
        },
    }


async def _events_from_chat_completion(data: object) -> AsyncIterator[RunEvent]:
    if not isinstance(data, dict):
        return
    choices = data.get("choices")
    if not isinstance(choices, list) or not choices:
        return
    first = choices[0]
    if not isinstance(first, dict):
        return
    message = first.get("message")
    if not isinstance(message, dict):
        return
    for event in _reasoning_summary_events(message.get("reasoning_details")):
        yield event
    for tool_call in message.get("tool_calls", []) or []:
        if not isinstance(tool_call, dict):
            continue
        function = tool_call.get("function")
        if not isinstance(function, dict):
            continue
        yield ToolCallRequestedEvent(
            call_id=str(tool_call.get("id", "")),
            name=str(function.get("name", "")),
            arguments=json.loads(str(function.get("arguments") or "{}")),
        )
    content = message.get("content")
    if isinstance(content, str) and content:
        yield ModelDeltaEvent(text=content)
        yield FinalOutputEvent(text=content)


async def _stream_json_items(response: httpx.Response) -> AsyncIterator[dict[str, object]]:
    data_lines: list[str] = []
    async for line in response.aiter_lines():
        if not line:
            if data_lines:
                payload = "\n".join(data_lines)
                data_lines.clear()
                if payload == "[DONE]":
                    return
                parsed = json.loads(payload)
                if isinstance(parsed, dict):
                    yield parsed
            continue
        if line.startswith(":"):
            continue
        if line.startswith("data:"):
            data_lines.append(line.removeprefix("data:").strip())
    if data_lines:
        payload = "\n".join(data_lines)
        if payload != "[DONE]":
            parsed = json.loads(payload)
            if isinstance(parsed, dict):
                yield parsed


async def _events_from_stream_chunk(
    item: dict[str, object],
    tool_calls: dict[int, "_StreamToolCall"],
    output_text: list[str],
) -> AsyncIterator[RunEvent]:
    choices = item.get("choices")
    if not isinstance(choices, list):
        return
    for choice in choices:
        if not isinstance(choice, dict):
            continue
        delta = choice.get("delta")
        if not isinstance(delta, dict):
            continue
        for event in _reasoning_summary_events(delta.get("reasoning_details")):
            yield event
        content = delta.get("content")
        if isinstance(content, str) and content:
            output_text.append(content)
            yield ModelDeltaEvent(text=content)
        _accumulate_tool_calls(delta.get("tool_calls"), tool_calls)


def _accumulate_tool_calls(value: object, tool_calls: dict[int, "_StreamToolCall"]) -> None:
    if not isinstance(value, list):
        return
    for item in value:
        if not isinstance(item, dict):
            continue
        index_value = item.get("index")
        index = index_value if isinstance(index_value, int) else len(tool_calls)
        state = tool_calls.setdefault(index, _StreamToolCall())
        call_id = item.get("id")
        if isinstance(call_id, str) and call_id:
            state.call_id = call_id
        function = item.get("function")
        if not isinstance(function, dict):
            continue
        name = function.get("name")
        if isinstance(name, str) and name:
            state.name = name
        arguments = function.get("arguments")
        if isinstance(arguments, str) and arguments:
            state.arguments.append(arguments)


def _tool_call_events(
    tool_calls: dict[int, "_StreamToolCall"],
) -> tuple[ToolCallRequestedEvent, ...]:
    events: list[ToolCallRequestedEvent] = []
    for index in sorted(tool_calls):
        call = tool_calls[index]
        if not call.call_id or not call.name:
            continue
        arguments_text = "".join(call.arguments) or "{}"
        events.append(
            ToolCallRequestedEvent(
                call_id=call.call_id,
                name=call.name,
                arguments=json.loads(arguments_text),
            )
        )
    return tuple(events)


def _reasoning_summary_events(value: object) -> tuple[ReasoningSummaryEvent, ...]:
    if not isinstance(value, list):
        return ()
    events: list[ReasoningSummaryEvent] = []
    for item in value:
        if not isinstance(item, dict) or item.get("type") != "reasoning.summary":
            continue
        summary = item.get("summary")
        if not isinstance(summary, str) or not summary:
            continue
        provider_format = item.get("format")
        detail_id = item.get("id")
        events.append(
            ReasoningSummaryEvent(
                summary=summary,
                provider_format=provider_format if isinstance(provider_format, str) else None,
                detail_id=detail_id if isinstance(detail_id, str) else None,
            )
        )
    return tuple(events)


@dataclass
class _StreamToolCall:
    call_id: str = ""
    name: str = ""
    arguments: list[str] = field(default_factory=list)


class _StreamingFallbackRequired(Exception):
    """Raised when a streaming attempt should be retried without streaming."""


def _http_error_detail(exc: httpx.HTTPStatusError) -> str:
    response_text = exc.response.text.strip()
    suffix = f": {response_text[:500]}" if response_text else ""
    return (
        f"{exc.response.status_code} from "
        f"{exc.request.url}{suffix}"
    )


def _extract_model_infos(data: object) -> tuple[ProviderModelInfo, ...]:
    if not isinstance(data, dict):
        return ()
    entries = data.get("data")
    if not isinstance(entries, list):
        return ()
    models: list[ProviderModelInfo] = []
    for entry in entries:
        if not isinstance(entry, dict):
            continue
        model_id = entry.get("id")
        if not isinstance(model_id, str):
            continue
        owner = entry.get("owned_by")
        created = entry.get("created")
        models.append(
            ProviderModelInfo(
                id=model_id,
                owner=owner if isinstance(owner, str) else None,
                created=created if isinstance(created, int) else None,
            )
        )
    return tuple(models)
