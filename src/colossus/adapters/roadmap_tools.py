"""Roadmap built-in tools for planning, repo context, and extensions."""

import difflib
import json
import re
import uuid
from collections.abc import Callable, Iterator
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, cast
from urllib.parse import urlparse

import httpx

from colossus.adapters.subprocess_broker import SubprocessBroker
from colossus.adapters.tool_schema import (
    injected_argument_schema,
    provider_hidden_argument_schema,
)
from colossus.adapters.workspace import Workspace
from colossus.application.decisions import DecisionService
from colossus.application.memories import MemoryService
from colossus.application.subagents import SubagentService
from colossus.application.tasks import TaskService
from colossus.application.tools import ToolHandler
from colossus.domain.decisions import (
    DecisionPriority,
    DecisionSource,
    DecisionStatus,
    KeyDecision,
)
from colossus.domain.errors import ColossusError, ToolExecutionError
from colossus.domain.memories import (
    MemoryItem,
    MemoryKind,
    MemoryScope,
    MemorySource,
    MemoryStatus,
)
from colossus.domain.subagents import SubagentJob, SubagentStatus
from colossus.domain.tasks import Task, TaskStatus
from colossus.domain.tools import ToolPermission, ToolSpec
from colossus.infrastructure.http_client import HttpClientConfig
from colossus.ports.research import McpGateway, SearchProvider

JsonObject = dict[str, Any]
HandlerMap = dict[str, ToolHandler]
ToolCatalogProvider = Callable[[], tuple[ToolSpec, ...]]
HttpTransport = httpx.AsyncBaseTransport

TRACE_FILE = ".colossus_trace.jsonl"
EXCLUDED_REPO_DIRS = frozenset(
    {
        ".git",
        ".hg",
        ".svn",
        ".colossus",
        ".venv",
        ".mypy_cache",
        ".pytest_cache",
        ".ruff_cache",
        "__pycache__",
        "dist",
    }
)
WEB_SEARCH_DISABLED = "web.search requires a configured search adapter."
MCP_DISABLED = "MCP calls require an explicitly configured MCP adapter."


def create_roadmap_tools(
    workspace: Workspace,
    broker: SubprocessBroker,
    catalog_provider: ToolCatalogProvider | None = None,
    http_transport: HttpTransport | None = None,
    task_service: TaskService | None = None,
    decision_service: DecisionService | None = None,
    memory_service: MemoryService | None = None,
    subagent_service: SubagentService | None = None,
    include_agent_delegate: bool = True,
    include_web_search: bool = False,
    include_mcp_call: bool = False,
    search_provider: SearchProvider | None = None,
    mcp_gateway: McpGateway | None = None,
    http_client_config: HttpClientConfig | None = None,
) -> tuple[tuple[ToolSpec, ...], HandlerMap]:
    handlers = RoadmapToolHandlers(
        workspace,
        broker,
        catalog_provider or (lambda: ()),
        http_transport=http_transport,
        task_service=task_service,
        decision_service=decision_service,
        memory_service=memory_service,
        subagent_service=subagent_service,
        search_provider=search_provider,
        mcp_gateway=mcp_gateway,
        http_client_config=http_client_config,
    )
    agent_specs = ((_agent_delegate_spec(),) if include_agent_delegate else ()) + (
        _agent_result_spec(),
        _agent_list_spec(),
    )
    expose_web_search = include_web_search or (
        search_provider is not None and search_provider.configured
    )
    expose_mcp_call = include_mcp_call or (mcp_gateway is not None and mcp_gateway.configured)
    web_search_specs = (_web_search_spec(),) if expose_web_search else ()
    mcp_call_specs = (_mcp_call_spec(),) if expose_mcp_call else ()
    specs = (
        _task_create_spec(),
        _task_update_spec(),
        _task_list_spec(),
        _decision_create_spec(),
        _decision_update_spec(),
        _decision_list_spec(),
        _decision_archive_spec(),
        _decision_supersede_spec(),
        _memory_create_spec(),
        _memory_update_spec(),
        _memory_list_spec(),
        _memory_search_spec(),
        _memory_archive_spec(),
        _memory_supersede_spec(),
        _plan_create_spec(),
        _plan_show_spec(),
        _plan_approve_request_spec(),
        _patch_preview_spec(),
        _patch_apply_spec(),
        _patch_reverse_spec(),
        _repo_map_spec(),
        _repo_symbol_search_spec(),
        _repo_references_spec(),
        _repo_file_summary_spec(),
        *agent_specs,
        _web_fetch_spec(),
        *web_search_specs,
        _docs_fetch_spec(),
        _mcp_servers_spec(),
        _mcp_tools_spec(),
        *mcp_call_specs,
        _tool_search_spec(),
        _trace_show_spec(),
        _trace_export_spec(),
    )
    handlers_by_name: HandlerMap = {
        "task.create": handlers.task_create,
        "task.update": handlers.task_update,
        "task.list": handlers.task_list,
        "decision.create": handlers.decision_create,
        "decision.update": handlers.decision_update,
        "decision.list": handlers.decision_list,
        "decision.archive": handlers.decision_archive,
        "decision.supersede": handlers.decision_supersede,
        "memory.create": handlers.memory_create,
        "memory.update": handlers.memory_update,
        "memory.list": handlers.memory_list,
        "memory.search": handlers.memory_search,
        "memory.archive": handlers.memory_archive,
        "memory.supersede": handlers.memory_supersede,
        "plan.create": handlers.plan_create,
        "plan.show": handlers.plan_show,
        "plan.approve_request": handlers.plan_approve_request,
        "patch.preview": handlers.patch_preview,
        "patch.apply": handlers.patch_apply,
        "patch.reverse": handlers.patch_reverse,
        "repo.map": handlers.repo_map,
        "repo.symbol_search": handlers.repo_symbol_search,
        "repo.references": handlers.repo_references,
        "repo.file_summary": handlers.repo_file_summary,
        "agent.result": handlers.agent_result,
        "agent.list": handlers.agent_list,
        "web.fetch": handlers.web_fetch,
        "docs.fetch": handlers.docs_fetch,
        "mcp.servers": handlers.mcp_servers,
        "mcp.tools": handlers.mcp_tools,
        "tool.search": handlers.tool_search,
        "trace.show": handlers.trace_show,
        "trace.export": handlers.trace_export,
    }
    if include_agent_delegate:
        handlers_by_name["agent.delegate"] = handlers.agent_delegate
    if expose_web_search:
        handlers_by_name["web.search"] = handlers.web_search
    if expose_mcp_call:
        handlers_by_name["mcp.call"] = handlers.mcp_call
    return specs, handlers_by_name


