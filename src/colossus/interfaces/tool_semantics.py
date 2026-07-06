"""Semantic summaries for rendered tool results.

These helpers are intentionally display-only. They consume already-bounded tool output
strings and return compact transcript text without changing execution, policy, or audit
behavior.
"""

from __future__ import annotations

import json
from collections.abc import Callable
from dataclasses import dataclass
from typing import Any


@dataclass(frozen=True)
class SemanticToolResult:
    title: str
    body: str


_LOCAL_SEMANTIC_TOOLS = {
    "filesystem.read",
    "filesystem.list",
    "filesystem.search",
    "filesystem.write",
    "filesystem.replace",
    "git.status",
    "git.diff",
    "git.show",
    "patch.preview",
    "patch.apply",
    "patch.reverse",
    "shell.run",
}


def is_semantic_tool_name(name: str) -> bool:
    if name == "echo" or name in _LOCAL_SEMANTIC_TOOLS:
        return True
    return name.startswith(
        (
            "task.",
            "decision.",
            "memory.",
            "plan.",
            "goal.",
            "context.",
            "repo.",
            "agent.",
            "skill.",
            "web.",
            "docs.",
            "mcp.",
            "trace.",
            "github.",
            "searxng.",
            "opensearch.",
            "openapi.",
        )
    ) or name == "tool.search"


def semantic_tool_result(
    name: str,
    output: str,
    *,
    call_id: str = "",
    exit_code: int = 0,
) -> SemanticToolResult | None:
    if name == "echo":
        return SemanticToolResult("echo", _one_line(output, 180))

    payload = _decode_payload(output)
    if payload is None:
        return None

    if name == "filesystem.list":
        return _filesystem_list(payload)
    if name == "filesystem.search":
        return _filesystem_search(payload)
    if name.startswith("task."):
        return _task_result(name, payload) or _generic_structured_result(
            name, payload, call_id, exit_code
        )
    if name.startswith("decision."):
        return _decision_result(name, payload) or _generic_structured_result(
            name, payload, call_id, exit_code
        )
    if name.startswith("memory."):
        return _memory_result(name, payload) or _generic_structured_result(
            name, payload, call_id, exit_code
        )
    if name.startswith("plan."):
        return _plan_result(name, payload) or _generic_structured_result(
            name, payload, call_id, exit_code
        )
    if name.startswith("goal."):
        return _goal_result(payload) or _generic_structured_result(
            name, payload, call_id, exit_code
        )
    if name.startswith("context."):
        return _context_result(name, payload) or _generic_structured_result(
            name, payload, call_id, exit_code
        )
    if name.startswith("repo."):
        return _repo_result(name, payload) or _generic_structured_result(
            name, payload, call_id, exit_code
        )
    if name.startswith("agent."):
        return _agent_result(name, payload) or _generic_structured_result(
            name, payload, call_id, exit_code
        )
    if name.startswith("skill."):
        return _skill_result(name, payload) or _generic_structured_result(
            name, payload, call_id, exit_code
        )
    if name in {"web.fetch", "docs.fetch"}:
        return _fetch_result(name, payload)
    if name == "web.search" or name.startswith("searxng."):
        return _search_result(name, payload)
    if name.startswith("mcp."):
        return _mcp_result(name, payload) or _generic_structured_result(
            name, payload, call_id, exit_code
        )
    if name == "tool.search":
        return _tool_search_result(payload)
    if name.startswith("trace."):
        return _trace_result(name, payload) or _generic_structured_result(
            name, payload, call_id, exit_code
        )
    if _is_integration_tool_name(name):
        return _integration_result(name, payload, exit_code)
    return _generic_structured_result(name, payload, call_id, exit_code)


def _filesystem_list(payload: dict[str, Any]) -> SemanticToolResult:
    entries = _list(payload.get("entries"))
    body = f"{len(entries)} entries"
    details = _records(
        entries,
        lambda item: f"{_string(item.get('type')):<9} {_string(item.get('path'))}",
    )
    if details:
        body = f"{body}\n{details}"
    return SemanticToolResult("list", body)


