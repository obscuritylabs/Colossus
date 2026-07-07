"""Local OpenAI-compatible chat-completions adapter."""

import asyncio
import json
import logging
from collections.abc import AsyncIterator
from dataclasses import dataclass, field
from pathlib import Path

import httpx

from colossus.adapters.model_catalog import extract_model_infos
from colossus.adapters.openai_compat import common as openai_common
from colossus.adapters.tool_name_codec import ToolNameCodec
from colossus.adapters.tool_schema import provider_input_schema
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
from colossus.infrastructure.http_client import HttpClientConfig

logger = logging.getLogger("colossus.adapters.local_openai_chat")


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
        http_client_config: HttpClientConfig | None = None,
        retry_attempts: int = 3,
        retry_delay_seconds: float = 0.25,
    ) -> None:
        self._base_url = base_url.rstrip("/")
        self._api_key = api_key
        self._timeout_seconds = timeout_seconds
        self._http_client_config = (
            http_client_config or HttpClientConfig()
        ).with_ca_bundle(ca_bundle)
        self._ca_bundle = self._http_client_config.ca_bundle
        self._transport = transport
        self._retry_attempts = max(1, retry_attempts)
        self._retry_delay_seconds = max(0.0, retry_delay_seconds)

    @property
    def base_url(self) -> str:
        return self._base_url

    @property
    def ca_bundle(self) -> Path | None:
        return self._ca_bundle

    @property
    def http_client_config(self) -> HttpClientConfig:
        return self._http_client_config

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
        return extract_model_infos(response.json())

    async def stream(self, request: ModelRequest) -> AsyncIterator[RunEvent]:
        tool_name_codec = ToolNameCodec.from_tools(request.tools)
        messages: list[dict[str, object]] = [
            {"role": "system", "content": request.instructions},
            *[
                _message_to_chat_message(message, tool_name_codec)
                for message in request.messages
            ],
        ]
        payload: dict[str, object] = {
            "model": request.model,
            "messages": messages,
        }
        if request.tools:
            payload["tools"] = [
                _tool_to_chat_tool(tool, tool_name_codec) for tool in request.tools
            ]
        async with httpx.AsyncClient(
            **self._http_client_config.async_client_kwargs(
                timeout=self._timeout_seconds,
                transport=self._transport,
            )
        ) as client:
            try:
                async for event in self._stream_chat_completion(client, payload, tool_name_codec):
                    yield event
                return
            except _StreamingFallbackRequired as exc:
                _debug_http("provider.chat.streaming_fallback", {"reason": str(exc)})
                try:
                    async for event in self._non_stream_chat_completion(
                        client,
                        payload,
                        tool_name_codec,
                    ):
                        yield event
                except ProviderError as fallback_exc:
                    detail = str(exc)
                    if detail:
                        raise ProviderError(
                            f"{fallback_exc}; streaming_fallback={detail}"
                        ) from fallback_exc
                    raise

    async def _get_models_response(self) -> httpx.Response:
        async with httpx.AsyncClient(
            **self._http_client_config.async_client_kwargs(
                timeout=self._timeout_seconds,
                transport=self._transport,
            )
        ) as client:
            return await client.get(
                f"{self._base_url}/models",
                headers={"Authorization": f"Bearer {self._api_key}"},
            )

    async def _stream_chat_completion(
        self,
        client: httpx.AsyncClient,
        payload: dict[str, object],
        tool_name_codec: ToolNameCodec,
    ) -> AsyncIterator[RunEvent]:
        streamed_payload = {**payload, "stream": True}
        streamed_body_bytes = openai_common.json_body_size(streamed_payload)
        endpoint = f"{self._base_url}/chat/completions"
        _debug_http(
            "provider.chat.stream.request",
            {
                "url": openai_common.safe_url(endpoint),
                "request_body_bytes": streamed_body_bytes,
                **_chat_request_debug_shape(streamed_payload),
            },
        )
        try:
            stream_context = client.stream(
                "POST",
                endpoint,
                headers={"Authorization": f"Bearer {self._api_key}"},
                json=streamed_payload,
            )
            async with stream_context as response:
                async for event in self._events_from_stream_response(
                    response,
                    tool_name_codec,
                    request_body_bytes=streamed_body_bytes,
                ):
                    yield event
        except (httpx.RequestError, OSError) as exc:
            detail = openai_common.transport_error_detail(exc, endpoint)
            _debug_http("provider.chat.stream.request_error", {"detail": detail})
            raise _StreamingFallbackRequired(
                "Provider stream failed before yielding events. "
                f"{detail}"
            ) from exc

    async def _events_from_stream_response(
        self,
        response: httpx.Response,
        tool_name_codec: ToolNameCodec,
        *,
        request_body_bytes: int,
    ) -> AsyncIterator[RunEvent]:
        _debug_http(
            "provider.chat.stream.response",
            openai_common.http_response_debug_shape(response),
        )
        try:
            response.raise_for_status()
        except httpx.HTTPStatusError as exc:
            _debug_http(
                "provider.chat.stream.status_error",
                openai_common.http_response_debug_shape(exc.response),
            )
            raise _StreamingFallbackRequired(
                _http_status_detail(exc, request_body_bytes=request_body_bytes)
            ) from None
        content_type = response.headers.get("content-type", "")
        if "text/event-stream" not in content_type:
            body = await response.aread()
            data = json.loads(body.decode())
            _debug_http(
                "provider.chat.stream.non_sse_response_shape",
                {"response_shape": _chat_completion_shape(data)},
            )
            events = [event async for event in _events_from_chat_completion(data, tool_name_codec)]
            for event in events:
                yield event
            if not _has_assistant_output(events):
                raise ProviderError(
                    "Provider returned no assistant content or tool calls. "
                    f"response_shape={json.dumps(_chat_completion_shape(data))}"
                )
            return

        output_text: list[str] = []
        tool_calls: dict[int, _StreamToolCall] = {}
        chunk_shapes: list[dict[str, object]] = []
        chunk_count = 0
        events_yielded = False
        try:
            async for item in _stream_json_items(response):
                chunk_count += 1
                chunk_shapes.append(_stream_chunk_shape(item))
                if len(chunk_shapes) > 5:
                    chunk_shapes.pop(0)
                async for event in _events_from_stream_chunk(item, tool_calls, output_text):
                    events_yielded = True
                    yield event
        except httpx.HTTPError as exc:
            detail = _stream_error_detail(exc)
            _debug_http("provider.chat.stream.http_error", {"detail": detail})
            if not events_yielded:
                raise _StreamingFallbackRequired(
                    "Provider stream failed before yielding events. "
                    f"{detail}"
                ) from exc
            raise ProviderError(
                "Provider stream failed after a partial response. "
                f"{detail}"
            ) from exc

        tool_call_events = _tool_call_events(tool_calls, tool_name_codec)
        _debug_http(
            "provider.chat.stream.complete",
            {
                "chunk_count": chunk_count,
                "text_chars": sum(len(chunk) for chunk in output_text),
                "tool_call_count": len(tool_call_events),
            },
        )
        for event in tool_call_events:
            yield event
        if output_text and not tool_call_events:
            yield FinalOutputEvent(text="".join(output_text))
        if not output_text and not tool_call_events:
            _debug_http(
                "provider.chat.stream.empty",
                {"chunk_count": chunk_count, "chunk_shapes": chunk_shapes},
            )
            raise _StreamingFallbackRequired(
                "Provider stream returned no assistant content or tool calls. "
                f"chunk_shapes={json.dumps(chunk_shapes)}"
            )

    async def _non_stream_chat_completion(
        self,
        client: httpx.AsyncClient,
        payload: dict[str, object],
        tool_name_codec: ToolNameCodec,
    ) -> AsyncIterator[RunEvent]:
        endpoint = f"{self._base_url}/chat/completions"
        request_body_bytes = openai_common.json_body_size(payload)
        _debug_http(
            "provider.chat.non_stream.request",
            {
                "url": openai_common.safe_url(endpoint),
                "request_body_bytes": request_body_bytes,
                **_chat_request_debug_shape(payload),
            },
        )
        try:
            response = await self._post_chat_with_retries(client, endpoint, payload)
        except (httpx.RequestError, OSError) as exc:
            detail = openai_common.transport_error_detail(exc, endpoint)
            _debug_http("provider.chat.non_stream.request_error", {"detail": detail})
            raise ProviderError(detail) from exc
        _debug_http(
            "provider.chat.non_stream.response",
            openai_common.http_response_debug_shape(response),
        )
        try:
            response.raise_for_status()
        except httpx.HTTPStatusError as exc:
            _debug_http(
                "provider.chat.non_stream.status_error",
                openai_common.http_response_debug_shape(exc.response),
            )
            raise ProviderError(
                openai_common.http_error_detail(
                    exc,
                    request_body_bytes=request_body_bytes,
                )
            ) from exc
        data = response.json()
        response_shape = _chat_completion_shape(data)
        _debug_http(
            "provider.chat.non_stream.response_shape",
            {"response_shape": response_shape},
        )
        events = [event async for event in _events_from_chat_completion(data, tool_name_codec)]
        for event in events:
            yield event
        if not _has_assistant_output(events):
            _debug_http(
                "provider.chat.non_stream.empty",
                {"response_shape": response_shape},
            )
            raise ProviderError(
                "Provider returned no assistant content or tool calls. "
                f"response_shape={json.dumps(response_shape)}"
            )

    async def _post_chat_with_retries(
        self,
        client: httpx.AsyncClient,
        endpoint: str,
        payload: dict[str, object],
    ) -> httpx.Response:
        last_exc: httpx.RequestError | OSError | None = None
        for attempt in range(1, self._retry_attempts + 1):
            try:
                response = await client.post(
                    endpoint,
                    headers={"Authorization": f"Bearer {self._api_key}"},
                    json=payload,
                )
                if (
                    openai_common.should_retry_status(response.status_code)
                    and attempt < self._retry_attempts
                ):
                    _debug_http(
                        "provider.chat.non_stream.status_retry",
                        {
                            "attempt": attempt,
                            "max_attempts": self._retry_attempts,
                            "status_code": response.status_code,
                            "url": openai_common.safe_url(str(response.request.url)),
                        },
                    )
                    await asyncio.sleep(
                        openai_common.retry_delay(self._retry_delay_seconds, attempt)
                    )
                    continue
                return response
            except (httpx.RequestError, OSError) as exc:
                last_exc = exc
                detail = openai_common.transport_error_detail(exc, endpoint)
                if attempt >= self._retry_attempts:
                    raise
                _debug_http(
                    "provider.chat.non_stream.transport_retry",
                    {
                        "attempt": attempt,
                        "max_attempts": self._retry_attempts,
                        "detail": detail,
                    },
                )
                await asyncio.sleep(
                    openai_common.retry_delay(self._retry_delay_seconds, attempt)
                )
        if last_exc is not None:
            raise last_exc
        raise RuntimeError("chat completion retry loop exited without a response")


