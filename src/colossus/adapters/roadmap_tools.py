"""Roadmap built-in tools for planning, verification, repo context, and extensions."""

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

from colossus.adapters.subprocess_broker import (
    SubprocessBroker,
    SubprocessCommand,
)
from colossus.adapters.workspace import Workspace
from colossus.application.tasks import TaskService
from colossus.application.tools import ToolHandler
from colossus.domain.errors import ColossusError, ToolExecutionError
from colossus.domain.tasks import Task, TaskStatus
from colossus.domain.tools import ToolPermission, ToolSpec

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
) -> tuple[tuple[ToolSpec, ...], HandlerMap]:
    handlers = RoadmapToolHandlers(
        workspace,
        broker,
        catalog_provider or (lambda: ()),
        http_transport=http_transport,
        task_service=task_service,
    )
    specs = (
        _task_create_spec(),
        _task_update_spec(),
        _task_list_spec(),
        _plan_create_spec(),
        _plan_show_spec(),
        _plan_approve_request_spec(),
        _test_run_spec(),
        _lint_run_spec(),
        _typecheck_run_spec(),
        _build_run_spec(),
        _patch_preview_spec(),
        _patch_apply_spec(),
        _patch_reverse_spec(),
        _repo_map_spec(),
        _repo_symbol_search_spec(),
        _repo_references_spec(),
        _repo_file_summary_spec(),
        _agent_delegate_spec(),
        _agent_result_spec(),
        _agent_list_spec(),
        _web_fetch_spec(),
        _web_search_spec(),
        _docs_fetch_spec(),
        _mcp_servers_spec(),
        _mcp_tools_spec(),
        _mcp_call_spec(),
        _tool_search_spec(),
        _trace_show_spec(),
        _trace_export_spec(),
        _eval_run_spec(),
    )
    return specs, {
        "task.create": handlers.task_create,
        "task.update": handlers.task_update,
        "task.list": handlers.task_list,
        "plan.create": handlers.plan_create,
        "plan.show": handlers.plan_show,
        "plan.approve_request": handlers.plan_approve_request,
        "test.run": handlers.test_run,
        "lint.run": handlers.lint_run,
        "typecheck.run": handlers.typecheck_run,
        "build.run": handlers.build_run,
        "patch.preview": handlers.patch_preview,
        "patch.apply": handlers.patch_apply,
        "patch.reverse": handlers.patch_reverse,
        "repo.map": handlers.repo_map,
        "repo.symbol_search": handlers.repo_symbol_search,
        "repo.references": handlers.repo_references,
        "repo.file_summary": handlers.repo_file_summary,
        "agent.delegate": handlers.agent_delegate,
        "agent.result": handlers.agent_result,
        "agent.list": handlers.agent_list,
        "web.fetch": handlers.web_fetch,
        "web.search": handlers.web_search,
        "docs.fetch": handlers.docs_fetch,
        "mcp.servers": handlers.mcp_servers,
        "mcp.tools": handlers.mcp_tools,
        "mcp.call": handlers.mcp_call,
        "tool.search": handlers.tool_search,
        "trace.show": handlers.trace_show,
        "trace.export": handlers.trace_export,
        "eval.run": handlers.eval_run,
    }