def _filesystem_search(payload: dict[str, Any]) -> SemanticToolResult:
    matches = _list(payload.get("matches"))
    suffix = " truncated" if _bool(payload.get("truncated")) else ""
    body = f"{len(matches)} matches{suffix}"
    details = _records(
        matches,
        lambda item: (
            f"{_string(item.get('path'))}:{_int(item.get('line'))} "
            f"{_one_line(_string(item.get('text')), 140)}"
        ),
    )
    if details:
        body = f"{body}\n{details}"
    return SemanticToolResult("search", body)


def _task_result(name: str, payload: dict[str, Any]) -> SemanticToolResult | None:
    if name.endswith(".list"):
        tasks = _list(payload.get("tasks"))
        return SemanticToolResult(
            "tasks",
            _counted_records(
                tasks,
                lambda item: (
                    f"{_fallback(_string(item.get('status')), 'pending')} "
                    f"{_short_id(_string(item.get('id')))} "
                    f"{_one_line(_string(item.get('title')), 120)}"
                ),
            ),
        )
    task = _dict(payload.get("task"))
    if task is None:
        return None
    body = (
        f"{_fallback(_string(task.get('status')), 'pending')} "
        f"{_short_id(_string(task.get('id')))} "
        f"{_one_line(_string(task.get('title')), 140)}"
    )
    if description := _string(task.get("description")):
        body = f"{body}\n{_one_line(description, 180)}"
    return SemanticToolResult("task", body)


def _decision_result(name: str, payload: dict[str, Any]) -> SemanticToolResult | None:
    if name.endswith(".list"):
        decisions = _list(payload.get("decisions"))
        return SemanticToolResult(
            "decisions",
            _counted_records(
                decisions,
                lambda item: (
                    f"{_fallback(_string(item.get('status')), 'active')} "
                    f"{_fallback(_string(item.get('priority')), 'normal')} "
                    f"{_short_id(_string(item.get('id')))} "
                    f"{_one_line(_first(item, 'title', 'decision'), 120)}"
                ),
            ),
        )
    decision = _dict(payload.get("decision"))
    if decision is None:
        return None
    body = (
        f"{_fallback(_string(decision.get('status')), 'active')} "
        f"{_short_id(_string(decision.get('id')))} "
        f"{_one_line(_first(decision, 'title', 'decision'), 140)}"
    )
    if text := _string(decision.get("decision")):
        body = f"{body}\n{_one_line(text, 180)}"
    return SemanticToolResult("decision", body)


def _memory_result(name: str, payload: dict[str, Any]) -> SemanticToolResult | None:
    if name.endswith(".list") or name.endswith(".search"):
        memories = _list(payload.get("memories"))
        title = "memory search" if name.endswith(".search") else "memories"
        return SemanticToolResult(
            title,
            _counted_records(
                memories,
                lambda item: (
                    f"{_fallback(_string(item.get('scope')), 'scope')}/"
                    f"{_fallback(_string(item.get('kind')), 'memory')} "
                    f"{_short_id(_string(item.get('id')))} "
                    f"{_one_line(_string(item.get('text')), 120)}"
                ),
            ),
        )
    memory = _dict(payload.get("memory"))
    if memory is None:
        return None
    body = (
        f"{_fallback(_string(memory.get('scope')), 'scope')}/"
        f"{_fallback(_string(memory.get('kind')), 'memory')} "
        f"{_short_id(_string(memory.get('id')))} "
        f"{_one_line(_string(memory.get('text')), 160)}"
    )
    if notice := _string(payload.get("notice")):
        body = f"{body}\n{_one_line(notice, 180)}"
    return SemanticToolResult("memory", body)