def _message_to_chat_message(
    message: Message,
    tool_name_codec: ToolNameCodec,
) -> dict[str, object]:
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
                            "name": tool_name_codec.encode(call.name),
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


def _tool_to_chat_tool(tool: ToolSpec, tool_name_codec: ToolNameCodec) -> dict[str, object]:
    return {
        "type": "function",
        "function": {
            "name": tool_name_codec.encode(tool.name),
            "description": tool.description,
            "parameters": provider_input_schema(tool.input_schema),
        },
    }


async def _events_from_chat_completion(
    data: object,
    tool_name_codec: ToolNameCodec,
) -> AsyncIterator[RunEvent]:
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
    for event in _reasoning_summary_events(message):
        yield event
    requested_tools = False
    for tool_call in message.get("tool_calls", []) or []:
        if not isinstance(tool_call, dict):
            continue
        function = tool_call.get("function")
        if not isinstance(function, dict):
            continue
        requested_tools = True
        yield ToolCallRequestedEvent(
            call_id=str(tool_call.get("id", "")),
            name=tool_name_codec.decode(str(function.get("name", ""))),
            arguments=openai_common.parse_tool_arguments(
                str(function.get("arguments") or "{}"),
                call_id=str(tool_call.get("id", "")),
                tool_name=str(function.get("name", "")),
            ),
        )
    content = _extract_content_text(message.get("content"))
    if content:
        yield ModelDeltaEvent(text=content)
        if not requested_tools:
            yield FinalOutputEvent(text=content)