class RoadmapToolHandlers:
    def __init__(
        self,
        workspace: Workspace,
        broker: SubprocessBroker,
        catalog_provider: ToolCatalogProvider,
        http_transport: HttpTransport | None = None,
        task_service: TaskService | None = None,
        decision_service: DecisionService | None = None,
        memory_service: MemoryService | None = None,
        subagent_service: SubagentService | None = None,
        search_provider: SearchProvider | None = None,
        mcp_gateway: McpGateway | None = None,
        http_client_config: HttpClientConfig | None = None,
    ) -> None:
        self._workspace = workspace
        self._broker = broker
        self._catalog_provider = catalog_provider
        self._http_transport = http_transport
        self._task_service = task_service
        self._decision_service = decision_service
        self._memory_service = memory_service
        self._subagent_service = subagent_service
        self._search_provider = search_provider
        self._mcp_gateway = mcp_gateway
        self._http_client_config = http_client_config or HttpClientConfig()
        self._tasks: list[JsonObject] = []
        self._decisions: list[JsonObject] = []
        self._memories: list[JsonObject] = []
        self._plans: list[JsonObject] = []
        self._agents: list[JsonObject] = []

    async def task_create(self, arguments: JsonObject) -> str:
        if self._task_service is not None:
            status = _validated_task_status(_string_arg(arguments, "status", "pending"))
            task_id = _optional_non_empty_string(arguments, "id")
            try:
                created_task = await self._task_service.create_task(
                    session_id=_required_string_arg(arguments, "session_id"),
                    task_id=task_id,
                    title=_required_string_arg(arguments, "title"),
                    description=_string_arg(arguments, "description", ""),
                    status=status,
                )
            except ColossusError as exc:
                raise ToolExecutionError(str(exc)) from exc
            return _json({"task": _task_payload(created_task)})
        task_record: JsonObject = {
            "id": _string_arg(arguments, "id", f"task-{uuid.uuid4().hex[:12]}"),
            "title": _required_string_arg(arguments, "title"),
            "description": _string_arg(arguments, "description", ""),
            "status": _string_arg(arguments, "status", "pending"),
            "created_at": _now(),
            "updated_at": _now(),
        }
        if task_record["status"] not in _task_statuses():
            raise ToolExecutionError("task.create status is not supported.")
        self._tasks.append(task_record)
        return _json({"task": task_record})

    async def task_update(self, arguments: JsonObject) -> str:
        task_id = _required_string_arg(arguments, "id")
        if self._task_service is not None:
            title = _optional_string(arguments, "title")
            description = _optional_string(arguments, "description")
            raw_status = _optional_string(arguments, "status")
            status = _validated_task_status(raw_status) if raw_status is not None else None
            try:
                updated_task = await self._task_service.update_task(
                    task_id,
                    session_id=_optional_non_empty_string(arguments, "session_id"),
                    title=title,
                    description=description,
                    status=status,
                )
            except ColossusError as exc:
                raise ToolExecutionError(str(exc)) from exc
            return _json({"task": _task_payload(updated_task)})
        task_record = _find_record(self._tasks, task_id, "task")
        for field in ("title", "description", "status"):
            value = arguments.get(field)
            if isinstance(value, str) and value:
                if field == "status" and value not in _task_statuses():
                    raise ToolExecutionError("task.update status is not supported.")
                task_record[field] = value
        task_record["updated_at"] = _now()
        return _json({"task": task_record})

    async def task_list(self, arguments: JsonObject) -> str:
        status = _string_arg(arguments, "status", "")
        if self._task_service is not None:
            task_status = _validated_task_status(status) if status else None
            try:
                tasks = await self._task_service.list_tasks(
                    session_id=_required_string_arg(arguments, "session_id"),
                    status=task_status,
                )
            except ColossusError as exc:
                raise ToolExecutionError(str(exc)) from exc
            return _json({"tasks": [_task_payload(task) for task in tasks]})
        records = self._tasks
        if status:
            records = [record for record in records if record.get("status") == status]
        return _json({"tasks": records})

    async def decision_create(self, arguments: JsonObject) -> str:
        source = _validated_decision_source(_string_arg(arguments, "source", "agent"))
        priority = _validated_decision_priority(_string_arg(arguments, "priority", "normal"))
        if self._decision_service is not None:
            try:
                decision = await self._decision_service.create_decision(
                    session_id=_required_string_arg(arguments, "session_id"),
                    decision_id=_optional_non_empty_string(arguments, "id"),
                    title=_required_string_arg(arguments, "title"),
                    decision=_required_string_arg(arguments, "decision"),
                    source=source,
                    priority=priority,
                    rationale=_string_arg(arguments, "rationale", ""),
                    goal_id=_optional_non_empty_string(arguments, "goal_id"),
                    plan_id=_optional_non_empty_string(arguments, "plan_id"),
                    supersedes=_optional_non_empty_string(arguments, "supersedes"),
                )
            except ColossusError as exc:
                raise ToolExecutionError(str(exc)) from exc
            return _json({"decision": _decision_payload(decision)})
        now = _now()
        record: JsonObject = {
            "id": _string_arg(arguments, "id", f"kd_{uuid.uuid4().hex[:12]}"),
            "session_id": _required_string_arg(arguments, "session_id"),
            "goal_id": _optional_non_empty_string(arguments, "goal_id"),
            "plan_id": _optional_non_empty_string(arguments, "plan_id"),
            "source": source,
            "status": "active",
            "priority": priority,
            "title": _required_string_arg(arguments, "title"),
            "decision": _required_string_arg(arguments, "decision"),
            "rationale": _string_arg(arguments, "rationale", ""),
            "supersedes": _optional_non_empty_string(arguments, "supersedes"),
            "created_at": now,
            "updated_at": now,
        }
        self._decisions.append(record)
        return _json({"decision": record})

    async def decision_update(self, arguments: JsonObject) -> str:
        decision_id = _required_string_arg(arguments, "id")
        if self._decision_service is not None:
            raw_priority = _optional_string(arguments, "priority")
            raw_status = _optional_string(arguments, "status")
            try:
                decision = await self._decision_service.update_decision(
                    decision_id,
                    session_id=_optional_non_empty_string(arguments, "session_id"),
                    title=_optional_string(arguments, "title"),
                    decision=_optional_string(arguments, "decision"),
                    priority=(
                        _validated_decision_priority(raw_priority)
                        if raw_priority is not None
                        else None
                    ),
                    rationale=_optional_string(arguments, "rationale"),
                    status=(
                        _validated_decision_status(raw_status) if raw_status is not None else None
                    ),
                    goal_id=_optional_string(arguments, "goal_id"),
                    plan_id=_optional_string(arguments, "plan_id"),
                )
            except ColossusError as exc:
                raise ToolExecutionError(str(exc)) from exc
            return _json({"decision": _decision_payload(decision)})
        record = _find_record(self._decisions, decision_id, "decision")
        for field in ("title", "decision", "rationale", "goal_id", "plan_id"):
            value = arguments.get(field)
            if isinstance(value, str):
                record[field] = value
        if isinstance(arguments.get("priority"), str):
            record["priority"] = _validated_decision_priority(cast(str, arguments["priority"]))
        if isinstance(arguments.get("status"), str):
            record["status"] = _validated_decision_status(cast(str, arguments["status"]))
        record["updated_at"] = _now()
        return _json({"decision": record})

    async def decision_list(self, arguments: JsonObject) -> str:
        raw_status = _optional_string(arguments, "status")
        status = _validated_decision_status(raw_status) if raw_status else "active"
        if self._decision_service is not None:
            try:
                decisions = await self._decision_service.list_decisions(
                    session_id=_required_string_arg(arguments, "session_id"),
                    status=status,
                )
            except ColossusError as exc:
                raise ToolExecutionError(str(exc)) from exc
            return _json({"decisions": [_decision_payload(decision) for decision in decisions]})
        records = [
            record
            for record in self._decisions
            if record.get("session_id") == _required_string_arg(arguments, "session_id")
        ]
        if status:
            records = [record for record in records if record.get("status") == status]
        return _json({"decisions": records})

    async def decision_archive(self, arguments: JsonObject) -> str:
        decision_id = _required_string_arg(arguments, "id")
        if self._decision_service is not None:
            try:
                decision = await self._decision_service.archive_decision(
                    decision_id,
                    session_id=_optional_non_empty_string(arguments, "session_id"),
                )
            except ColossusError as exc:
                raise ToolExecutionError(str(exc)) from exc
            return _json({"decision": _decision_payload(decision)})
        record = _find_record(self._decisions, decision_id, "decision")
        record["status"] = "archived"
        record["updated_at"] = _now()
        return _json({"decision": record})

    async def decision_supersede(self, arguments: JsonObject) -> str:
        decision_id = _required_string_arg(arguments, "id")
        source = _validated_decision_source(_string_arg(arguments, "source", "agent"))
        priority = _validated_decision_priority(_string_arg(arguments, "priority", "normal"))
        if self._decision_service is not None:
            try:
                decision = await self._decision_service.supersede_decision(
                    decision_id,
                    session_id=_optional_non_empty_string(arguments, "session_id"),
                    title=_required_string_arg(arguments, "title"),
                    decision=_required_string_arg(arguments, "decision"),
                    source=source,
                    priority=priority,
                    rationale=_string_arg(arguments, "rationale", ""),
                    goal_id=_optional_non_empty_string(arguments, "goal_id"),
                    plan_id=_optional_non_empty_string(arguments, "plan_id"),
                )
            except ColossusError as exc:
                raise ToolExecutionError(str(exc)) from exc
            return _json({"decision": _decision_payload(decision)})
        old = _find_record(self._decisions, decision_id, "decision")
        old["status"] = "superseded"
        old["updated_at"] = _now()
        now = _now()
        replacement: JsonObject = {
            "id": f"kd_{uuid.uuid4().hex[:12]}",
            "session_id": old["session_id"],
            "goal_id": _optional_non_empty_string(arguments, "goal_id") or old.get("goal_id"),
            "plan_id": _optional_non_empty_string(arguments, "plan_id") or old.get("plan_id"),
            "source": source,
            "status": "active",
            "priority": priority,
            "title": _required_string_arg(arguments, "title"),
            "decision": _required_string_arg(arguments, "decision"),
            "rationale": _string_arg(arguments, "rationale", ""),
            "supersedes": decision_id,
            "created_at": now,
            "updated_at": now,
        }
        self._decisions.append(replacement)
        return _json({"decision": replacement})

    async def memory_create(self, arguments: JsonObject) -> str:
        source = _validated_memory_source(_string_arg(arguments, "source", "agent"))
        scope = _validated_memory_scope(_string_arg(arguments, "scope", "repo"))
        kind = _validated_memory_kind(_string_arg(arguments, "kind", "episode"))
        repo_root = _memory_repo_root(self._workspace, arguments, scope)
        session_id = _optional_non_empty_string(arguments, "session_id")
        if self._memory_service is not None:
            try:
                memory = await self._memory_service.create_memory(
                    memory_id=_optional_non_empty_string(arguments, "id"),
                    scope=scope,
                    kind=kind,
                    text=_required_string_arg(arguments, "text"),
                    source=source,
                    confidence=_float_arg(arguments, "confidence", 1.0),
                    rationale=_string_arg(arguments, "rationale", ""),
                    repo_root=repo_root,
                    session_id=session_id,
                    supersedes=_optional_non_empty_string(arguments, "supersedes"),
                    stale_after=_optional_non_empty_string(arguments, "stale_after"),
                    expires_at=_optional_non_empty_string(arguments, "expires_at"),
                )
            except ColossusError as exc:
                raise ToolExecutionError(str(exc)) from exc
            return _json({"memory": _memory_payload(memory), "notice": _memory_notice(memory)})
        now = _now()
        record: JsonObject = {
            "id": _string_arg(arguments, "id", f"mem_{uuid.uuid4().hex[:12]}"),
            "scope": scope,
            "kind": kind,
            "status": "active",
            "source": source,
            "confidence": _float_arg(arguments, "confidence", 1.0),
            "text": _required_string_arg(arguments, "text"),
            "rationale": _string_arg(arguments, "rationale", ""),
            "repo_root": repo_root,
            "session_id": session_id,
            "supersedes": _optional_non_empty_string(arguments, "supersedes"),
            "stale_after": _optional_non_empty_string(arguments, "stale_after"),
            "expires_at": _optional_non_empty_string(arguments, "expires_at"),
            "created_at": now,
            "updated_at": now,
        }
        self._memories.append(record)
        return _json({"memory": record, "notice": _memory_record_notice(record)})

    async def memory_update(self, arguments: JsonObject) -> str:
        memory_id = _required_string_arg(arguments, "id")
        raw_scope = _optional_string(arguments, "scope")
        raw_kind = _optional_string(arguments, "kind")
        raw_status = _optional_string(arguments, "status")
        scope = _validated_memory_scope(raw_scope) if raw_scope is not None else None
        if self._memory_service is not None:
            try:
                memory = await self._memory_service.update_memory(
                    memory_id,
                    scope=scope,
                    kind=_validated_memory_kind(raw_kind) if raw_kind is not None else None,
                    text=_optional_string(arguments, "text"),
                    status=(
                        _validated_memory_status(raw_status) if raw_status is not None else None
                    ),
                    confidence=_optional_float(arguments, "confidence"),
                    rationale=_optional_string(arguments, "rationale"),
                    repo_root=_memory_repo_root(self._workspace, arguments, scope),
                    session_id=_optional_string(arguments, "session_id"),
                    stale_after=_optional_string(arguments, "stale_after"),
                    expires_at=_optional_string(arguments, "expires_at"),
                )
            except ColossusError as exc:
                raise ToolExecutionError(str(exc)) from exc
            return _json({"memory": _memory_payload(memory), "notice": _memory_notice(memory)})
        record = _find_record(self._memories, memory_id, "memory")
        for field in ("text", "rationale", "repo_root", "session_id", "stale_after", "expires_at"):
            value = arguments.get(field)
            if isinstance(value, str):
                record[field] = value
        if raw_scope is not None:
            record["scope"] = _validated_memory_scope(raw_scope)
        if raw_kind is not None:
            record["kind"] = _validated_memory_kind(raw_kind)
        if raw_status is not None:
            record["status"] = _validated_memory_status(raw_status)
        confidence = _optional_float(arguments, "confidence")
        if confidence is not None:
            record["confidence"] = confidence
        record["updated_at"] = _now()
        return _json({"memory": record, "notice": _memory_record_notice(record)})

    async def memory_list(self, arguments: JsonObject) -> str:
        raw_scope = _optional_string(arguments, "scope")
        raw_kind = _optional_string(arguments, "kind")
        raw_status = _optional_string(arguments, "status")
        scope = _validated_memory_scope(raw_scope) if raw_scope else None
        kind = _validated_memory_kind(raw_kind) if raw_kind else None
        status = _validated_memory_status(raw_status) if raw_status else "active"
        repo_root = _memory_repo_root(self._workspace, arguments, scope)
        session_id = _optional_non_empty_string(arguments, "session_id")
        if self._memory_service is not None:
            if scope is None:
                memories = await self._memory_service.search_memories(
                    "",
                    repo_root=repo_root or str(self._workspace.root),
                    session_id=session_id,
                    kind=kind,
                    status=status,
                    limit=_int_arg(arguments, "limit", 50),
                )
            else:
                memories = await self._memory_service.list_memories(
                    scope=scope,
                    kind=kind,
                    status=status,
                    repo_root=repo_root,
                    session_id=session_id,
                )
            return _json({"memories": [_memory_payload(memory) for memory in memories]})
        records = _filter_memory_records(
            self._memories,
            scope=scope,
            kind=kind,
            status=status,
            repo_root=repo_root or str(self._workspace.root),
            session_id=session_id,
        )
        return _json({"memories": records[: _int_arg(arguments, "limit", 50)]})

    async def memory_search(self, arguments: JsonObject) -> str:
        raw_kind = _optional_string(arguments, "kind")
        raw_status = _optional_string(arguments, "status")
        kind = _validated_memory_kind(raw_kind) if raw_kind else None
        status = _validated_memory_status(raw_status) if raw_status else "active"
        repo_root = _optional_non_empty_string(arguments, "repo_root") or str(self._workspace.root)
        session_id = _optional_non_empty_string(arguments, "session_id")
        query = _required_string_arg(arguments, "query")
        if self._memory_service is not None:
            memories = await self._memory_service.search_memories(
                query,
                repo_root=repo_root,
                session_id=session_id,
                kind=kind,
                status=status,
                limit=_int_arg(arguments, "limit", 8),
            )
            return _json({"memories": [_memory_payload(memory) for memory in memories]})
        query_lower = query.lower()
        records = [
            record
            for record in _filter_memory_records(
                self._memories,
                scope=None,
                kind=kind,
                status=status,
                repo_root=repo_root,
                session_id=session_id,
            )
            if query_lower in str(record.get("text", "")).lower()
        ]
        return _json({"memories": records[: _int_arg(arguments, "limit", 8)]})

    async def memory_archive(self, arguments: JsonObject) -> str:
        memory_id = _required_string_arg(arguments, "id")
        if self._memory_service is not None:
            try:
                memory = await self._memory_service.archive_memory(memory_id)
            except ColossusError as exc:
                raise ToolExecutionError(str(exc)) from exc
            return _json({"memory": _memory_payload(memory)})
        record = _find_record(self._memories, memory_id, "memory")
        record["status"] = "archived"
        record["updated_at"] = _now()
        return _json({"memory": record})

    async def memory_supersede(self, arguments: JsonObject) -> str:
        memory_id = _required_string_arg(arguments, "id")
        raw_scope = _optional_string(arguments, "scope")
        raw_kind = _optional_string(arguments, "kind")
        scope = _validated_memory_scope(raw_scope) if raw_scope else None
        source = _validated_memory_source(_string_arg(arguments, "source", "agent"))
        if self._memory_service is not None:
            try:
                memory = await self._memory_service.supersede_memory(
                    memory_id,
                    text=_required_string_arg(arguments, "text"),
                    source=source,
                    scope=scope,
                    kind=_validated_memory_kind(raw_kind) if raw_kind else None,
                    confidence=_optional_float(arguments, "confidence"),
                    rationale=_string_arg(arguments, "rationale", ""),
                    repo_root=_memory_repo_root(self._workspace, arguments, scope),
                    session_id=_optional_string(arguments, "session_id"),
                    stale_after=_optional_string(arguments, "stale_after"),
                    expires_at=_optional_string(arguments, "expires_at"),
                )
            except ColossusError as exc:
                raise ToolExecutionError(str(exc)) from exc
            return _json({"memory": _memory_payload(memory), "notice": _memory_notice(memory)})
        old = _find_record(self._memories, memory_id, "memory")
        old["status"] = "superseded"
        old["updated_at"] = _now()
        now = _now()
        replacement: JsonObject = {
            "id": f"mem_{uuid.uuid4().hex[:12]}",
            "scope": scope or old["scope"],
            "kind": _validated_memory_kind(raw_kind) if raw_kind else old["kind"],
            "status": "active",
            "source": source,
            "confidence": _optional_float(arguments, "confidence") or old["confidence"],
            "text": _required_string_arg(arguments, "text"),
            "rationale": _string_arg(arguments, "rationale", ""),
            "repo_root": (
                _memory_repo_root(self._workspace, arguments, scope) or old.get("repo_root")
            ),
            "session_id": _optional_string(arguments, "session_id") or old.get("session_id"),
            "supersedes": memory_id,
            "stale_after": _optional_string(arguments, "stale_after") or old.get("stale_after"),
            "expires_at": _optional_string(arguments, "expires_at") or old.get("expires_at"),
            "created_at": now,
            "updated_at": now,
        }
        self._memories.append(replacement)
        return _json({"memory": replacement, "notice": _memory_record_notice(replacement)})

    async def plan_create(self, arguments: JsonObject) -> str:
        prompt = _required_string_arg(arguments, "prompt")
        steps = _string_list_arg(arguments, "steps")
        if not steps:
            steps = [
                "Gather relevant context.",
                "Make the scoped change.",
                "Run verification and summarize results.",
            ]
        plan: JsonObject = {
            "id": _string_arg(arguments, "id", f"plan-{uuid.uuid4().hex[:12]}"),
            "prompt": prompt,
            "status": "draft",
            "approval_requested": False,
            "steps": [
                {"id": f"step-{index}", "title": title, "status": "pending"}
                for index, title in enumerate(steps, start=1)
            ],
            "created_at": _now(),
            "updated_at": _now(),
        }
        self._plans.append(plan)
        return _json({"plan": plan})

    async def plan_show(self, arguments: JsonObject) -> str:
        plan = _find_record(self._plans, _required_string_arg(arguments, "id"), "plan")
        return _json({"plan": plan})

    async def plan_approve_request(self, arguments: JsonObject) -> str:
        plan = _find_record(self._plans, _required_string_arg(arguments, "id"), "plan")
        plan["status"] = "approved"
        plan["approval_requested"] = True
        plan["approved_at"] = _now()
        plan["updated_at"] = _now()
        return _json({"approved": True, "plan": plan})

    async def patch_preview(self, arguments: JsonObject) -> str:
        path = self._workspace.resolve(_required_string_arg(arguments, "path"))
        old_text = _read_text(path)
        updated, replacements = _replace_exact(old_text, arguments)
        return _json(
            {
                "path": self._workspace.relative(path),
                "replacements": replacements,
                "diff": _diff(self._workspace.relative(path), old_text, updated),
            }
        )

    async def patch_apply(self, arguments: JsonObject) -> str:
        path = self._workspace.resolve(_required_string_arg(arguments, "path"))
        old_text = _read_text(path)
        updated, replacements = _replace_exact(old_text, arguments)
        path.write_text(updated, encoding="utf-8")
        relative_path = self._workspace.relative(path)
        return _json(
            {
                "path": relative_path,
                "replacements": replacements,
                "diff": _diff(relative_path, old_text, updated),
                "changed_line_ranges": _changed_line_ranges(old_text, updated),
            }
        )

    async def patch_reverse(self, arguments: JsonObject) -> str:
        reversed_arguments = {
            "old": _required_string_arg(arguments, "new"),
            "new": _required_string_arg(arguments, "old"),
            "replace_all": _bool_arg(arguments, "replace_all", False),
        }
        path = self._workspace.resolve(_required_string_arg(arguments, "path"))
        old_text = _read_text(path)
        updated, replacements = _replace_exact(old_text, reversed_arguments)
        path.write_text(updated, encoding="utf-8")
        relative_path = self._workspace.relative(path)
        return _json(
            {
                "path": relative_path,
                "replacements": replacements,
                "diff": _diff(relative_path, old_text, updated),
                "changed_line_ranges": _changed_line_ranges(old_text, updated),
            }
        )

    async def repo_map(self, arguments: JsonObject) -> str:
        root = self._workspace.resolve(_string_arg(arguments, "path", "."))
        glob_value = _string_arg(arguments, "glob", "**/*")
        max_files = _int_arg(arguments, "max_files", 500)
        files = []
        extensions: dict[str, int] = {}
        for path in _iter_text_files(self._workspace, root, glob_value, max_files):
            suffix = path.suffix or "[none]"
            extensions[suffix] = extensions.get(suffix, 0) + 1
            files.append(
                {
                    "path": self._workspace.relative(path),
                    "size": path.stat().st_size,
                    "extension": suffix,
                }
            )
        return _json(
            {
                "root": self._workspace.relative(root),
                "files": files,
                "extension_counts": extensions,
                "truncated": len(files) >= max_files,
            }
        )

    async def repo_symbol_search(self, arguments: JsonObject) -> str:
        pattern = _required_string_arg(arguments, "pattern")
        regex = _bool_arg(arguments, "regex", False)
        case_sensitive = _bool_arg(arguments, "case_sensitive", True)
        max_matches = _int_arg(arguments, "max_matches", 100)
        root = self._workspace.resolve(_string_arg(arguments, "path", "."))
        matches: list[JsonObject] = []
        for path in _iter_text_files(self._workspace, root, "**/*", max_matches * 20):
            for symbol in _extract_symbols(path, self._workspace):
                haystack = f"{symbol['kind']} {symbol['name']} {symbol['text']}"
                if _matches(haystack, pattern, regex, case_sensitive):
                    matches.append(symbol)
                    if len(matches) >= max_matches:
                        return _json({"symbols": matches, "truncated": True})
        return _json({"symbols": matches, "truncated": False})

    async def repo_references(self, arguments: JsonObject) -> str:
        symbol = _required_string_arg(arguments, "symbol")
        regex = _bool_arg(arguments, "regex", False)
        case_sensitive = _bool_arg(arguments, "case_sensitive", True)
        max_matches = _int_arg(arguments, "max_matches", 100)
        root = self._workspace.resolve(_string_arg(arguments, "path", "."))
        matches: list[JsonObject] = []
        for path in _iter_text_files(self._workspace, root, "**/*", max_matches * 20):
            for line_no, line in enumerate(_read_text(path).splitlines(), start=1):
                if _matches(line, symbol, regex, case_sensitive):
                    matches.append(
                        {"path": self._workspace.relative(path), "line": line_no, "text": line}
                    )
                    if len(matches) >= max_matches:
                        return _json({"references": matches, "truncated": True})
        return _json({"references": matches, "truncated": False})

    async def repo_file_summary(self, arguments: JsonObject) -> str:
        path = self._workspace.resolve(_required_string_arg(arguments, "path"))
        text = _read_text(path)
        lines = text.splitlines()
        imports = [
            line.strip()
            for line in lines
            if line.strip().startswith(("import ", "from ", "const ", "let ", "var "))
        ][:30]
        headings = [line.strip() for line in lines if line.strip().startswith("#")][:20]
        return _json(
            {
                "path": self._workspace.relative(path),
                "bytes": path.stat().st_size,
                "lines": len(lines),
                "imports": imports,
                "headings": headings,
                "symbols": _extract_symbols(path, self._workspace)[:60],
            }
        )

    async def agent_delegate(self, arguments: JsonObject) -> str:
        if self._subagent_service is not None:
            try:
                job = await self._subagent_service.create_job(
                    session_id=_required_string_arg(arguments, "session_id"),
                    parent_run_id=_required_string_arg(arguments, "parent_run_id"),
                    parent_call_id=_required_string_arg(arguments, "parent_call_id"),
                    job_id=_optional_non_empty_string(arguments, "id"),
                    role=_string_arg(arguments, "role", "subagent_default"),
                    task=_required_string_arg(arguments, "task"),
                )
            except ColossusError as exc:
                raise ToolExecutionError(str(exc)) from exc
            return _json({"agent": _subagent_payload(job)})
        agent: JsonObject = {
            "id": _string_arg(arguments, "id", f"agent-{uuid.uuid4().hex[:12]}"),
            "role": _string_arg(arguments, "role", "default"),
            "task": _required_string_arg(arguments, "task"),
            "status": "completed",
            "mutation_allowed": _bool_arg(arguments, "mutation_allowed", False),
            "created_at": _now(),
            "completed_at": _now(),
            "result": (
                "Recorded local subagent delegation request. Remote or parallel child-run "
                "execution is an adapter extension point."
            ),
        }
        self._agents.append(agent)
        return _json({"agent": agent})

    async def agent_result(self, arguments: JsonObject) -> str:
        if self._subagent_service is not None:
            try:
                job = await self._subagent_service.get_job(
                    _required_string_arg(arguments, "id")
                )
            except ColossusError as exc:
                raise ToolExecutionError(str(exc)) from exc
            return _json({"agent": _subagent_payload(job)})
        agent = _find_record(self._agents, _required_string_arg(arguments, "id"), "agent")
        return _json({"agent": agent})

    async def agent_list(self, arguments: JsonObject) -> str:
        if self._subagent_service is not None:
            raw_status = _optional_string(arguments, "status")
            status = _validated_subagent_status(raw_status) if raw_status else None
            try:
                jobs = await self._subagent_service.list_jobs(
                    session_id=_optional_non_empty_string(arguments, "session_id"),
                    status=status,
                )
            except ColossusError as exc:
                raise ToolExecutionError(str(exc)) from exc
            return _json({"agents": [_subagent_payload(job) for job in jobs]})
        legacy_status = _string_arg(arguments, "status", "")
        records = self._agents
        if legacy_status:
            records = [
                record for record in records if record.get("status") == legacy_status
            ]
        return _json({"agents": records})

    async def web_fetch(self, arguments: JsonObject) -> str:
        url = _required_string_arg(arguments, "url")
        max_bytes = _int_arg(arguments, "max_bytes", 200_000)
        return await self._fetch_url(url, max_bytes)

    async def web_search(self, arguments: JsonObject) -> str:
        if self._search_provider is None or not self._search_provider.configured:
            raise ToolExecutionError(WEB_SEARCH_DISABLED)
        drafts = await self._search_provider.collect(
            _required_string_arg(arguments, "query"),
            max_results=_int_arg(arguments, "max_results", 10),
        )
        return _json(
            {
                "results": [
                    {
                        "title": draft.title,
                        "url": draft.uri,
                        "snippet": draft.content,
                        "metadata": draft.metadata,
                    }
                    for draft in drafts
                ]
            }
        )

    async def docs_fetch(self, arguments: JsonObject) -> str:
        url = _required_string_arg(arguments, "url")
        max_bytes = _int_arg(arguments, "max_bytes", 200_000)
        return await self._fetch_url(url, max_bytes)

    async def mcp_servers(self, arguments: JsonObject) -> str:
        _ = arguments
        if self._mcp_gateway is not None and self._mcp_gateway.configured:
            return _json(
                {
                    "servers": list(await self._mcp_gateway.list_servers()),
                    "configured": True,
                    "message": "",
                }
            )
        return _json({"servers": [], "configured": False, "message": MCP_DISABLED})

    async def mcp_tools(self, arguments: JsonObject) -> str:
        if self._mcp_gateway is not None and self._mcp_gateway.configured:
            return _json(
                {
                    "tools": list(
                        await self._mcp_gateway.list_tools(
                            _optional_non_empty_string(arguments, "server")
                        )
                    ),
                    "configured": True,
                    "message": "",
                }
            )
        return _json({"tools": [], "configured": False, "message": MCP_DISABLED})

    async def mcp_call(self, arguments: JsonObject) -> str:
        if self._mcp_gateway is None or not self._mcp_gateway.configured:
            raise ToolExecutionError(MCP_DISABLED)
        result = await self._mcp_gateway.call_tool(
            server=_required_string_arg(arguments, "server"),
            tool=_required_string_arg(arguments, "tool"),
            arguments=_object_arg(arguments, "arguments"),
        )
        return _json({"result": result})

    async def tool_search(self, arguments: JsonObject) -> str:
        query = _required_string_arg(arguments, "query").lower()
        max_results = _int_arg(arguments, "max_results", 20)
        results = []
        for spec in self._catalog_provider():
            haystack = f"{spec.name} {spec.description}".lower()
            if query in haystack:
                results.append(
                    {
                        "name": spec.name,
                        "description": spec.description,
                        "filesystem": spec.permissions.filesystem,
                        "network": spec.permissions.network,
                        "approval_required": (
                            spec.permissions.approval_required or spec.permissions.mutation
                        ),
                        "risk": spec.permissions.risk,
                    }
                )
                if len(results) >= max_results:
                    break
        return _json({"tools": results})

    async def trace_show(self, arguments: JsonObject) -> str:
        path = self._workspace.resolve(_string_arg(arguments, "path", TRACE_FILE))
        max_events = _int_arg(arguments, "max_events", 100)
        if not path.exists():
            return _json({"events": [], "available": False})
        events = []
        for line in path.read_text(encoding="utf-8").splitlines()[:max_events]:
            if not line:
                continue
            try:
                parsed = json.loads(line)
            except json.JSONDecodeError:
                parsed = {"raw": line}
            events.append(parsed)
        return _json({"events": events, "available": True})

    async def trace_export(self, arguments: JsonObject) -> str:
        output = self._workspace.resolve(_required_string_arg(arguments, "path"))
        snapshot = {
            "exported_at": _now(),
            "tasks": self._tasks,
            "plans": self._plans,
            "agents": self._agents,
        }
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(snapshot, indent=2, sort_keys=True), encoding="utf-8")
        return _json(
            {"path": self._workspace.relative(output), "bytes_written": output.stat().st_size}
        )

    async def _fetch_url(self, url: str, max_bytes: int) -> str:
        parsed = urlparse(url)
        if parsed.scheme not in {"http", "https"} or not parsed.netloc:
            raise ToolExecutionError("web.fetch only supports absolute http:// or https:// URLs.")
        chunks: list[bytes] = []
        total = 0
        truncated = False
        headers = {"User-Agent": "colossus-agent/0.1"}
        try:
            async with httpx.AsyncClient(
                **self._http_client_config.async_client_kwargs(
                    follow_redirects=True,
                    timeout=20.0,
                    transport=self._http_transport,
                )
            ) as client, client.stream("GET", url, headers=headers) as response:
                async for chunk in response.aiter_bytes():
                    if not chunk:
                        continue
                    remaining = max_bytes - total
                    if remaining <= 0:
                        truncated = True
                        break
                    chunks.append(chunk[:remaining])
                    total += min(len(chunk), remaining)
                    if len(chunk) > remaining:
                        truncated = True
                        break
                content_type = response.headers.get("content-type", "")
                status_code = response.status_code
                final_url = str(response.url)
        except httpx.RequestError as exc:
            raise ToolExecutionError(f"web.fetch request failed: {exc}") from exc
        content = b"".join(chunks).decode("utf-8", errors="replace")
        return _json(
            {
                "url": final_url,
                "status_code": status_code,
                "content_type": content_type,
                "content": content,
                "truncated": truncated,
            }
        )

