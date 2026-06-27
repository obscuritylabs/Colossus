import asyncio
import json
import os
import time
from uuid import uuid4

import httpx
import pytest

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.credentials_env import EnvCredentialBroker
from colossus.adapters.integration_tools import create_integration_tools
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.application.integrations import IntegrationService
from colossus.application.tools import FunctionToolExecutor, InMemoryToolRegistry
from colossus.domain.errors import ToolExecutionError
from colossus.domain.tools import ToolCall, ToolResult
from colossus.infrastructure.http_client import HttpClientConfig

pytestmark = pytest.mark.integration
if os.environ.get("COLOSSUS_OPENSEARCH_LIVE") != "1":
    pytestmark = [
        pytest.mark.integration,
        pytest.mark.skip(
            reason="Set COLOSSUS_OPENSEARCH_LIVE=1 to run against local OpenSearch."
        ),
    ]


@pytest.mark.asyncio
async def test_opensearch_live_document_lifecycle(tmp_path) -> None:
    base_url = os.environ.get("COLOSSUS_OPENSEARCH_URL", "http://127.0.0.1:9200")
    await _wait_for_opensearch(base_url)
    index_name = f"colossus-live-{uuid4().hex}"
    service = IntegrationService(
        SQLiteStateStore(tmp_path / "state.sqlite3"),
        JsonlAuditSink(tmp_path / "audit.jsonl"),
        EnvCredentialBroker(),
    )
    connection = await service.connect(
        "opensearch",
        config={"base_url": base_url, "auth_type": "none"},
    )
    specs, handlers = create_integration_tools(
        (connection,),
        EnvCredentialBroker(),
        http_client_config=HttpClientConfig(trust_env=False),
    )
    executor = FunctionToolExecutor(handlers, InMemoryToolRegistry(specs))
    try:
        health = await _execute(executor, "opensearch.health", {})
        assert json.loads(health.output)["status_code"] == 200

        await _execute(
            executor,
            "opensearch.index_document",
            {
                "index": index_name,
                "id": "doc-1",
                "document": {"title": "Colossus", "status": "new"},
                "refresh": "wait_for",
            },
        )
        fetched = await _execute(
            executor,
            "opensearch.get_document",
            {"index": index_name, "id": "doc-1"},
        )
        assert json.loads(fetched.output)["result"]["_source"]["title"] == "Colossus"

        searched = await _execute(
            executor,
            "opensearch.search",
            {
                "index": index_name,
                "query": {"match": {"title": "Colossus"}},
                "size": 5,
            },
        )
        hits = json.loads(searched.output)["result"]["hits"]["hits"]
        assert hits[0]["_id"] == "doc-1"

        await _execute(
            executor,
            "opensearch.update_document",
            {
                "index": index_name,
                "id": "doc-1",
                "doc": {"status": "updated"},
                "refresh": "wait_for",
            },
        )
        updated = await _execute(
            executor,
            "opensearch.get_document",
            {"index": index_name, "id": "doc-1"},
        )
        assert json.loads(updated.output)["result"]["_source"]["status"] == "updated"

        await _execute(
            executor,
            "opensearch.delete_document",
            {"index": index_name, "id": "doc-1", "refresh": "wait_for"},
        )
        with pytest.raises(ToolExecutionError, match="404"):
            await _execute(
                executor,
                "opensearch.get_document",
                {"index": index_name, "id": "doc-1"},
            )
    finally:
        await _delete_index(base_url, index_name)


async def _execute(
    executor: FunctionToolExecutor,
    name: str,
    arguments: dict[str, object],
) -> ToolResult:
    return await executor.execute(
        ToolCall(call_id=f"call-{uuid4().hex}", name=name, arguments=arguments)
    )


async def _wait_for_opensearch(base_url: str, *, timeout_seconds: float = 60.0) -> None:
    deadline = time.monotonic() + timeout_seconds
    last_error = "not checked"
    async with httpx.AsyncClient(timeout=2.0, trust_env=False) as client:
        while time.monotonic() < deadline:
            try:
                response = await client.get(f"{base_url.rstrip('/')}/_cluster/health")
            except httpx.RequestError as exc:
                last_error = exc.__class__.__name__
            else:
                if response.status_code == 200:
                    return
                last_error = f"HTTP {response.status_code}: {response.text[:120]}"
            await asyncio.sleep(1)
    pytest.fail(f"OpenSearch did not become ready at {base_url}: {last_error}")


async def _delete_index(base_url: str, index_name: str) -> None:
    async with httpx.AsyncClient(timeout=10.0, trust_env=False) as client:
        await client.delete(f"{base_url.rstrip('/')}/{index_name}")