async def _stream_json_items(response: httpx.Response) -> AsyncIterator[dict[str, object]]:
    data_lines: list[str] = []
    done = False
    async for line in response.aiter_lines():
        if done:
            continue
        if not line:
            if data_lines:
                payload = "\n".join(data_lines)
                data_lines.clear()
                if payload == "[DONE]":
                    done = True
                    continue
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
        for event in _reasoning_summary_events(delta):
            yield event
        content = _extract_content_text(delta.get("content"))
        if content:
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
    tool_name_codec: ToolNameCodec,
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
                name=tool_name_codec.decode(call.name),
                arguments=openai_common.parse_tool_arguments(
                    arguments_text,
                    call_id=call.call_id,
                    tool_name=call.name,
                ),
            )
        )
    return tuple(events)


def _extract_content_text(value: object) -> str:
    if isinstance(value, str):
        return value
    if not isinstance(value, list):
        return ""
    chunks: list[str] = []
    for item in value:
        if not isinstance(item, dict):
            continue
        item_type = item.get("type")
        if item_type not in {
            "text",
            "output_text",
            "input_text",
        }:
            continue
        text = item.get("text")
        if isinstance(text, str):
            chunks.append(text)
    return "".join(chunks)


def _reasoning_summary_events(message_or_delta: object) -> tuple[ReasoningSummaryEvent, ...]:
    if not isinstance(message_or_delta, dict):
        return ()
    value = message_or_delta.get("reasoning_details")
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