def _plan_result(name: str, payload: dict[str, Any]) -> SemanticToolResult | None:
    plan = _dict(payload.get("plan"))
    if plan is None:
        return None
    title = (
        "plan approved"
        if _bool(payload.get("approved")) or name.endswith(".approve_request")
        else "plan"
    )
    steps = _list(plan.get("steps"))
    body = (
        f"{_fallback(_string(plan.get('status')), 'draft')} "
        f"{_short_id(_string(plan.get('id')))} steps={len(steps)}"
    )
    if prompt := _string(plan.get("prompt")):
        body = f"{body}\n{_one_line(prompt, 180)}"
    details = _records(
        steps,
        lambda item: (
            f"{_int(item.get('index'))} "
            f"{'mutates' if _bool(item.get('requires_mutation')) else 'read'} "
            f"{_one_line(_string(item.get('title')), 120)}"
        ),
    )
    if details:
        body = f"{body}\n{details}"
    return SemanticToolResult(title, body)


def _goal_result(payload: dict[str, Any]) -> SemanticToolResult | None:
    goal = _dict(payload.get("goal"))
    if goal is None:
        return None
    body = (
        f"{_fallback(_string(goal.get('status')), 'active')} "
        f"{_short_id(_string(goal.get('id')))} "
        f"iterations={_int(goal.get('iterations_completed'))}/{_goal_budget(goal)}"
    )
    if objective := _string(goal.get("objective")):
        body = f"{body}\n{_one_line(objective, 180)}"
    if summary := _string(goal.get("summary")):
        body = f"{body}\nsummary {_one_line(summary, 180)}"
    if blocked := _string(goal.get("blocked_reason")):
        body = f"{body}\nblocked {_one_line(blocked, 180)}"
    return SemanticToolResult("goal", body)


def _context_result(name: str, payload: dict[str, Any]) -> SemanticToolResult | None:
    if name == "context.show":
        status = _dict(payload.get("status"))
        if status is None:
            return None
        body = (
            f"session={_short_id(_string(status.get('session_id')))} "
            f"tokens={_int(status.get('token_estimate'))}/"
            f"{_int(status.get('context_window_tokens'))} "
            f"messages={_int(status.get('message_count'))} "
            f"compacted={str(_bool(status.get('compacted'))).lower()} "
            f"auto={str(_bool(status.get('auto_compaction'))).lower()}"
        )
        if snapshot_id := _string(status.get("latest_snapshot_id")):
            body = f"{body}\nlatest_snapshot={_short_id(snapshot_id)}"
        return SemanticToolResult("context", body)
    if name == "context.snapshots":
        snapshots = _list(payload.get("snapshots"))
        return SemanticToolResult("snapshots", _counted_records(snapshots, _snapshot_line))
    if name in {"context.compact", "context.restore"}:
        snapshot = _dict(payload.get("snapshot"))
        if snapshot is None:
            return None
        title = "snapshot restored" if _bool(payload.get("restored")) else "snapshot"
        body = _snapshot_line(snapshot)
        if summary := _string(snapshot.get("summary")):
            body = f"{body}\n{_one_line(summary, 180)}"
        return SemanticToolResult(title, body)
    return None


def _repo_result(name: str, payload: dict[str, Any]) -> SemanticToolResult | None:
    if name == "repo.map":
        files = _list(payload.get("files"))
        suffix = " truncated" if _bool(payload.get("truncated")) else ""
        body = f"{_fallback(_string(payload.get('root')), '.')} files={len(files)}{suffix}"
        if counts := _dict(payload.get("extension_counts")):
            body = f"{body}\nextensions {_count_map(counts)}"
        details = _records(
            files,
            lambda item: (
                f"{_string(item.get('path'))} {_int(item.get('size'))} bytes"
            ),
        )
        if details:
            body = f"{body}\n{details}"
        return SemanticToolResult("repo map", body)
    if name == "repo.symbol_search":
        symbols = _list(payload.get("symbols"))
        suffix = " truncated" if _bool(payload.get("truncated")) else ""
        return SemanticToolResult("symbols", _counted_records(symbols, _symbol_line, suffix=suffix))
    if name == "repo.references":
        references = _list(payload.get("references"))
        suffix = " truncated" if _bool(payload.get("truncated")) else ""
        return SemanticToolResult(
            "references",
            _counted_records(
                references,
                lambda item: (
                    f"{_string(item.get('path'))}:{_int(item.get('line'))} "
                    f"{_one_line(_string(item.get('text')), 140)}"
                ),
                suffix=suffix,
            ),
        )
    if name == "repo.file_summary":
        body = (
            f"{_string(payload.get('path'))} "
            f"lines={_int(payload.get('lines'))} bytes={_int(payload.get('bytes'))} "
            f"imports={len(_list(payload.get('imports')))} "
            f"headings={len(_list(payload.get('headings')))} "
            f"symbols={len(_list(payload.get('symbols')))}"
        )
        details = _records(_list(payload.get("symbols")), _symbol_line)
        if details:
            body = f"{body}\n{details}"
        return SemanticToolResult("file summary", body)
    return None