def _iter_text_files(
    workspace: Workspace,
    root: Path,
    glob_value: str,
    max_files: int,
) -> Iterator[Path]:
    candidates = [root] if root.is_file() else root.glob(glob_value)
    yielded = 0
    for path in sorted(candidates):
        if yielded >= max_files:
            break
        if not path.is_file() or _is_excluded(workspace, path) or _is_binary(path):
            continue
        yielded += 1
        yield path


def _is_excluded(workspace: Workspace, path: Path) -> bool:
    try:
        relative = Path(workspace.relative(path))
    except ToolExecutionError:
        return True
    return bool(EXCLUDED_REPO_DIRS.intersection(relative.parts))


def _is_binary(path: Path) -> bool:
    try:
        return b"\x00" in path.read_bytes()[:2048]
    except OSError:
        return True


def _extract_symbols(path: Path, workspace: Workspace) -> list[JsonObject]:
    symbols: list[JsonObject] = []
    for line_no, line in enumerate(_read_text(path).splitlines(), start=1):
        stripped = line.strip()
        match = re.match(r"^(async\s+def|def|class)\s+([A-Za-z_][A-Za-z0-9_]*)", stripped)
        if match:
            symbols.append(
                {
                    "path": workspace.relative(path),
                    "line": line_no,
                    "kind": match.group(1).replace("async ", ""),
                    "name": match.group(2),
                    "text": stripped,
                }
            )
            continue
        match = re.match(
            r"^(function|const|let|var)\s+([A-Za-z_$][A-Za-z0-9_$]*)",
            stripped,
        )
        if match:
            symbols.append(
                {
                    "path": workspace.relative(path),
                    "line": line_no,
                    "kind": match.group(1),
                    "name": match.group(2),
                    "text": stripped,
                }
            )
    return symbols


