"""OpenAI Responses API adapter."""

import asyncio
import json
import logging
from collections.abc import AsyncIterator
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

logger = logging.getLogger("colossus.adapters.openai_responses")


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
        http_client_config: HttpClientConfig | None = None,
        retry_attempts: int = 3,
        retry_delay_seconds: float = 0.25,
        transport_retry_attempts: int | None = None,
        transport_retry_delay_seconds: float | None = None,
    ) -> None:
        self._api_key = api_key
        self._base_url = base_url.rstrip("/")
        self._timeout_seconds = timeout_seconds
        self._http_client_config = (
            http_client_config or HttpClientConfig()
        ).with_ca_bundle(ca_bundle)
        self._ca_bundle = self._http_client_config.ca_bundle
        self._transport = transport
        self._retry_attempts = max(1, transport_retry_attempts or retry_attempts)
        self._retry_delay_seconds = max(
            0.0,
            (
                transport_retry_delay_seconds
                if transport_retry_delay_seconds is not None
                else retry_delay_seconds
            ),
        )

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
        except (httpx.RequestError, OSError) as exc:
            return ProviderReadiness(
                provider=self.name,
                ready=False,
                checks=(
                    ProviderReadinessCheck(
                        name="models_endpoint",
                        status="fail",
                        detail=openai_common.transport_error_detail(
                            exc,
                            f"{self._base_url}/models",
                        ),
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
        payload: dict[str, object] = {
            "model": request.model,
            "instructions": request.instructions,
            "input": _messages_to_responses_input(request.messages, tool_name_codec),
            "store": False,
        }
        if request.tools:
            payload["tools"] = [
                _tool_to_responses_tool(tool, tool_name_codec) for tool in request.tools
            ]
        request_body_bytes = openai_common.json_body_size(payload)
        async with httpx.AsyncClient(
            **self._http_client_config.async_client_kwargs(
                timeout=self._timeout_seconds,
                transport=self._transport,
            )
        ) as client:
            endpoint = f"{self._base_url}/responses"
            _debug_http(
                "provider.responses.request",
                {
                    "url": openai_common.safe_url(endpoint),
                    "request_body_bytes": request_body_bytes,
                    **_responses_request_debug_shape(payload),
                },
            )
            try:
                response = await self._post_responses_with_retries(
                    client,
                    endpoint,
                    payload,
                )
                _debug_http(
                    "provider.responses.response",
                    openai_common.http_response_debug_shape(response),
                )
                response.raise_for_status()
            except httpx.HTTPStatusError as exc:
                _debug_http(
                    "provider.responses.status_error",
                    openai_common.http_response_debug_shape(exc.response),
                )
                raise ProviderError(
                    openai_common.http_error_detail(
                        exc,
                        request_body_bytes=request_body_bytes,
                    )
                ) from exc
            except (httpx.RequestError, OSError) as exc:
                detail = openai_common.transport_error_detail(exc, endpoint)
                _debug_http("provider.responses.request_error", {"detail": detail})
                raise ProviderError(detail) from exc
            data = response.json()

        response_shape = _responses_response_shape(data)
        _debug_http(
            "provider.responses.response_shape",
            {"response_shape": response_shape},
        )
        if not isinstance(data, dict):
            raise ProviderError(
                "Provider returned a non-object JSON response. "
                f"response_shape={json.dumps(response_shape)}"
            )

        output_text: list[str] = []
        tool_call_count = 0
        for item in data.get("output", []):
            if not isinstance(item, dict):
                continue
            item_type = item.get("type")
            if item_type == "message":
                text = _extract_message_text(item)
                if text:
                    output_text.append(text)
                    yield ModelDeltaEvent(text=text)
            elif item_type == "function_call":
                tool_call_count += 1
                yield ToolCallRequestedEvent(
                    call_id=str(item.get("call_id", "")),
                    name=tool_name_codec.decode(str(item.get("name", ""))),
                    arguments=openai_common.parse_tool_arguments(
                        str(item.get("arguments") or "{}"),
                        call_id=str(item.get("call_id", "")),
                        tool_name=str(item.get("name", "")),
                    ),
                )
            elif item_type == "custom_tool_call":
                tool_call_count += 1
                yield ToolCallRequestedEvent(
                    call_id=str(item.get("call_id", "")),
                    name=tool_name_codec.decode(str(item.get("name", ""))),
                    arguments={"input": str(item.get("input", ""))},
                )
        _debug_http(
            "provider.responses.complete",
            {
                "text_chars": sum(len(chunk) for chunk in output_text),
                "tool_call_count": tool_call_count,
            },
        )
        if output_text and tool_call_count == 0:
            yield FinalOutputEvent(text="".join(output_text))
        if not output_text and tool_call_count == 0:
            _debug_http(
                "provider.responses.empty",
                {"response_shape": response_shape},
            )
            raise ProviderError(
                "Provider returned no assistant content or tool calls. "
                f"response_shape={json.dumps(response_shape)}"
            )

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

    async def _post_responses_with_retries(
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
                        "provider.responses.status_retry",
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
                    "provider.responses.transport_retry",
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
        raise RuntimeError("responses transport retry loop exited without a response")


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
        "parameters": provider_input_schema(tool.input_schema),
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


def _debug_http(event: str, details: dict[str, object]) -> None:
    openai_common.debug_http(logger, event, details)


def _responses_request_debug_shape(payload: dict[str, object]) -> dict[str, object]:
    instructions = payload.get("instructions")
    shape: dict[str, object] = {
        "model": payload.get("model") if isinstance(payload.get("model"), str) else None,
        "instructions_chars": len(instructions) if isinstance(instructions, str) else 0,
        "store": payload.get("store") is True,
    }
    input_items = payload.get("input")
    if isinstance(input_items, list):
        shape["input_count"] = len(input_items)
        shape["input_item_types"] = [_responses_input_item_type(item) for item in input_items]
        if input_items:
            shape["last_input"] = _responses_input_item_debug_shape(input_items[-1])
    else:
        shape["input_type"] = type(input_items).__name__

    tools = payload.get("tools")
    if isinstance(tools, list):
        shape["tool_count"] = len(tools)
        shape["tool_names"] = [
            str(tool.get("name"))
            for tool in tools
            if isinstance(tool, dict) and isinstance(tool.get("name"), str)
        ]
    else:
        shape["tools_type"] = type(tools).__name__
    return shape


def _responses_input_item_type(item: object) -> str:
    if not isinstance(item, dict):
        return type(item).__name__
    item_type = item.get("type")
    if isinstance(item_type, str):
        return item_type
    role = item.get("role")
    return role if isinstance(role, str) else "<missing>"


def _responses_input_item_debug_shape(item: object) -> dict[str, object]:
    if not isinstance(item, dict):
        return {"type": type(item).__name__}
    shape: dict[str, object] = {
        "keys": sorted(item.keys()),
        "item_type": _responses_input_item_type(item),
    }
    for field in ("content", "output", "arguments", "input"):
        value = item.get(field)
        if value is None:
            continue
        shape[f"{field}_type"] = type(value).__name__
        if isinstance(value, str):
            shape[f"{field}_chars"] = len(value)
        elif isinstance(value, list):
            shape[f"{field}_items"] = len(value)
    call_id = item.get("call_id")
    if isinstance(call_id, str):
        shape["call_id_present"] = bool(call_id)
    name = item.get("name")
    if isinstance(name, str):
        shape["name_present"] = bool(name)
    return shape


def _responses_response_shape(data: object) -> dict[str, object]:
    if not isinstance(data, dict):
        return {"type": type(data).__name__}
    output = data.get("output")
    if not isinstance(output, list):
        return {"top_keys": sorted(data.keys()), "output_type": type(output).__name__}
    return {
        "top_keys": sorted(data.keys()),
        "output_count": len(output),
        "output": [_responses_output_item_shape(item) for item in output[:5]],
    }


def _responses_output_item_shape(item: object) -> dict[str, object]:
    if not isinstance(item, dict):
        return {"type": type(item).__name__}
    shape: dict[str, object] = {"keys": sorted(item.keys())}
    item_type = item.get("type")
    if isinstance(item_type, str):
        shape["type"] = item_type
    content = item.get("content")
    shape["content_type"] = type(content).__name__
    if isinstance(content, list):
        shape["content_items"] = [_typed_item_shape(entry) for entry in content[:3]]
    for field in ("arguments", "input"):
        value = item.get(field)
        if value is None:
            continue
        shape[f"{field}_type"] = type(value).__name__
        if isinstance(value, str):
            shape[f"{field}_chars"] = len(value)
    call_id = item.get("call_id")
    if isinstance(call_id, str):
        shape["call_id_present"] = bool(call_id)
    name = item.get("name")
    if isinstance(name, str):
        shape["name_present"] = bool(name)
    return shape


def _typed_item_shape(item: object) -> dict[str, object]:
    if not isinstance(item, dict):
        return {"type": type(item).__name__}
    shape: dict[str, object] = {"keys": sorted(item.keys())}
    item_type = item.get("type")
    if isinstance(item_type, str):
        shape["type"] = item_type
    text = item.get("text")
    if isinstance(text, str):
        shape["text_chars"] = len(text)
    return shape
