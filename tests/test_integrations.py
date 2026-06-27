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
    assert github.status == "available"
    assert github.auth_type == "bearer"
    assert "github.repos" in github.tools
    assert connection.status == "pending_auth"
    assert connection.credential_ref is None


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