def _replace_exact(text: str, arguments: JsonObject) -> tuple[str, int]:
    old = _required_string_arg(arguments, "old")
    new = _required_string_arg(arguments, "new")
    replace_all = _bool_arg(arguments, "replace_all", False)
    occurrences = text.count(old)
    if occurrences == 0:
        raise ToolExecutionError("Patch old text was not found.")
    if occurrences > 1 and not replace_all:
        raise ToolExecutionError("Patch old text is ambiguous.")
    replacements = occurrences if replace_all else 1
    return text.replace(old, new) if replace_all else text.replace(old, new, 1), replacements


def _diff(path: str, old_text: str, new_text: str) -> str:
    return "".join(
        difflib.unified_diff(
            old_text.splitlines(keepends=True),
            new_text.splitlines(keepends=True),
            fromfile=f"a/{path}",
            tofile=f"b/{path}",
        )
    )


def _changed_line_ranges(old_text: str, new_text: str) -> list[JsonObject]:
    old_lines = old_text.splitlines()
    new_lines = new_text.splitlines()
    ranges: list[JsonObject] = []
    matcher = difflib.SequenceMatcher(a=old_lines, b=new_lines, autojunk=False)
    for tag, _old_start, _old_end, new_start, new_end in matcher.get_opcodes():
        if tag == "equal":
            continue
        if new_start == new_end:
            line = min(new_start + 1, max(len(new_lines), 1))
            ranges.append({"start": line, "end": line})
        else:
            ranges.append({"start": new_start + 1, "end": new_end})
    return _merge_line_ranges(ranges)