def _has_assistant_output(events: list[RunEvent]) -> bool:
    return any(
        isinstance(event, ModelDeltaEvent | FinalOutputEvent | ToolCallRequestedEvent)
        for event in events
    )


def _chat_completion_shape(data: object) -> dict[str, object]:
    if not isinstance(data, dict):
        return {"type": type(data).__name__}
    choices = data.get("choices")
    if not isinstance(choices, list):
        return {"top_keys": sorted(data.keys()), "choices_type": type(choices).__name__}
    shaped_choices: list[dict[str, object]] = []
    for choice in choices[:2]:
        shaped_choices.append(_choice_shape(choice))
    return {"top_keys": sorted(data.keys()), "choices": shaped_choices}


def _stream_chunk_shape(item: object) -> dict[str, object]:
    if not isinstance(item, dict):
        return {"type": type(item).__name__}
    choices = item.get("choices")
    if not isinstance(choices, list):
        return {"top_keys": sorted(item.keys()), "choices_type": type(choices).__name__}
    return {
        "top_keys": sorted(item.keys()),
        "choices": [_choice_shape(choice) for choice in choices[:2]],
    }


def _choice_shape(choice: object) -> dict[str, object]:
    if not isinstance(choice, dict):
        return {"type": type(choice).__name__}
    shaped: dict[str, object] = {"keys": sorted(choice.keys())}
    finish_reason = choice.get("finish_reason")
    if isinstance(finish_reason, str) or finish_reason is None:
        shaped["finish_reason"] = finish_reason
    for payload_field in ("message", "delta"):
        value = choice.get(payload_field)
        if isinstance(value, dict):
            shaped[payload_field] = _message_shape(value)
        elif value is not None:
            shaped[f"{payload_field}_type"] = type(value).__name__
    return shaped


def _message_shape(value: dict[str, object]) -> dict[str, object]:
    shape: dict[str, object] = {"keys": sorted(value.keys())}
    content = value.get("content")
    shape["content_type"] = type(content).__name__
    if isinstance(content, list):
        shape["content_items"] = [_typed_item_shape(item) for item in content[:3]]
    tool_calls = value.get("tool_calls")
    shape["tool_calls_type"] = type(tool_calls).__name__
    if isinstance(tool_calls, list):
        shape["tool_call_items"] = [_typed_item_shape(item) for item in tool_calls[:3]]
    reasoning_details = value.get("reasoning_details")
    shape["reasoning_details_type"] = type(reasoning_details).__name__
    reasoning = value.get("reasoning")
    if reasoning is not None:
        shape["reasoning_type"] = type(reasoning).__name__
    return shape