def _agent_result(name: str, payload: dict[str, Any]) -> SemanticToolResult | None:
    if name.endswith(".list"):
        agents = _list(payload.get("agents"))
        return SemanticToolResult("subagents", _counted_records(agents, _subagent_line))
    agent = _dict(payload.get("agent"))
    if agent is None:
        return None
    body = _subagent_line(agent)
    if final_output := _string(agent.get("final_output")):
        body = f"{body}\noutput {_one_line(final_output, 180)}"
    if error := _string(agent.get("error")):
        body = f"{body}\nerror {_one_line(error, 180)}"
    return SemanticToolResult("subagent", body)


def _skill_result(name: str, payload: dict[str, Any]) -> SemanticToolResult | None:
    if name == "skill.resource.list":
        resources = _list(payload.get("resources"))
        return SemanticToolResult(
            "resource",
            _counted_records(
                resources,
                lambda item: (
                    f"{_string(item.get('kind'))} {_string(item.get('path'))} "
                    f"{_int(item.get('size'))} bytes"
                ),
            ),
        )
    if name == "skill.resource.read":
        resource = _dict(payload.get("resource"))
        if resource is None:
            return None
        body = f"read {_string(resource.get('path'))} {_int(resource.get('size'))} bytes"
        if content := _string(resource.get("content")):
            body = f"{body}\n{_source_preview_text(content, 8)}"
        return SemanticToolResult("resource", body)
    if name == "skill.validate":
        validation = _dict(payload.get("validation"))
        if validation is None:
            return None
        return SemanticToolResult("skill", _validation_text(validation))
    if name == "skill.read":
        file = _dict(payload.get("file"))
        if file is None:
            return None
        body = (
            f"read {_string(file.get('name'))}/{_string(file.get('path'))} "
            f"{_int(file.get('size'))} bytes"
        )
        if content := _string(file.get("content")):
            body = f"{body}\n{_source_preview_text(content, 8)}"
        return SemanticToolResult("skill", body)
    if name == "skill.write":
        file = _dict(payload.get("file"))
        if file is None:
            return None
        body = (
            f"write {_string(file.get('name'))}/{_string(file.get('path'))} "
            f"{_int(file.get('size'))} bytes mode={_string(file.get('mode'))}"
        )
        if validation := _dict(file.get("validation")):
            body = f"{body}\nvalidation {_validation_text(validation)}"
        return SemanticToolResult("skill", body)
    skill = _dict(payload.get("skill"))
    if skill is None:
        return None
    action = name.removeprefix("skill.")
    display_name = _first(skill, "name", "path", "target_path")
    body = f"{action} {_one_line(display_name, 160)}"
    path = _first(skill, "path", "target_path", "source_path")
    if path and path != display_name:
        body = f"{body}\npath {_one_line(path, 180)}"
    files = _list(skill.get("files"))
    if files:
        body = f"{body}\nfiles={len(files)}"
    if validation := _dict(skill.get("validation")):
        body = f"{body}\nvalidation {_validation_text(validation)}"
    return SemanticToolResult("skill", body)