def _merge_line_ranges(ranges: list[JsonObject]) -> list[JsonObject]:
    merged: list[JsonObject] = []
    for item in ranges:
        start = int(item["start"])
        end = int(item["end"])
        if merged and start <= int(merged[-1]["end"]) + 1:
            merged[-1]["end"] = max(int(merged[-1]["end"]), end)
        else:
            merged.append({"start": start, "end": end})
    return merged


def _read_text(path: Path) -> str:
    try:
        data = path.read_bytes()
        if b"\x00" in data[:2048]:
            raise ToolExecutionError("Binary-looking files are not supported.")
        return data.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise ToolExecutionError("Only UTF-8 text files are supported.") from exc


def _find_record(records: list[JsonObject], record_id: str, label: str) -> JsonObject:
    for record in records:
        if record.get("id") == record_id:
            return record
    raise ToolExecutionError(f"Unknown {label}: {record_id}")


def _matches(text: str, pattern: str, regex: bool, case_sensitive: bool) -> bool:
    if regex:
        flags = 0 if case_sensitive else re.IGNORECASE
        return bool(re.search(pattern, text, flags))
    if case_sensitive:
        return pattern in text
    return pattern.lower() in text.lower()


def _required_string_arg(arguments: JsonObject, name: str) -> str:
    value = arguments.get(name)
    if not isinstance(value, str) or not value:
        raise ToolExecutionError(f"Argument {name} must be a non-empty string.")
    return value


def _string_arg(arguments: JsonObject, name: str, default: str) -> str:
    value = arguments.get(name, default)
    return value if isinstance(value, str) else default


def _optional_string(arguments: JsonObject, name: str) -> str | None:
    value = arguments.get(name)
    return value if isinstance(value, str) else None


def _optional_non_empty_string(arguments: JsonObject, name: str) -> str | None:
    value = _optional_string(arguments, name)
    if value is None or not value:
        return None
    return value


def _bool_arg(arguments: JsonObject, name: str, default: bool) -> bool:
    value = arguments.get(name, default)
    return value if isinstance(value, bool) else default


def _int_arg(arguments: JsonObject, name: str, default: int) -> int:
    value = arguments.get(name, default)
    return value if isinstance(value, int) else default


