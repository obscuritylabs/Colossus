"""Shared helpers for OpenAI-compatible provider adapters."""

import json
import logging
from urllib.parse import urlsplit, urlunsplit

import httpx

from colossus.domain.errors import ProviderError


def parse_tool_arguments(
    arguments_text: str,
    *,
    call_id: str,
    tool_name: str,
) -> dict[str, object]:
    try:
        parsed = json.loads(arguments_text or "{}")
    except json.JSONDecodeError as exc:
        raise ProviderError(
            "Provider returned invalid JSON for tool call arguments. "
            f"tool={tool_name or '<unknown>'} call_id={call_id or '<unknown>'} "
            f"size={len(arguments_text)} position={exc.pos}"
        ) from exc
    if not isinstance(parsed, dict):
        raise ProviderError(
            "Provider returned non-object JSON for tool call arguments. "
            f"tool={tool_name or '<unknown>'} call_id={call_id or '<unknown>'}"
        )
    return parsed


def debug_http(
    logger: logging.Logger,
    event: str,
    details: dict[str, object],
) -> None:
    if not logger.isEnabledFor(logging.DEBUG):
        return
    logger.debug("%s %s", event, json.dumps(details, sort_keys=True))


def http_response_debug_shape(response: httpx.Response) -> dict[str, object]:
    content_length = response.headers.get("content-length")
    return {
        "status_code": response.status_code,
        "content_type": response.headers.get("content-type", ""),
        "content_length": content_length if content_length is not None else "",
        "url": safe_url(str(response.request.url)),
    }


def json_body_size(payload: dict[str, object]) -> int:
    return len(json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode("utf-8"))


def http_error_detail(
    exc: httpx.HTTPStatusError,
    *,
    request_body_bytes: int | None = None,
) -> str:
    response_text = exc.response.text.strip()
    suffix = f": {response_text[:500]}" if response_text else ""
    detail = f"{exc.response.status_code} from {safe_url(str(exc.request.url))}{suffix}"
    if request_body_bytes is not None:
        detail = f"{detail} request_body_bytes={request_body_bytes}"
    if exc.response.status_code == 413:
        detail = (
            f"{detail}. The endpoint rejected the serialized HTTP request body; "
            "this can happen even when the context token estimate fits. "
            "Reduce message/tool payload size or set context.max_request_bytes below "
            "the endpoint limit."
        )
    return detail


def should_retry_status(status_code: int) -> bool:
    return status_code in {408, 409, 429} or status_code >= 500


def retry_delay(base_delay_seconds: float, attempt: int) -> float:
    multiplier = 1.0
    for _ in range(max(attempt - 1, 0)):
        multiplier *= 2.0
    return base_delay_seconds * multiplier


def transport_error_detail(
    exc: httpx.RequestError | OSError,
    url: str | None = None,
) -> str:
    request = exc.request if isinstance(exc, httpx.RequestError) else None
    location_url = str(request.url) if request is not None else url
    location = f" from {safe_url(location_url)}" if location_url else ""
    suffix = f": {exc}" if str(exc) else ""
    return f"{type(exc).__name__}{location}{suffix}"


def safe_url(value: object) -> str:
    text = str(value)
    try:
        parts = urlsplit(text)
    except ValueError:
        return text.split("?", 1)[0]
    host = parts.hostname or ""
    if ":" in host and not host.startswith("["):
        host = f"[{host}]"
    netloc = host
    try:
        port = parts.port
    except ValueError:
        port = None
    if port is not None:
        netloc = f"{netloc}:{port}"
    return urlunsplit((parts.scheme, netloc, parts.path, "", ""))
