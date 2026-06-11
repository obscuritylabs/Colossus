import json
from typing import Any

import httpx
import pytest

from colossus.adapters.local_openai_chat import LocalOpenAIChatProvider
from colossus.adapters.openai_responses import OpenAIResponsesProvider
from colossus.domain.errors import ProviderError
from colossus.domain.events import (
    FinalOutputEvent,
    ModelDeltaEvent,
    ReasoningSummaryEvent,
    ToolCallRequestedEvent,
)
from colossus.domain.messages import AssistantMessage, ToolResultMessage, UserMessage
from colossus.domain.requests import ModelRequest
from colossus.domain.tools import ToolCall, ToolSpec


def _tool() -> ToolSpec:
    return ToolSpec(
        name="lookup",
        description="Look up a value.",
        input_schema={
            "type": "object",
            "properties": {"query": {"type": "string"}},
            "required": ["query"],
            "additionalProperties": False,
        },
    )


@pytest.mark.asyncio
async def test_openai_responses_provider_maps_payload_and_events() -> None:
    captured: dict[str, Any] = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["url"] = str(request.url)
        captured["authorization"] = request.headers["authorization"]
        captured["payload"] = json.loads(request.content)
        return httpx.Response(
            200,
            json={
                "output": [
                    {"type": "message", "content": [{"type": "output_text", "text": "hi "}]},
                    {
                        "type": "function_call",
                        "call_id": "call_1",
                        "name": "lookup",
                        "arguments": '{"query":"alpha"}',
                    },
                    {
                        "type": "custom_tool_call",
                        "call_id": "call_2",
                        "name": "shell",
                        "input": "pwd",
                    },
                    {"type": "message", "content": [{"type": "text", "text": "there"}]},
                ]
            },
        )

    provider = OpenAIResponsesProvider(
        api_key="test-key",
        base_url="https://provider.test/v1/",
        transport=httpx.MockTransport(handler),
    )
    request = ModelRequest(
        model="model-a",
        instructions="Be terse.",
        messages=(
            UserMessage(content="hello"),
            AssistantMessage(content="working"),
            ToolResultMessage(call_id="call_0", name="lookup", content='{"ok":true}'),
        ),
        tools=(_tool(),),
    )

    events = [event async for event in provider.stream(request)]

    assert captured["url"] == "https://provider.test/v1/responses"
    assert captured["authorization"] == "Bearer test-key"
    assert captured["payload"]["input"] == [
        {"role": "user", "content": "hello"},
        {"role": "assistant", "content": "working"},
        {"type": "function_call_output", "call_id": "call_0", "output": '{"ok":true}'},
    ]
    assert captured["payload"]["tools"][0]["strict"] is True
    assert events == [
        ModelDeltaEvent(text="hi "),
        ToolCallRequestedEvent(call_id="call_1", name="lookup", arguments={"query": "alpha"}),
        ToolCallRequestedEvent(call_id="call_2", name="shell", arguments={"input": "pwd"}),
        ModelDeltaEvent(text="there"),
        FinalOutputEvent(text="hi there"),
    ]


@pytest.mark.asyncio
async def test_local_openai_chat_provider_maps_payload_and_events() -> None:
    captured: dict[str, Any] = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["url"] = str(request.url)
        captured["authorization"] = request.headers["authorization"]
        captured["payload"] = json.loads(request.content)
        return httpx.Response(
            200,
            json={
                "choices": [
                    {
                        "message": {
                            "content": "done",
                            "tool_calls": [
                                {
                                    "id": "call_1",
                                    "function": {
                                        "name": "lookup",
                                        "arguments": '{"query":"beta"}',
                                    },
                                }
                            ],
                        }
                    }
                ]
            },
        )

    provider = LocalOpenAIChatProvider(
        api_key="local-key",
        base_url="http://localhost:11434/v1/",
        transport=httpx.MockTransport(handler),
    )
    request = ModelRequest(
        model="model-b",
        instructions="System text.",
        messages=(
            UserMessage(content="question"),
            AssistantMessage(content="answer so far"),
            ToolResultMessage(call_id="call_0", name="lookup", content="tool output"),
        ),
        tools=(_tool(),),
    )

    events = [event async for event in provider.stream(request)]

    assert captured["url"] == "http://localhost:11434/v1/chat/completions"
    assert captured["authorization"] == "Bearer local-key"
    assert captured["payload"]["messages"] == [
        {"role": "system", "content": "System text."},
        {"role": "user", "content": "question"},
        {"role": "assistant", "content": "answer so far"},
        {"role": "tool", "tool_call_id": "call_0", "content": "tool output"},
    ]
    assert captured["payload"]["tools"][0]["function"]["name"] == "lookup"
    assert captured["payload"]["stream"] is True
    assert events == [
        ToolCallRequestedEvent(call_id="call_1", name="lookup", arguments={"query": "beta"}),
        ModelDeltaEvent(text="done"),
        FinalOutputEvent(text="done"),
    ]