def _float_arg(arguments: JsonObject, name: str, default: float) -> float:
    value = arguments.get(name, default)
    if isinstance(value, int | float):
        return float(value)
    return default


def _optional_float(arguments: JsonObject, name: str) -> float | None:
    value = arguments.get(name)
    if isinstance(value, int | float):
        return float(value)
    return None


def _string_list_arg(arguments: JsonObject, name: str) -> list[str]:
    value = arguments.get(name, [])
    if value is None:
        return []
    if not isinstance(value, list):
        raise ToolExecutionError(f"Argument {name} must be an array of strings.")
    if not all(isinstance(item, str) for item in value):
        raise ToolExecutionError(f"Argument {name} must be an array of strings.")
    return value


def _object_arg(arguments: JsonObject, name: str) -> JsonObject:
    value = arguments.get(name, {})
    if not isinstance(value, dict):
        raise ToolExecutionError(f"Argument {name} must be an object.")
    return {str(key): item for key, item in value.items()}


def _now() -> str:
    return datetime.now(tz=UTC).isoformat()


def _json(value: JsonObject) -> str:
    return json.dumps(value, sort_keys=True)


def _task_statuses() -> set[str]:
    return {"pending", "in_progress", "completed", "blocked", "cancelled"}


def _validated_task_status(value: str) -> TaskStatus:
    if value not in _task_statuses():
        raise ToolExecutionError("Task status is not supported.")
    return cast(TaskStatus, value)


def _task_payload(task: Task) -> JsonObject:
    return task.model_dump(mode="json")


def _decision_sources() -> set[str]:
    return {"user", "agent"}


def _validated_decision_source(value: str) -> DecisionSource:
    if value not in _decision_sources():
        raise ToolExecutionError("Decision source is not supported.")
    return cast(DecisionSource, value)


def _decision_statuses() -> set[str]:
    return {"active", "archived", "superseded"}


def _validated_decision_status(value: str) -> DecisionStatus:
    if value not in _decision_statuses():
        raise ToolExecutionError("Decision status is not supported.")
    return cast(DecisionStatus, value)


def _decision_priorities() -> set[str]:
    return {"critical", "high", "normal"}


def _validated_decision_priority(value: str) -> DecisionPriority:
    if value not in _decision_priorities():
        raise ToolExecutionError("Decision priority is not supported.")
    return cast(DecisionPriority, value)


def _decision_payload(decision: KeyDecision) -> JsonObject:
    return decision.model_dump(mode="json")


def _memory_scopes() -> set[str]:
    return {"global", "repo", "session"}


def _validated_memory_scope(value: str) -> MemoryScope:
    if value not in _memory_scopes():
        raise ToolExecutionError("Memory scope is not supported.")
    return cast(MemoryScope, value)


def _memory_kinds() -> set[str]:
    return {"preference", "project_fact", "episode", "capability", "warning"}


def _validated_memory_kind(value: str) -> MemoryKind:
    if value not in _memory_kinds():
        raise ToolExecutionError("Memory kind is not supported.")
    return cast(MemoryKind, value)


def _memory_statuses() -> set[str]:
    return {"active", "archived", "superseded"}


def _validated_memory_status(value: str) -> MemoryStatus:
    if value not in _memory_statuses():
        raise ToolExecutionError("Memory status is not supported.")
    return cast(MemoryStatus, value)


def _validated_memory_source(value: str) -> MemorySource:
    if value not in _decision_sources():
        raise ToolExecutionError("Memory source is not supported.")
    return cast(MemorySource, value)


def _memory_repo_root(
    workspace: Workspace,
    arguments: JsonObject,
    scope: MemoryScope | None,
) -> str | None:
    value = _optional_non_empty_string(arguments, "repo_root")
    if value is not None:
        return value
    if scope == "repo":
        return str(workspace.root)
    return None


def _memory_payload(memory: MemoryItem) -> JsonObject:
    return memory.model_dump(mode="json")


def _memory_notice(memory: MemoryItem) -> str:
    return f"Saved memory {memory.id} [{memory.scope}/{memory.kind}]"


def _memory_record_notice(record: JsonObject) -> str:
    return f"Saved memory {record['id']} [{record['scope']}/{record['kind']}]"


def _filter_memory_records(
    records: list[JsonObject],
    *,
    scope: MemoryScope | None,
    kind: MemoryKind | None,
    status: MemoryStatus | None,
    repo_root: str | None,
    session_id: str | None,
) -> list[JsonObject]:
    filtered: list[JsonObject] = []
    for record in records:
        if scope is not None and record.get("scope") != scope:
            continue
        if kind is not None and record.get("kind") != kind:
            continue
        if status is not None and record.get("status") != status:
            continue
        record_scope = record.get("scope")
        if record_scope == "repo" and record.get("repo_root") != repo_root:
            continue
        if record_scope == "session" and record.get("session_id") != session_id:
            continue
        filtered.append(record)
    return filtered


def _subagent_statuses() -> set[str]:
    return {"queued", "running", "completed", "failed", "cancelled", "interrupted"}


def _validated_subagent_status(value: str) -> SubagentStatus:
    if value not in _subagent_statuses():
        raise ToolExecutionError("Subagent status is not supported.")
    return cast(SubagentStatus, value)


def _subagent_payload(job: SubagentJob) -> JsonObject:
    return job.model_dump(mode="json")


def _object_schema(properties: JsonObject, required: list[str] | None = None) -> JsonObject:
    return {
        "type": "object",
        "properties": properties,
        "required": required or [],
        "additionalProperties": False,
    }


def _array_of_strings() -> JsonObject:
    return {"type": "array", "items": {"type": "string"}}


def _write_permission() -> ToolPermission:
    return ToolPermission(
        filesystem="write",
        approval_required=True,
        mutation=True,
        risk="high",
    )


def _network_permission() -> ToolPermission:
    return ToolPermission(
        filesystem="none",
        network="allow",
        approval_required=True,
        mutation=False,
        working_root_required=False,
        risk="high",
    )


def _task_create_spec() -> ToolSpec:
    return ToolSpec(
        name="task.create",
        description="Create a model-visible task record for progress tracking.",
        input_schema=_object_schema(
            {
                "id": {"type": "string"},
                "session_id": {"type": "string"},
                "title": {"type": "string", "minLength": 1},
                "description": {"type": "string"},
                "status": {"type": "string", "enum": sorted(_task_statuses())},
            },
            ["title"],
        ),
        output_schema=_object_schema({"task": {"type": "object"}}),
        permissions=ToolPermission(filesystem="read", risk="low"),
    )


def _task_update_spec() -> ToolSpec:
    return ToolSpec(
        name="task.update",
        description="Update an existing task status or detail.",
        input_schema=_object_schema(
            {
                "id": {"type": "string", "minLength": 1},
                "session_id": {"type": "string"},
                "title": {"type": "string"},
                "description": {"type": "string"},
                "status": {"type": "string", "enum": sorted(_task_statuses())},
            },
            ["id"],
        ),
        output_schema=_object_schema({"task": {"type": "object"}}),
        permissions=ToolPermission(filesystem="read", risk="low"),
    )


def _task_list_spec() -> ToolSpec:
    return ToolSpec(
        name="task.list",
        description="List model-visible task records.",
        input_schema=_object_schema(
            {
                "session_id": {"type": "string"},
                "status": {"type": "string", "enum": sorted(_task_statuses())},
            }
        ),
        output_schema=_object_schema({"tasks": {"type": "array"}}),
        permissions=ToolPermission(working_root_required=False, risk="low"),
    )


def _decision_permission() -> ToolPermission:
    return ToolPermission(
        filesystem="none",
        approval_required=False,
        mutation=True,
        working_root_required=False,
        risk="medium",
    )


def _decision_create_spec() -> ToolSpec:
    return ToolSpec(
        name="decision.create",
        description="Create an active durable key decision for this session.",
        input_schema=_object_schema(
            {
                "id": {"type": "string"},
                "session_id": injected_argument_schema({"type": "string"}),
                "goal_id": {"type": "string"},
                "plan_id": {"type": "string"},
                "source": {"type": "string", "enum": sorted(_decision_sources())},
                "priority": {"type": "string", "enum": sorted(_decision_priorities())},
                "title": {"type": "string", "minLength": 1},
                "decision": {"type": "string", "minLength": 1},
                "rationale": {"type": "string"},
                "supersedes": {"type": "string"},
            },
            ["title", "decision"],
        ),
        output_schema=_object_schema({"decision": {"type": "object"}}),
        permissions=_decision_permission(),
    )


def _decision_update_spec() -> ToolSpec:
    return ToolSpec(
        name="decision.update",
        description="Update an existing durable key decision.",
        input_schema=_object_schema(
            {
                "id": {"type": "string", "minLength": 1},
                "session_id": injected_argument_schema({"type": "string"}),
                "goal_id": {"type": "string"},
                "plan_id": {"type": "string"},
                "status": {"type": "string", "enum": sorted(_decision_statuses())},
                "priority": {"type": "string", "enum": sorted(_decision_priorities())},
                "title": {"type": "string"},
                "decision": {"type": "string"},
                "rationale": {"type": "string"},
            },
            ["id"],
        ),
        output_schema=_object_schema({"decision": {"type": "object"}}),
        permissions=_decision_permission(),
    )