def _typed_item_shape(item: object) -> dict[str, object]:
    if not isinstance(item, dict):
        return {"type": type(item).__name__}
    shaped: dict[str, object] = {"keys": sorted(item.keys())}
    item_type = item.get("type")
    if isinstance(item_type, str):
        shaped["type"] = item_type
    function = item.get("function")
    if isinstance(function, dict):
        shaped["function_keys"] = sorted(function.keys())
    return shaped


def _debug_http(event: str, details: dict[str, object]) -> None:
    openai_common.debug_http(logger, event, details)


def _chat_request_debug_shape(payload: dict[str, object]) -> dict[str, object]:
    shape: dict[str, object] = {
        "model": payload.get("model") if isinstance(payload.get("model"), str) else None,
        "stream": payload.get("stream") is True,
    }
    messages = payload.get("messages")
    if isinstance(messages, list):
        role_counts: dict[str, int] = {}
        roles: list[str] = []
        for message in messages:
            role = _message_role(message)
            roles.append(role)
            role_counts[role] = role_counts.get(role, 0) + 1
        shape["message_count"] = len(messages)
        shape["message_roles"] = roles
        shape["message_role_counts"] = role_counts
        if messages:
            shape["last_message"] = _chat_message_debug_shape(messages[-1])
    else:
        shape["messages_type"] = type(messages).__name__

    tools = payload.get("tools")
    if isinstance(tools, list):
        shape["tool_count"] = len(tools)
        shape["tool_names"] = _chat_tool_names(tools)
    else:
        shape["tools_type"] = type(tools).__name__
    return shape


def _message_role(message: object) -> str:
    if not isinstance(message, dict):
        return type(message).__name__
    role = message.get("role")
    return role if isinstance(role, str) else "<missing>"


def _chat_message_debug_shape(message: object) -> dict[str, object]:
    if not isinstance(message, dict):
        return {"type": type(message).__name__}
    shape: dict[str, object] = {"keys": sorted(message.keys()), "role": _message_role(message)}
    content = message.get("content")
    shape["content_type"] = type(content).__name__
    if isinstance(content, str):
        shape["content_chars"] = len(content)
    elif isinstance(content, list):
        shape["content_items"] = len(content)
    tool_calls = message.get("tool_calls")
    if isinstance(tool_calls, list):
        shape["tool_call_count"] = len(tool_calls)
    tool_call_id = message.get("tool_call_id")
    if isinstance(tool_call_id, str):
        shape["tool_call_id_present"] = bool(tool_call_id)
    return shape


def _chat_tool_names(tools: list[object]) -> list[str]:
    names: list[str] = []
    for tool in tools:
        if not isinstance(tool, dict):
            continue
        function = tool.get("function")
        if not isinstance(function, dict):
            continue
        name = function.get("name")
        if isinstance(name, str):
            names.append(name)
    return names


@dataclass
class _StreamToolCall:
    call_id: str = ""
    name: str = ""
    arguments: list[str] = field(default_factory=list)


class _StreamingFallbackRequired(Exception):
    """Raised when a streaming attempt should be retried without streaming."""


def _http_status_detail(
    exc: httpx.HTTPStatusError,
    *,
    request_body_bytes: int,
) -> str:
    content_type = exc.response.headers.get("content-type", "")
    suffix = f" content_type={content_type}" if content_type else ""
    detail = (
        f"HTTP {exc.response.status_code} from "
        f"{openai_common.safe_url(str(exc.request.url))}{suffix}"
    )
    if request_body_bytes:
        detail = f"{detail} request_body_bytes={request_body_bytes}"
    return detail


def _stream_error_detail(exc: httpx.HTTPError) -> str:
    request = getattr(exc, "request", None)
    location = (
        f" from {openai_common.safe_url(str(request.url))}"
        if request is not None
        else ""
    )
    suffix = f": {exc}" if str(exc) else ""
    return f"{type(exc).__name__}{location}{suffix}"