class RoadmapToolHandlers:
    def __init__(
        self,
        workspace: Workspace,
        broker: SubprocessBroker,
        catalog_provider: ToolCatalogProvider,
        http_transport: HttpTransport | None = None,
        task_service: TaskService | None = None,
    ) -> None:
        self._workspace = workspace
        self._broker = broker
        self._catalog_provider = catalog_provider
        self._http_transport = http_transport
        self._task_service = task_service
        self._tasks: list[JsonObject] = []
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

    async def test_run(self, arguments: JsonObject) -> str:
        paths = _workspace_paths(self._workspace, _string_list_arg(arguments, "paths") or ["tests"])
        return await self._run_named_command(("uv", "run", "pytest", *paths), arguments)

    async def lint_run(self, arguments: JsonObject) -> str:
        paths = _workspace_paths(self._workspace, _string_list_arg(arguments, "paths") or ["."])
        return await self._run_named_command(("uv", "run", "ruff", "check", *paths), arguments)

    async def typecheck_run(self, arguments: JsonObject) -> str:
        paths = _workspace_paths(
            self._workspace,
            _string_list_arg(arguments, "paths") or ["src/colossus"],
        )
        return await self._run_named_command(("uv", "run", "mypy", *paths), arguments)

    async def build_run(self, arguments: JsonObject) -> str:
        return await self._run_named_command(("uv", "run", "python", "-m", "build"), arguments)

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
        return _json({"path": self._workspace.relative(path), "replacements": replacements})

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
        return _json({"path": self._workspace.relative(path), "replacements": replacements})

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
        agent = _find_record(self._agents, _required_string_arg(arguments, "id"), "agent")
        return _json({"agent": agent})

    async def agent_list(self, arguments: JsonObject) -> str:
        status = _string_arg(arguments, "status", "")
        records = self._agents
        if status:
            records = [record for record in records if record.get("status") == status]
        return _json({"agents": records})

    async def web_fetch(self, arguments: JsonObject) -> str:
        url = _required_string_arg(arguments, "url")
        max_bytes = _int_arg(arguments, "max_bytes", 200_000)
        return await self._fetch_url(url, max_bytes)

    async def web_search(self, arguments: JsonObject) -> str:
        _ = arguments
        raise ToolExecutionError(WEB_SEARCH_DISABLED)

    async def docs_fetch(self, arguments: JsonObject) -> str:
        url = _required_string_arg(arguments, "url")
        max_bytes = _int_arg(arguments, "max_bytes", 200_000)
        return await self._fetch_url(url, max_bytes)

    async def mcp_servers(self, arguments: JsonObject) -> str:
        _ = arguments
        return _json({"servers": [], "configured": False, "message": MCP_DISABLED})

    async def mcp_tools(self, arguments: JsonObject) -> str:
        _ = arguments
        return _json({"tools": [], "configured": False, "message": MCP_DISABLED})

    async def mcp_call(self, arguments: JsonObject) -> str:
        _ = arguments
        raise ToolExecutionError(MCP_DISABLED)

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

    async def eval_run(self, arguments: JsonObject) -> str:
        paths = _workspace_paths(self._workspace, _string_list_arg(arguments, "paths") or ["tests"])
        return await self._run_named_command(("uv", "run", "pytest", *paths), arguments)

    async def _run_named_command(self, argv: tuple[str, ...], arguments: JsonObject) -> str:
        extra_args = tuple(_string_list_arg(arguments, "extra_args"))
        if extra_args:
            argv = (*argv, *extra_args)
        result = await self._broker.run(
            SubprocessCommand(
                argv=argv,
                cwd=self._workspace.root,
                timeout_seconds=float(arguments.get("timeout_seconds", 120.0)),
                max_output_bytes=_int_arg(arguments, "max_output_bytes", 64_000),
            )
        )
        return _json(
            {
                "command": list(argv),
                "exit_code": result.exit_code,
                "stdout": result.stdout,
                "stderr": result.stderr,
            }
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
                follow_redirects=True,
                timeout=20.0,
                transport=self._http_transport,
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


def _read_text(path: Path) -> str:
    try:
        data = path.read_bytes()
        if b"\x00" in data[:2048]:
            raise ToolExecutionError("Binary-looking files are not supported.")
        return data.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise ToolExecutionError("Only UTF-8 text files are supported.") from exc


def _workspace_paths(workspace: Workspace, values: list[str]) -> tuple[str, ...]:
    return tuple(workspace.relative(workspace.resolve(value)) for value in values)


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


def _string_list_arg(arguments: JsonObject, name: str) -> list[str]:
    value = arguments.get(name, [])
    if value is None:
        return []
    if not isinstance(value, list):
        raise ToolExecutionError(f"Argument {name} must be an array of strings.")
    if not all(isinstance(item, str) for item in value):
        raise ToolExecutionError(f"Argument {name} must be an array of strings.")
    return value


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


def _object_schema(properties: JsonObject, required: list[str] | None = None) -> JsonObject:
    return {
        "type": "object",
        "properties": properties,
        "required": required or [],
        "additionalProperties": False,
    }


def _array_of_strings() -> JsonObject:
    return {"type": "array", "items": {"type": "string"}}


def _common_run_properties() -> JsonObject:
    return {
        "paths": _array_of_strings(),
        "extra_args": _array_of_strings(),
        "timeout_seconds": {"type": "number", "minimum": 0.1, "maximum": 600},
        "max_output_bytes": {"type": "integer", "minimum": 1, "maximum": 200000},
    }


def _run_output_schema() -> JsonObject:
    return _object_schema(
        {
            "command": _array_of_strings(),
            "exit_code": {"type": "integer"},
            "stdout": {"type": "string"},
            "stderr": {"type": "string"},
        }
    )


def _command_permission() -> ToolPermission:
    return ToolPermission(
        filesystem="read",
        approval_required=True,
        mutation=True,
        risk="high",
    )


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


def _test_run_spec() -> ToolSpec:
    return ToolSpec(
        name="test.run",
        description="Run the configured Python test command through the subprocess broker.",
        input_schema=_object_schema(_common_run_properties()),
        output_schema=_run_output_schema(),
        permissions=_command_permission(),
        timeout_seconds=120.0,
        max_output_bytes=64_000,
    )


def _lint_run_spec() -> ToolSpec:
    return ToolSpec(
        name="lint.run",
        description="Run the configured linter through the subprocess broker.",
        input_schema=_object_schema(_common_run_properties()),
        output_schema=_run_output_schema(),
        permissions=_command_permission(),
        timeout_seconds=120.0,
        max_output_bytes=64_000,
    )


def _typecheck_run_spec() -> ToolSpec:
    return ToolSpec(
        name="typecheck.run",
        description="Run the configured type checker through the subprocess broker.",
        input_schema=_object_schema(_common_run_properties()),
        output_schema=_run_output_schema(),
        permissions=_command_permission(),
        timeout_seconds=120.0,
        max_output_bytes=64_000,
    )


def _build_run_spec() -> ToolSpec:
    return ToolSpec(
        name="build.run",
        description="Run the configured package build through the subprocess broker.",
        input_schema=_object_schema(
            {
                "extra_args": _array_of_strings(),
                "timeout_seconds": {"type": "number", "minimum": 0.1, "maximum": 600},
                "max_output_bytes": {"type": "integer", "minimum": 1, "maximum": 200000},
            }
        ),
        output_schema=_run_output_schema(),
        permissions=_command_permission(),
        timeout_seconds=120.0,
        max_output_bytes=64_000,
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
            {"path": {"type": "string"}, "replacements": {"type": "integer"}}
        ),
        permissions=_write_permission(),
    )