def _decision_list_spec() -> ToolSpec:
    return ToolSpec(
        name="decision.list",
        description="List durable key decisions for this session.",
        input_schema=_object_schema(
            {
                "session_id": injected_argument_schema({"type": "string"}),
                "status": {"type": "string", "enum": sorted(_decision_statuses())},
            }
        ),
        output_schema=_object_schema({"decisions": {"type": "array"}}),
        permissions=ToolPermission(working_root_required=False, risk="low"),
    )


def _decision_archive_spec() -> ToolSpec:
    return ToolSpec(
        name="decision.archive",
        description="Archive an active durable key decision.",
        input_schema=_object_schema(
            {
                "id": {"type": "string", "minLength": 1},
                "session_id": injected_argument_schema({"type": "string"}),
            },
            ["id"],
        ),
        output_schema=_object_schema({"decision": {"type": "object"}}),
        permissions=_decision_permission(),
    )


def _decision_supersede_spec() -> ToolSpec:
    return ToolSpec(
        name="decision.supersede",
        description="Supersede a durable key decision with a new active decision.",
        input_schema=_object_schema(
            {
                "id": {"type": "string", "minLength": 1},
                "session_id": injected_argument_schema({"type": "string"}),
                "goal_id": {"type": "string"},
                "plan_id": {"type": "string"},
                "source": {"type": "string", "enum": sorted(_decision_sources())},
                "priority": {"type": "string", "enum": sorted(_decision_priorities())},
                "title": {"type": "string", "minLength": 1},
                "decision": {"type": "string", "minLength": 1},
                "rationale": {"type": "string"},
            },
            ["id", "title", "decision"],
        ),
        output_schema=_object_schema({"decision": {"type": "object"}}),
        permissions=_decision_permission(),
    )


def _memory_permission() -> ToolPermission:
    return ToolPermission(
        filesystem="none",
        approval_required=False,
        mutation=False,
        working_root_required=False,
        risk="medium",
    )


def _memory_common_properties() -> JsonObject:
    return {
        "scope": {"type": "string", "enum": sorted(_memory_scopes())},
        "kind": {"type": "string", "enum": sorted(_memory_kinds())},
        "source": {"type": "string", "enum": sorted(_decision_sources())},
        "confidence": {"type": "number", "minimum": 0.0, "maximum": 1.0},
        "text": {"type": "string", "minLength": 1},
        "rationale": {"type": "string"},
        "repo_root": provider_hidden_argument_schema({"type": "string"}),
        "session_id": injected_argument_schema({"type": "string"}),
        "supersedes": {"type": "string"},
        "stale_after": {"type": "string"},
        "expires_at": {"type": "string"},
    }


def _memory_create_spec() -> ToolSpec:
    return ToolSpec(
        name="memory.create",
        description="Create an active durable memory as relevant context, not a commitment.",
        input_schema=_object_schema(
            {
                "id": {"type": "string"},
                **_memory_common_properties(),
            },
            ["text"],
        ),
        output_schema=_object_schema(
            {"memory": {"type": "object"}, "notice": {"type": "string"}}
        ),
        permissions=_memory_permission(),
    )


def _memory_update_spec() -> ToolSpec:
    return ToolSpec(
        name="memory.update",
        description="Update an existing durable memory.",
        input_schema=_object_schema(
            {
                "id": {"type": "string", "minLength": 1},
                "status": {"type": "string", "enum": sorted(_memory_statuses())},
                **_memory_common_properties(),
            },
            ["id"],
        ),
        output_schema=_object_schema(
            {"memory": {"type": "object"}, "notice": {"type": "string"}}
        ),
        permissions=_memory_permission(),
    )


def _memory_list_spec() -> ToolSpec:
    return ToolSpec(
        name="memory.list",
        description="List durable memories relevant to the current session or repository.",
        input_schema=_object_schema(
            {
                "scope": {"type": "string", "enum": sorted(_memory_scopes())},
                "kind": {"type": "string", "enum": sorted(_memory_kinds())},
                "status": {"type": "string", "enum": sorted(_memory_statuses())},
                "repo_root": provider_hidden_argument_schema({"type": "string"}),
                "session_id": injected_argument_schema({"type": "string"}),
                "limit": {"type": "integer", "minimum": 1, "maximum": 100},
            }
        ),
        output_schema=_object_schema({"memories": {"type": "array"}}),
        permissions=ToolPermission(working_root_required=False, risk="low"),
    )


def _memory_search_spec() -> ToolSpec:
    return ToolSpec(
        name="memory.search",
        description="Search durable memories using the configured memory index.",
        input_schema=_object_schema(
            {
                "query": {"type": "string", "minLength": 1},
                "kind": {"type": "string", "enum": sorted(_memory_kinds())},
                "status": {"type": "string", "enum": sorted(_memory_statuses())},
                "repo_root": provider_hidden_argument_schema({"type": "string"}),
                "session_id": injected_argument_schema({"type": "string"}),
                "limit": {"type": "integer", "minimum": 1, "maximum": 50},
            },
            ["query"],
        ),
        output_schema=_object_schema({"memories": {"type": "array"}}),
        permissions=ToolPermission(working_root_required=False, risk="low"),
    )


def _memory_archive_spec() -> ToolSpec:
    return ToolSpec(
        name="memory.archive",
        description="Archive a durable memory so it is no longer injected.",
        input_schema=_object_schema({"id": {"type": "string", "minLength": 1}}, ["id"]),
        output_schema=_object_schema({"memory": {"type": "object"}}),
        permissions=_memory_permission(),
    )


def _memory_supersede_spec() -> ToolSpec:
    return ToolSpec(
        name="memory.supersede",
        description="Supersede a durable memory with a new active memory.",
        input_schema=_object_schema(
            {
                "id": {"type": "string", "minLength": 1},
                "scope": {"type": "string", "enum": sorted(_memory_scopes())},
                "kind": {"type": "string", "enum": sorted(_memory_kinds())},
                "source": {"type": "string", "enum": sorted(_decision_sources())},
                "confidence": {"type": "number", "minimum": 0.0, "maximum": 1.0},
                "text": {"type": "string", "minLength": 1},
                "rationale": {"type": "string"},
                "repo_root": provider_hidden_argument_schema({"type": "string"}),
                "session_id": injected_argument_schema({"type": "string"}),
                "stale_after": {"type": "string"},
                "expires_at": {"type": "string"},
            },
            ["id", "text"],
        ),
        output_schema=_object_schema(
            {"memory": {"type": "object"}, "notice": {"type": "string"}}
        ),
        permissions=_memory_permission(),
    )


def _plan_create_spec() -> ToolSpec:
    return ToolSpec(
        name="plan.create",
        description="Create a model-callable execution plan.",
        input_schema=_object_schema(
            {
                "id": {"type": "string"},
                "prompt": {"type": "string", "minLength": 1},
                "steps": _array_of_strings(),
            },
            ["prompt"],
        ),
        output_schema=_object_schema({"plan": {"type": "object"}}),
        permissions=ToolPermission(working_root_required=False, risk="low"),
    )


def _plan_show_spec() -> ToolSpec:
    return ToolSpec(
        name="plan.show",
        description="Show a model-callable execution plan.",
        input_schema=_object_schema({"id": {"type": "string", "minLength": 1}}, ["id"]),
        output_schema=_object_schema({"plan": {"type": "object"}}),
        permissions=ToolPermission(working_root_required=False, risk="low"),
    )


def _plan_approve_request_spec() -> ToolSpec:
    return ToolSpec(
        name="plan.approve_request",
        description=(
            "Request approval for a model-created plan and mark it approved after approval."
        ),
        input_schema=_object_schema({"id": {"type": "string", "minLength": 1}}, ["id"]),
        output_schema=_object_schema({"approved": {"type": "boolean"}, "plan": {"type": "object"}}),
        permissions=ToolPermission(
            approval_required=True,
            mutation=True,
            working_root_required=False,
            risk="medium",
        ),
    )


def _patch_preview_spec() -> ToolSpec:
    return ToolSpec(
        name="patch.preview",
        description="Preview an exact text patch as a unified diff without writing files.",
        input_schema=_patch_schema(),
        output_schema=_object_schema(
            {
                "path": {"type": "string"},
                "replacements": {"type": "integer"},
                "diff": {"type": "string"},
            }
        ),
        permissions=ToolPermission(filesystem="read", risk="medium"),
    )


def _patch_apply_spec() -> ToolSpec:
    return ToolSpec(
        name="patch.apply",
        description="Apply an exact text patch inside the workspace.",
        input_schema=_patch_schema(),
        output_schema=_object_schema(
            {
                "path": {"type": "string"},
                "replacements": {"type": "integer"},
                "diff": {"type": "string"},
                "changed_line_ranges": {"type": "array"},
            }
        ),
        permissions=_write_permission(),
    )


def _patch_reverse_spec() -> ToolSpec:
    return ToolSpec(
        name="patch.reverse",
        description="Reverse an exact text patch inside the workspace.",
        input_schema=_patch_schema(),
        output_schema=_object_schema(
            {
                "path": {"type": "string"},
                "replacements": {"type": "integer"},
                "diff": {"type": "string"},
                "changed_line_ranges": {"type": "array"},
            }
        ),
        permissions=_write_permission(),
    )