@pytest.mark.asyncio
async def test_local_openai_chat_provider_streams_content_reasoning_and_tool_calls() -> None:
    captured: dict[str, Any] = {}

    def sse(value: object) -> str:
        return f"data: {json.dumps(value)}\n\n"

    content = (
        ": OPENROUTER PROCESSING\n\n"
        + sse(
            {
                "choices": [
                    {
                        "delta": {
                            "reasoning_details": [
                                {
                                    "type": "reasoning.summary",
                                    "summary": "Checked whether a lookup is needed.",
                                    "format": "openrouter",
                                    "id": "reason-1",
                                },
                                {
                                    "type": "reasoning.text",
                                    "text": "raw private-style reasoning should not render",
                                },
                            ],
                            "content": "hel",
                        }
                    }
                ]
            }
        )
        + sse({"choices": [{"delta": {"content": "lo"}}]})
        + sse(
            {
                "choices": [
                    {
                        "delta": {
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_1",
                                    "type": "function",
                                    "function": {
                                        "name": "lookup",
                                        "arguments": '{"query"',
                                    },
                                }
                            ]
                        }
                    }
                ]
            }
        )
        + sse(
            {
                "choices": [
                    {
                        "delta": {
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "function": {
                                        "arguments": ':"gamma"}',
                                    },
                                }
                            ]
                        }
                    }
                ]
            }
        )
        + "data: [DONE]\n\n"
    )

    def handler(request: httpx.Request) -> httpx.Response:
        captured["payload"] = json.loads(request.content)
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            content=content,
            request=request,
        )

    provider = LocalOpenAIChatProvider(
        api_key="local-key",
        base_url="http://localhost:11434/v1/",
        transport=httpx.MockTransport(handler),
    )

    events = [
        event
        async for event in provider.stream(
            ModelRequest(
                model="model-b",
                instructions="System text.",
                messages=(UserMessage(content="question"),),
                tools=(_tool(),),
            )
        )
    ]

    assert captured["payload"]["stream"] is True
    assert events == [
        ReasoningSummaryEvent(
            summary="Checked whether a lookup is needed.",
            provider_format="openrouter",
            detail_id="reason-1",
        ),
        ModelDeltaEvent(text="hel"),
        ModelDeltaEvent(text="lo"),
        ToolCallRequestedEvent(call_id="call_1", name="lookup", arguments={"query": "gamma"}),
        FinalOutputEvent(text="hello"),
    ]


@pytest.mark.asyncio
async def test_local_openai_chat_provider_serializes_assistant_tool_calls() -> None:
    captured: dict[str, Any] = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["payload"] = json.loads(request.content)
        return httpx.Response(
            200,
            json={"choices": [{"message": {"content": "done"}}]},
            request=request,
        )

    provider = LocalOpenAIChatProvider(
        api_key="local-key",
        base_url="http://localhost:11434/v1/",
        transport=httpx.MockTransport(handler),
    )
    request = ModelRequest(
        model="model-b",
        instructions="System text.",
        messages=(
            UserMessage(content="question"),
            AssistantMessage(
                content="",
                tool_calls=(
                    ToolCall(
                        call_id="call_1",
                        name="lookup",
                        arguments={"query": "beta"},
                    ),
                ),
            ),
            ToolResultMessage(call_id="call_1", name="lookup", content="tool output"),
        ),
        tools=(_tool(),),
    )

    _ = [event async for event in provider.stream(request)]

    assert captured["payload"]["messages"] == [
        {"role": "system", "content": "System text."},
        {"role": "user", "content": "question"},
        {
            "role": "assistant",
            "content": None,
            "tool_calls": [
                {
                    "id": "call_1",
                    "type": "function",
                    "function": {
                        "name": "lookup",
                        "arguments": '{"query": "beta"}',
                    },
                }
            ],
        },
        {"role": "tool", "tool_call_id": "call_1", "content": "tool output"},
    ]


@pytest.mark.asyncio
async def test_local_openai_chat_provider_wraps_http_errors() -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        return httpx.Response(404, text="model not found", request=request)

    provider = LocalOpenAIChatProvider(
        api_key="local-key",
        base_url="http://localhost:11434/v1/",
        transport=httpx.MockTransport(handler),
    )
    request = ModelRequest(
        model="missing-model",
        instructions="System text.",
        messages=(UserMessage(content="question"),),
        tools=(),
    )

    with pytest.raises(ProviderError, match=r"404.*model not found"):
        _ = [event async for event in provider.stream(request)]
