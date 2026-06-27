import base64
import json
from pathlib import Path

import httpx
import pytest
from typer.testing import CliRunner

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.credentials_env import EnvCredentialBroker
from colossus.adapters.integration_tools import create_integration_tools
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.application.integrations import IntegrationService
from colossus.application.tools import FunctionToolExecutor, InMemoryToolRegistry
from colossus.cli import app
from colossus.domain.tools import ToolCall
from colossus.infrastructure.http_client import HttpClientConfig


def _service(tmp_path: Path) -> IntegrationService:
    return IntegrationService(
        SQLiteStateStore(tmp_path / "state.sqlite3"),
        JsonlAuditSink(tmp_path / "audit.jsonl"),
        EnvCredentialBroker(),
    )


@pytest.mark.asyncio
async def test_github_integration_is_available_and_pending_auth(tmp_path: Path) -> None:
    service = _service(tmp_path)

    statuses = await service.list_statuses()
    connection = await service.connect("github")

    github = next(status for status in statuses if status.name == "github")
    opensearch = next(status for status in statuses if status.name == "opensearch")
    searxng = next(status for status in statuses if status.name == "searxng")
    assert github.status == "available"
    assert github.auth_type == "bearer"
    assert "github.repos" in github.tools
    assert opensearch.status == "available"
    assert opensearch.auth_type == "none"
    assert "opensearch.search" in opensearch.tools
    assert "opensearch.index_document" in opensearch.tools
    assert searxng.status == "available"
    assert searxng.auth_type == "none"
    assert "searxng.search" in searxng.tools
    assert connection.status == "pending_auth"
    assert connection.credential_ref is None


@pytest.mark.asyncio
async def test_searxng_integration_connects_with_local_base_url(tmp_path: Path) -> None:
    service = _service(tmp_path)

    connection = await service.connect(
        "searxng",
        config={"base_url": "http://localhost:8888"},
    )

    stored = await service.get_connection("searxng")
    assert connection.status == "connected"
    assert connection.credential_ref is None
    assert connection.config["base_url"] == "http://localhost:8888"
    assert stored is not None
    assert stored.config["base_url"] == "http://localhost:8888"