def _patch_schema() -> JsonObject:
    return _object_schema(
        {
            "path": {"type": "string", "minLength": 1},
            "old": {"type": "string", "minLength": 1},
            "new": {"type": "string", "minLength": 1},
            "replace_all": {"type": "boolean", "default": False},
        },
        ["path", "old", "new"],
    )


def _repo_map_spec() -> ToolSpec:
    return ToolSpec(
        name="repo.map",
        description="Return a compact workspace file map for local context building.",
        input_schema=_object_schema(
            {
                "path": {"type": "string"},
                "glob": {"type": "string", "default": "**/*"},
                "max_files": {"type": "integer", "minimum": 1, "maximum": 5000},
            }
        ),
        output_schema=_object_schema(
            {
                "root": {"type": "string"},
                "files": {"type": "array"},
                "extension_counts": {"type": "object"},
                "truncated": {"type": "boolean"},
            }
        ),
        permissions=ToolPermission(filesystem="read", risk="low"),
    )


def _repo_symbol_search_spec() -> ToolSpec:
    return ToolSpec(
        name="repo.symbol_search",
        description="Search extracted local code symbols.",
        input_schema=_object_schema(
            {
                "pattern": {"type": "string", "minLength": 1},
                "path": {"type": "string"},
                "regex": {"type": "boolean", "default": False},
                "case_sensitive": {"type": "boolean", "default": True},
                "max_matches": {"type": "integer", "minimum": 1, "maximum": 1000},
            },
            ["pattern"],
        ),
        output_schema=_object_schema(
            {"symbols": {"type": "array"}, "truncated": {"type": "boolean"}}
        ),
        permissions=ToolPermission(filesystem="read", risk="low"),
    )


def _repo_references_spec() -> ToolSpec:
    return ToolSpec(
        name="repo.references",
        description="Find local references to a symbol or text pattern.",
        input_schema=_object_schema(
            {
                "symbol": {"type": "string", "minLength": 1},
                "path": {"type": "string"},
                "regex": {"type": "boolean", "default": False},
                "case_sensitive": {"type": "boolean", "default": True},
                "max_matches": {"type": "integer", "minimum": 1, "maximum": 1000},
            },
            ["symbol"],
        ),
        output_schema=_object_schema(
            {"references": {"type": "array"}, "truncated": {"type": "boolean"}}
        ),
        permissions=ToolPermission(filesystem="read", risk="low"),
    )


def _repo_file_summary_spec() -> ToolSpec:
    return ToolSpec(
        name="repo.file_summary",
        description="Summarize a UTF-8 source or documentation file.",
        input_schema=_object_schema({"path": {"type": "string", "minLength": 1}}, ["path"]),
        output_schema=_object_schema(
            {
                "path": {"type": "string"},
                "bytes": {"type": "integer"},
                "lines": {"type": "integer"},
                "imports": {"type": "array"},
                "headings": {"type": "array"},
                "symbols": {"type": "array"},
            }
        ),
        permissions=ToolPermission(filesystem="read", risk="low"),
    )


def _agent_delegate_spec() -> ToolSpec:
    return ToolSpec(
        name="agent.delegate",
        description="Queue a durable local subagent job.",
        input_schema=_object_schema(
            {
                "id": {"type": "string"},
                "role": provider_hidden_argument_schema({"type": "string"}),
                "task": {"type": "string", "minLength": 1},
                "mutation_allowed": {"type": "boolean", "default": False},
                "session_id": injected_argument_schema({"type": "string"}),
                "parent_run_id": injected_argument_schema({"type": "string"}),
                "parent_call_id": injected_argument_schema({"type": "string"}),
            },
            ["task"],
        ),
        output_schema=_object_schema({"agent": {"type": "object"}}),
        permissions=ToolPermission(working_root_required=False, risk="medium"),
    )


def _agent_result_spec() -> ToolSpec:
    return ToolSpec(
        name="agent.result",
        description="Return a durable local subagent job result.",
        input_schema=_object_schema(
            {
                "id": {"type": "string", "minLength": 1},
                "session_id": injected_argument_schema({"type": "string"}),
            },
            ["id"],
        ),
        output_schema=_object_schema({"agent": {"type": "object"}}),
        permissions=ToolPermission(working_root_required=False, risk="low"),
    )


def _agent_list_spec() -> ToolSpec:
    return ToolSpec(
        name="agent.list",
        description="List durable local subagent jobs.",
        input_schema=_object_schema(
            {
                "status": {"type": "string"},
                "session_id": injected_argument_schema({"type": "string"}),
            }
        ),
        output_schema=_object_schema({"agents": {"type": "array"}}),
        permissions=ToolPermission(working_root_required=False, risk="low"),
    )


def _web_fetch_spec() -> ToolSpec:
    return ToolSpec(
        name="web.fetch",
        description="Fetch an HTTP or HTTPS URL after explicit network approval.",
        input_schema=_object_schema(
            {
                "url": {"type": "string", "minLength": 1},
                "max_bytes": {"type": "integer", "minimum": 1, "maximum": 200000},
            },
            ["url"],
        ),
        output_schema=_object_schema(
            {
                "url": {"type": "string"},
                "status_code": {"type": "integer"},
                "content_type": {"type": "string"},
                "content": {"type": "string"},
                "truncated": {"type": "boolean"},
            }
        ),
        permissions=_network_permission(),
    )


def _web_search_spec() -> ToolSpec:
    return ToolSpec(
        name="web.search",
        description="Search the web when a network-enabled profile is installed.",
        input_schema=_object_schema(
            {
                "query": {"type": "string", "minLength": 1},
                "max_results": {"type": "integer", "minimum": 1, "maximum": 20},
            },
            ["query"],
        ),
        output_schema=_object_schema({"results": {"type": "array"}}),
        permissions=_network_permission(),
    )


def _docs_fetch_spec() -> ToolSpec:
    return ToolSpec(
        name="docs.fetch",
        description="Fetch an HTTP or HTTPS documentation URL after explicit network approval.",
        input_schema=_object_schema(
            {
                "url": {"type": "string", "minLength": 1},
                "max_bytes": {"type": "integer", "minimum": 1, "maximum": 200000},
            },
            ["url"],
        ),
        output_schema=_object_schema(
            {
                "url": {"type": "string"},
                "status_code": {"type": "integer"},
                "content_type": {"type": "string"},
                "content": {"type": "string"},
                "truncated": {"type": "boolean"},
            }
        ),
        permissions=_network_permission(),
    )


def _mcp_servers_spec() -> ToolSpec:
    return ToolSpec(
        name="mcp.servers",
        description="List configured MCP servers.",
        input_schema=_object_schema({}),
        output_schema=_object_schema(
            {
                "servers": {"type": "array"},
                "configured": {"type": "boolean"},
                "message": {"type": "string"},
            }
        ),
        permissions=ToolPermission(working_root_required=False, risk="low"),
    )


def _mcp_tools_spec() -> ToolSpec:
    return ToolSpec(
        name="mcp.tools",
        description="List tools exposed by a configured MCP server.",
        input_schema=_object_schema({"server": {"type": "string"}}),
        output_schema=_object_schema(
            {
                "tools": {"type": "array"},
                "configured": {"type": "boolean"},
                "message": {"type": "string"},
            }
        ),
        permissions=ToolPermission(working_root_required=False, risk="low"),
    )


def _mcp_call_spec() -> ToolSpec:
    return ToolSpec(
        name="mcp.call",
        description="Call an MCP tool through a configured MCP adapter.",
        input_schema=_object_schema(
            {
                "server": {"type": "string", "minLength": 1},
                "tool": {"type": "string", "minLength": 1},
                "arguments": {"type": "object"},
            },
            ["server", "tool", "arguments"],
        ),
        output_schema=_object_schema({"result": {"type": "object"}}),
        permissions=ToolPermission(
            network="allow",
            approval_required=True,
            mutation=True,
            working_root_required=False,
            risk="high",
        ),
    )


def _tool_search_spec() -> ToolSpec:
    return ToolSpec(
        name="tool.search",
        description="Search the registered Colossus tool catalog.",
        input_schema=_object_schema(
            {
                "query": {"type": "string", "minLength": 1},
                "max_results": {"type": "integer", "minimum": 1, "maximum": 100},
            },
            ["query"],
        ),
        output_schema=_object_schema({"tools": {"type": "array"}}),
        permissions=ToolPermission(working_root_required=False, risk="low"),
    )


def _trace_show_spec() -> ToolSpec:
    return ToolSpec(
        name="trace.show",
        description="Show a bounded workspace trace JSONL file.",
        input_schema=_object_schema(
            {
                "path": {"type": "string"},
                "max_events": {"type": "integer", "minimum": 1, "maximum": 1000},
            }
        ),
        output_schema=_object_schema(
            {"events": {"type": "array"}, "available": {"type": "boolean"}}
        ),
        permissions=ToolPermission(filesystem="read", risk="low"),
    )


def _trace_export_spec() -> ToolSpec:
    return ToolSpec(
        name="trace.export",
        description=(
            "Export a workspace trace snapshot containing tool task, plan, and agent metadata."
        ),
        input_schema=_object_schema({"path": {"type": "string", "minLength": 1}}, ["path"]),
        output_schema=_object_schema(
            {"path": {"type": "string"}, "bytes_written": {"type": "integer"}}
        ),
        permissions=_write_permission(),
    )