def _fetch_result(name: str, payload: dict[str, Any]) -> SemanticToolResult:
    content = _string(payload.get("content"))
    title = "docs fetch" if name == "docs.fetch" else "fetch"
    body = (
        f"status={_int(payload.get('status_code'))} bytes={len(content.encode('utf-8'))} "
        f"type={_fallback(_string(payload.get('content_type')), 'unknown')} "
        f"url={_one_line(_string(payload.get('url')), 140)}"
    )
    if _bool(payload.get("truncated")):
        body = f"{body} truncated"
    if content:
        body = f"{body}\n{_source_preview_text(content, 8)}"
    return SemanticToolResult(title, body)


def _search_result(name: str, payload: dict[str, Any]) -> SemanticToolResult:
    if name.endswith(".health"):
        return SemanticToolResult(
            "searxng",
            f"health status={_fallback(_string(payload.get('status')), 'unknown')} "
            f"results={_int(payload.get('result_count'))}",
        )
    results = _list(payload.get("results"))
    title = "searxng" if name.startswith("searxng.") else "web search"
    provider = _first(payload, "search_provider", "provider") or (
        "searxng" if title == "searxng" else "search"
    )
    body = (
        f"provider={provider} results={len(results)} "
        f"query={_one_line(_string(payload.get('query')), 120)}"
    )
    details = _records(
        results,
        lambda item: (
            f"{_one_line(_first(item, 'title', 'name', 'content'), 100)} "
            f"{_one_line(_first(item, 'url', 'uri'), 120)}"
        ),
    )
    if details:
        body = f"{body}\n{details}"
    return SemanticToolResult(title, body)


def _mcp_result(name: str, payload: dict[str, Any]) -> SemanticToolResult | None:
    if name == "mcp.servers":
        servers = _list(payload.get("servers"))
        body = f"servers={len(servers)} configured={str(_bool(payload.get('configured'))).lower()}"
        if message := _string(payload.get("message")):
            body = f"{body}\n{_one_line(message, 180)}"
        details = _records(
            servers,
            lambda item: (
                f"{_string(item.get('name'))} "
                f"tools={len(_list(item.get('allowed_tools')))} "
                f"env_keys={len(_list(item.get('env_keys')))}"
            ),
        )
        if details:
            body = f"{body}\n{details}"
        return SemanticToolResult("mcp", body)
    if name == "mcp.tools":
        tools = _list(payload.get("tools"))
        body = f"tools={len(tools)} configured={str(_bool(payload.get('configured'))).lower()}"
        if message := _string(payload.get("message")):
            body = f"{body}\n{_one_line(message, 180)}"
        details = _records(
            tools,
            lambda item: (
                f"{_string(item.get('server'))}/{_string(item.get('name'))} "
                f"{_string(item.get('source'))}"
            ),
        )
        if details:
            body = f"{body}\n{details}"
        return SemanticToolResult("mcp", body)
    return None


def _tool_search_result(payload: dict[str, Any]) -> SemanticToolResult:
    tools = _list(payload.get("tools"))
    return SemanticToolResult(
        "catalog",
        _counted_records(
            tools,
            lambda item: (
                f"{_string(item.get('name'))} "
                f"risk={_fallback(_string(item.get('risk')), 'low')} "
                f"approval={str(_bool(item.get('approval_required'))).lower()} "
                f"{_one_line(_string(item.get('description')), 100)}"
            ),
        ),
    )


def _trace_result(name: str, payload: dict[str, Any]) -> SemanticToolResult | None:
    if name == "trace.show":
        events = _list(payload.get("events"))
        body = f"events={len(events)} available={str(_bool(payload.get('available'))).lower()}"
        details = _records(
            events,
            lambda item: _one_line(_first(item, "event", "type", "raw"), 160),
        )
        if details:
            body = f"{body}\n{details}"
        return SemanticToolResult("trace", body)
    if name == "trace.export":
        return SemanticToolResult(
            "trace",
            f"exported {_string(payload.get('path'))} bytes={_int(payload.get('bytes_written'))}",
        )
    return None