@pytest.mark.asyncio
async def test_integration_connect_stores_credential_ref_not_secret(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("GITHUB_TOKEN", "super-secret-token")
    service = _service(tmp_path)

    connection = await service.connect("github", credential_ref="env:GITHUB_TOKEN")

    stored = await service.get_connection("github")
    audit_text = (tmp_path / "audit.jsonl").read_text(encoding="utf-8")
    assert connection.status == "connected"
    assert stored is not None
    assert stored.credential_ref == "env:GITHUB_TOKEN"
    assert "super-secret-token" not in stored.model_dump_json()
    assert "super-secret-token" not in audit_text
    assert "env:GITHUB_TOKEN" in audit_text


@pytest.mark.asyncio
async def test_github_tool_uses_brokered_auth_without_model_visible_secret(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("GITHUB_TOKEN", "super-secret-token")
    service = _service(tmp_path)
    connection = await service.connect("github", credential_ref="env:GITHUB_TOKEN")
    seen_authorization = ""

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal seen_authorization
        seen_authorization = request.headers["authorization"]
        return httpx.Response(
            200,
            json=[{"name": "colossus", "private": True}],
            headers={"content-type": "application/json"},
        )

    specs, handlers = create_integration_tools(
        (connection,),
        EnvCredentialBroker(),
        audit_sink=JsonlAuditSink(tmp_path / "tool-audit.jsonl"),
        http_client_config=HttpClientConfig(trust_env=False),
        http_transport=httpx.MockTransport(handler),
        github_api_base_url="https://github.example.test",
    )
    repo_spec = next(spec for spec in specs if spec.name == "github.repos")
    executor = FunctionToolExecutor(handlers, InMemoryToolRegistry(specs))

    result = await executor.execute(
        ToolCall(
            call_id="call-1",
            name="github.repos",
            arguments={"visibility": "all", "max_results": 2},
        )
    )

    assert "credential_ref" not in repo_spec.input_schema.get("properties", {})
    assert seen_authorization == "Bearer super-secret-token"
    assert "super-secret-token" not in result.output
    assert json.loads(result.output)["result"][0]["name"] == "colossus"
    assert "super-secret-token" not in (tmp_path / "tool-audit.jsonl").read_text(
        encoding="utf-8"
    )


@pytest.mark.asyncio
async def test_searxng_tool_uses_config_and_optional_brokered_auth(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("SEARXNG_API_KEY", "secret-token")
    service = _service(tmp_path)
    connection = await service.connect(
        "searxng",
        credential_ref="env:SEARXNG_API_KEY",
        config={
            "base_url": "https://search.example.test",
            "auth_header": "X-Searxng-Key",
            "auth_scheme": "raw",
        },
    )
    seen_header = ""
    seen_url = ""

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal seen_header, seen_url
        seen_header = request.headers["x-searxng-key"]
        seen_url = str(request.url)
        return httpx.Response(
            200,
            json={
                "results": [
                    {
                        "title": "Local result",
                        "url": "https://example.test/local",
                        "content": "Local snippet",
                    }
                ]
            },
            headers={"content-type": "application/json"},
        )

    specs, handlers = create_integration_tools(
        (connection,),
        EnvCredentialBroker(),
        audit_sink=JsonlAuditSink(tmp_path / "tool-audit.jsonl"),
        http_client_config=HttpClientConfig(trust_env=False),
        http_transport=httpx.MockTransport(handler),
    )
    spec = next(item for item in specs if item.name == "searxng.search")
    executor = FunctionToolExecutor(handlers, InMemoryToolRegistry(specs))

    result = await executor.execute(
        ToolCall(
            call_id="call-1",
            name="searxng.search",
            arguments={"query": "local apps", "max_results": 5},
        )
    )

    payload = json.loads(result.output)
    assert "credential_ref" not in spec.input_schema.get("properties", {})
    assert seen_header == "secret-token"
    assert "https://search.example.test/search?" in seen_url
    assert "q=local+apps" in seen_url
    assert "format=json" in seen_url
    assert payload["results"][0]["title"] == "Local result"
    assert payload["results"][0]["url"] == "https://example.test/local"
    assert "secret-token" not in result.output
    assert "secret-token" not in (tmp_path / "tool-audit.jsonl").read_text(
        encoding="utf-8"
    )


@pytest.mark.asyncio
async def test_opensearch_bearer_tool_uses_brokered_proxy_auth(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("OPENSEARCH_TOKEN", "opensearch-secret-token")
    service = _service(tmp_path)
    connection = await service.connect(
        "opensearch",
        credential_ref="env:OPENSEARCH_TOKEN",
        config={
            "base_url": "https://search.example.test",
            "auth_type": "bearer",
            "auth_header": "X-Proxy-Token",
            "auth_scheme": "raw",
        },
    )
    seen_header = ""
    seen_path = ""
    seen_body: dict[str, object] = {}

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal seen_header, seen_path, seen_body
        seen_header = request.headers["x-proxy-token"]
        seen_path = request.url.path
        seen_body = json.loads(request.content)
        return httpx.Response(
            200,
            json={"hits": {"total": {"value": 1}, "hits": [{"_id": "one"}]}},
            headers={"content-type": "application/json"},
        )

    specs, handlers = create_integration_tools(
        (connection,),
        EnvCredentialBroker(),
        audit_sink=JsonlAuditSink(tmp_path / "tool-audit.jsonl"),
        http_client_config=HttpClientConfig(trust_env=False),
        http_transport=httpx.MockTransport(handler),
    )
    spec = next(item for item in specs if item.name == "opensearch.search")
    executor = FunctionToolExecutor(handlers, InMemoryToolRegistry(specs))

    result = await executor.execute(
        ToolCall(
            call_id="call-1",
            name="opensearch.search",
            arguments={
                "index": "logs-*",
                "query": {"match_all": {}},
                "size": 7,
                "source_includes": ["message"],
                "sort": [{"@timestamp": {"order": "desc"}}],
            },
        )
    )

    assert connection.status == "connected"
    assert "credential_ref" not in spec.input_schema.get("properties", {})
    assert "credential_refs" not in spec.input_schema.get("properties", {})
    assert spec.permissions.approval_required is True
    assert spec.permissions.network == "allow"
    assert seen_header == "opensearch-secret-token"
    assert seen_path == "/logs-*/_search"
    assert seen_body["query"] == {"match_all": {}}
    assert seen_body["size"] == 7
    assert seen_body["_source"] == ["message"]
    assert json.loads(result.output)["result"]["hits"]["hits"][0]["_id"] == "one"
    assert "opensearch-secret-token" not in result.output
    assert "opensearch-secret-token" not in (tmp_path / "tool-audit.jsonl").read_text(
        encoding="utf-8"
    )


@pytest.mark.asyncio
async def test_opensearch_basic_auth_and_document_mutations(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("OPENSEARCH_USER", "colossus-user")
    monkeypatch.setenv("OPENSEARCH_PASSWORD", "colossus-password")
    service = _service(tmp_path)
    connection = await service.connect(
        "opensearch",
        credential_refs={
            "username": "env:OPENSEARCH_USER",
            "password": "env:OPENSEARCH_PASSWORD",
        },
        config={
            "base_url": "https://search.example.test",
            "auth_type": "basic",
        },
    )
    expected_auth = "Basic " + base64.b64encode(
        b"colossus-user:colossus-password"
    ).decode("ascii")
    seen: list[tuple[str, str, dict[str, object] | None, str]] = []

    def handler(request: httpx.Request) -> httpx.Response:
        body = json.loads(request.content) if request.content else None
        seen.append((request.method, request.url.path, body, request.headers["authorization"]))
        return httpx.Response(
            200,
            json={"acknowledged": True},
            headers={"content-type": "application/json"},
        )

    specs, handlers = create_integration_tools(
        (connection,),
        EnvCredentialBroker(),
        audit_sink=JsonlAuditSink(tmp_path / "tool-audit.jsonl"),
        http_client_config=HttpClientConfig(trust_env=False),
        http_transport=httpx.MockTransport(handler),
    )
    index_spec = next(item for item in specs if item.name == "opensearch.index_document")
    update_spec = next(item for item in specs if item.name == "opensearch.update_document")
    delete_spec = next(item for item in specs if item.name == "opensearch.delete_document")
    executor = FunctionToolExecutor(handlers, InMemoryToolRegistry(specs))

    index_result = await executor.execute(
        ToolCall(
            call_id="call-1",
            name="opensearch.index_document",
            arguments={
                "index": "docs",
                "id": "abc",
                "document": {"title": "Hello"},
                "refresh": "wait_for",
            },
        )
    )
    await executor.execute(
        ToolCall(
            call_id="call-2",
            name="opensearch.update_document",
            arguments={
                "index": "docs",
                "id": "abc",
                "doc": {"title": "Updated"},
                "doc_as_upsert": True,
            },
        )
    )
    await executor.execute(
        ToolCall(
            call_id="call-3",
            name="opensearch.delete_document",
            arguments={"index": "docs", "id": "abc"},
        )
    )

    assert connection.status == "connected"
    assert connection.credential_refs == {
        "username": "env:OPENSEARCH_USER",
        "password": "env:OPENSEARCH_PASSWORD",
    }
    assert index_spec.permissions.mutation is True
    assert update_spec.permissions.mutation is True
    assert delete_spec.permissions.mutation is True
    assert index_spec.permissions.risk == "high"
    assert seen[0] == ("PUT", "/docs/_doc/abc", {"title": "Hello"}, expected_auth)
    assert seen[1][0:2] == ("POST", "/docs/_update/abc")
    assert seen[1][2] == {"doc": {"title": "Updated"}, "doc_as_upsert": True}
    assert "script" not in seen[1][2]
    assert seen[2] == ("DELETE", "/docs/_doc/abc", None, expected_auth)
    assert "colossus-password" not in index_result.output
    assert "colossus-password" not in (tmp_path / "tool-audit.jsonl").read_text(
        encoding="utf-8"
    )


@pytest.mark.asyncio
async def test_opensearch_non_json_response_is_wrapped(tmp_path: Path) -> None:
    service = _service(tmp_path)
    connection = await service.connect(
        "opensearch",
        config={"base_url": "https://search.example.test", "auth_type": "none"},
    )

    def handler(_request: httpx.Request) -> httpx.Response:
        return httpx.Response(200, text="plain ok", headers={"content-type": "text/plain"})

    specs, handlers = create_integration_tools(
        (connection,),
        EnvCredentialBroker(),
        http_client_config=HttpClientConfig(trust_env=False),
        http_transport=httpx.MockTransport(handler),
    )
    executor = FunctionToolExecutor(handlers, InMemoryToolRegistry(specs))

    result = await executor.execute(
        ToolCall(call_id="call-1", name="opensearch.info", arguments={})
    )

    assert json.loads(result.output)["result"] == "plain ok"


@pytest.mark.asyncio
async def test_openapi_import_generates_and_executes_brokered_tool(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("API_TOKEN", "openapi-secret")
    spec_path = tmp_path / "openapi.json"
    spec_path.write_text(
        json.dumps(
            {
                "openapi": "3.0.0",
                "info": {"title": "Demo API"},
                "servers": [{"url": "https://api.example.test"}],
                "paths": {
                    "/items/{item_id}": {
                        "get": {
                            "operationId": "getItem",
                            "summary": "Get an item.",
                            "parameters": [
                                {
                                    "name": "item_id",
                                    "in": "path",
                                    "required": True,
                                    "schema": {"type": "string"},
                                }
                            ],
                        }
                    }
                },
            }
        ),
        encoding="utf-8",
    )
    service = _service(tmp_path)
    connection = await service.import_openapi(
        "demo",
        spec_path=spec_path,
        credential_ref="env:API_TOKEN",
        auth_type="bearer",
    )
    seen_authorization = ""

    def handler(request: httpx.Request) -> httpx.Response:
        nonlocal seen_authorization
        seen_authorization = request.headers["authorization"]
        assert request.url.path == "/items/abc"
        return httpx.Response(200, json={"id": "abc"}, headers={"content-type": "application/json"})

    specs, handlers = create_integration_tools(
        (connection,),
        EnvCredentialBroker(),
        http_client_config=HttpClientConfig(trust_env=False),
        http_transport=httpx.MockTransport(handler),
    )
    tool_name = "openapi.demo.getitem"
    spec = next(item for item in specs if item.name == tool_name)
    executor = FunctionToolExecutor(handlers, InMemoryToolRegistry(specs))

    result = await executor.execute(
        ToolCall(call_id="call-1", name=tool_name, arguments={"item_id": "abc"})
    )

    assert "credential_ref" not in spec.input_schema.get("properties", {})
    assert seen_authorization == "Bearer openapi-secret"
    assert "openapi-secret" not in result.output
    assert json.loads(result.output)["result"] == {"id": "abc"}


def test_cli_integrations_connect_updates_tool_catalog(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    monkeypatch.setenv("GITHUB_TOKEN", "super-secret-token")
    runner = CliRunner()

    listed = runner.invoke(app, ["integrations", "list"])
    connected = runner.invoke(
        app,
        ["integrations", "connect", "github", "--credential-ref", "env:GITHUB_TOKEN"],
    )
    tools = runner.invoke(app, ["tools", "list"])

    assert listed.exit_code == 0
    assert "github" in listed.stdout
    assert "available" in listed.stdout
    assert connected.exit_code == 0
    assert "connected" in connected.stdout
    assert "super-secret-token" not in connected.stdout
    assert tools.exit_code == 0
    assert "github.repos" in tools.stdout
    assert "super-secret-token" not in tools.stdout


def test_cli_integrations_connect_searxng_updates_tool_catalog(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    runner = CliRunner()

    connected = runner.invoke(
        app,
        [
            "integrations",
            "connect",
            "searxng",
            "--base-url",
            "http://localhost:8888",
        ],
    )
    tools = runner.invoke(app, ["tools", "list"])

    assert connected.exit_code == 0
    assert "connected" in connected.stdout
    assert tools.exit_code == 0
    assert "searxng.search" in tools.stdout
    assert "searxng.health" in tools.stdout


def test_cli_integrations_connect_opensearch_updates_tool_catalog(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    runner = CliRunner()

    before = runner.invoke(app, ["tools", "list"])
    connected = runner.invoke(
        app,
        [
            "integrations",
            "connect",
            "opensearch",
            "--base-url",
            "http://localhost:9200",
            "--auth-type",
            "none",
        ],
    )
    tools = runner.invoke(app, ["tools", "list"])
    show = runner.invoke(app, ["integrations", "show", "opensearch"])

    assert before.exit_code == 0
    assert "opensearch.search" not in before.stdout
    assert connected.exit_code == 0
    assert "connected" in connected.stdout
    assert tools.exit_code == 0
    assert "opensearch.search" in tools.stdout
    assert "opensearch.index_document" in tools.stdout
    assert show.exit_code == 0
    assert "config.auth_type" in show.stdout