def _patch_reverse_spec() -> ToolSpec:
    return ToolSpec(
        name="patch.reverse",
        description="Reverse an exact text patch inside the workspace.",
        input_schema=_patch_schema(),
        output_schema=_object_schema(
            {"path": {"type": "string"}, "replacements": {"type": "integer"}}
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
        description="Record a bounded local subagent delegation request.",
        input_schema=_object_schema(
            {
                "id": {"type": "string"},
                "role": {"type": "string"},
                "task": {"type": "string", "minLength": 1},
                "mutation_allowed": {"type": "boolean", "default": False},
            },
            ["task"],
        ),
        output_schema=_object_schema({"agent": {"type": "object"}}),
        permissions=ToolPermission(working_root_required=False, risk="medium"),
    )


def _agent_result_spec() -> ToolSpec:
    return ToolSpec(
        name="agent.result",
        description="Return a recorded local subagent result.",
        input_schema=_object_schema({"id": {"type": "string", "minLength": 1}}, ["id"]),
        output_schema=_object_schema({"agent": {"type": "object"}}),
        permissions=ToolPermission(working_root_required=False, risk="low"),
    )


def _agent_list_spec() -> ToolSpec:
    return ToolSpec(
        name="agent.list",
        description="List recorded local subagent delegations.",
        input_schema=_object_schema({"status": {"type": "string"}}),
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


def _eval_run_spec() -> ToolSpec:
    return ToolSpec(
        name="eval.run",
        description="Run a configured local evaluation suite through pytest.",
        input_schema=_object_schema(_common_run_properties()),
        output_schema=_run_output_schema(),
        permissions=_command_permission(),
        timeout_seconds=120.0,
        max_output_bytes=64_000,
    )