def _integration_result(name: str, payload: dict[str, Any], exit_code: int) -> SemanticToolResult:
    result = payload.get("result")
    body = (
        f"{name} status={_int(payload.get('status_code'))} exit={exit_code} "
        f"items={_result_item_count(result)}"
    )
    if _bool(payload.get("truncated")):
        body = f"{body} truncated"
    preview = _integration_preview(result)
    if preview:
        body = f"{body}\n{preview}"
    return SemanticToolResult(_integration_label(name), body)


def _generic_structured_result(
    name: str,
    payload: dict[str, Any],
    call_id: str,
    exit_code: int,
) -> SemanticToolResult:
    keys = sorted(payload)
    body = (
        f"{name} exit={exit_code} keys={','.join(keys)} "
        f"call_id={_short_id(call_id)}"
    )
    if summary := _generic_summary(payload):
        body = f"{body}\n{summary}"
    return SemanticToolResult("tool result", body)


def _decode_payload(output: str) -> dict[str, Any] | None:
    try:
        payload = json.loads(output)
    except json.JSONDecodeError:
        return None
    return payload if isinstance(payload, dict) else None


def _list(value: object) -> list[Any]:
    return value if isinstance(value, list) else []


def _dict(value: object) -> dict[str, Any] | None:
    return value if isinstance(value, dict) else None


def _string(value: object) -> str:
    if isinstance(value, str):
        return value
    if value is None:
        return ""
    return str(value)


def _int(value: object) -> int:
    if isinstance(value, bool):
        return 0
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        return int(value)
    return 0


def _bool(value: object) -> bool:
    return value if isinstance(value, bool) else False


def _fallback(value: str, fallback: str) -> str:
    return value.strip() or fallback


def _first(source: dict[str, Any], *keys: str) -> str:
    for key in keys:
        value = _string(source.get(key)).strip()
        if value:
            return value
    return ""


def _one_line(value: str, limit: int) -> str:
    return _truncate(" ".join(value.splitlines()), limit)


def _truncate(value: str, limit: int) -> str:
    if len(value) <= limit:
        return value
    if limit <= 3:
        return value[:limit]
    return f"{value[: limit - 3]}..."


def _short_id(value: str) -> str:
    return value[:8]


def _counted_records(
    records: list[Any],
    render: Callable[[dict[str, Any]], str],
    *,
    suffix: str = "",
) -> str:
    body = f"{len(records)}{suffix}"
    details = _records(records, render)
    return f"{body}\n{details}" if details else body


def _records(records: list[Any], render: Callable[[dict[str, Any]], str], limit: int = 14) -> str:
    lines: list[str] = []
    for index, raw in enumerate(records):
        if index >= limit:
            lines.append(f"... {len(records) - index} more")
            break
        item = _dict(raw)
        if item is None:
            continue
        line = render(item).strip()
        if line:
            lines.append(line)
    return "\n".join(lines)


def _goal_budget(goal: dict[str, Any]) -> str:
    budget = _int(goal.get("iteration_budget"))
    return str(budget) if budget > 0 else "unbounded"


def _snapshot_line(item: dict[str, Any]) -> str:
    parts = [_short_id(_string(item.get("id")))]
    if strategy := _string(item.get("strategy")):
        parts.append(f"strategy={strategy}")
    if source_range := _int_range(item.get("source_message_range")):
        parts.append(f"messages={source_range}")
    parts.append(f"facts={len(_list(item.get('pinned_facts')))}")
    parts.append(f"tasks={len(_list(item.get('open_tasks')))}")
    return " ".join(parts)


def _int_range(value: object) -> str:
    values = _list(value)
    if len(values) < 2:
        return ""
    start = _int(values[0])
    end = _int(values[1])
    if start == 0 and end == 0:
        return ""
    return f"{start}-{end}"


def _symbol_line(item: dict[str, Any]) -> str:
    return (
        f"{_string(item.get('path'))}:{_int(item.get('line'))} "
        f"{_string(item.get('kind'))} {_string(item.get('name'))}"
    )


def _subagent_line(item: dict[str, Any]) -> str:
    return (
        f"{_fallback(_string(item.get('status')), 'queued')} "
        f"{_short_id(_string(item.get('id')))} "
        f"role={_fallback(_string(item.get('role')), 'subagent_default')} "
        f"{_one_line(_string(item.get('task')), 120)}"
    )


def _validation_text(validation: dict[str, Any]) -> str:
    errors = _list(validation.get("errors"))
    body = (
        f"valid={str(_bool(validation.get('valid'))).lower()} "
        f"errors={len(errors)} path={_one_line(_string(validation.get('path')), 120)}"
    )
    lines = [_one_line(error, 160) for error in errors[:14] if isinstance(error, str)]
    if len(errors) > 14:
        lines.append(f"... {len(errors) - 14} more")
    if lines:
        body = f"{body}\n" + "\n".join(lines)
    return body


def _source_preview_text(content: str, limit: int) -> str:
    lines = content.rstrip("\n").splitlines()
    if not lines:
        return ""
    selected = _preview_indexes(len(lines), limit)
    rendered: list[str] = []
    for index in selected:
        if index is None:
            rendered.append("  ...")
        else:
            rendered.append(f"{index + 1:>4}  {lines[index]}")
    return "\n".join(rendered)


def _preview_indexes(count: int, limit: int) -> list[int | None]:
    if count <= limit:
        return list(range(count))
    head = max(1, limit // 2)
    tail = max(1, limit - head - 1)
    return [*range(head), None, *range(count - tail, count)]


def _count_map(values: dict[str, Any], limit: int = 8) -> str:
    keys = sorted(values)
    parts = [f"{key}={_int(values[key])}" for key in keys[:limit]]
    if len(keys) > limit:
        parts.append(f"...{len(keys) - limit} more")
    return " ".join(parts) if parts else "none"


def _is_integration_tool_name(name: str) -> bool:
    return name.startswith(("github.", "opensearch.", "openapi."))


def _integration_label(name: str) -> str:
    if name.startswith("github."):
        return "github"
    if name.startswith("opensearch."):
        return "opensearch"
    if name.startswith("openapi."):
        return "openapi"
    return "integration"


def _result_item_count(value: object) -> int:
    if isinstance(value, list | dict):
        return len(value)
    if isinstance(value, str):
        return 1 if value.strip() else 0
    return 0 if value is None else 1


def _integration_preview(value: object) -> str:
    if isinstance(value, list):
        return _records(
            value,
            lambda item: (
                f"{_one_line(_first(item, 'full_name', 'title', 'name', 'id', 'status'), 110)} "
                f"{_one_line(_first(item, 'html_url', 'url', 'uri'), 120)}"
            ),
        )
    if isinstance(value, dict):
        lines = [f"result keys={','.join(sorted(value))}"]
        hits = _dict(value.get("hits"))
        hit_items = _list(hits.get("hits")) if hits is not None else []
        if hit_items:
            hit_preview = _records(
                hit_items,
                lambda item: _one_line(_first(item, "_id", "id", "title", "name"), 140),
            )
            if hit_preview:
                lines.append(hit_preview)
        return "\n".join(lines)
    if isinstance(value, str) and value.strip():
        return _one_line(value, 180)
    return ""


def _generic_summary(payload: dict[str, Any]) -> str:
    parts: list[str] = []
    for key in sorted(payload):
        value = payload[key]
        if isinstance(value, list):
            parts.append(f"{key}={len(value)}")
        elif isinstance(value, dict):
            parts.append(f"{key}.keys={','.join(sorted(value))}")
        elif isinstance(value, str) and value.strip():
            parts.append(f"{key}={_one_line(value, 80)}")
        elif isinstance(value, bool):
            parts.append(f"{key}={str(value).lower()}")
        elif value is not None:
            parts.append(f"{key}={_one_line(str(value), 80)}")
        if len(parts) >= 6:
            break
    return " ".join(parts)
