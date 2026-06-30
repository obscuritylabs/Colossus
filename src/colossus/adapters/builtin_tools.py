"""Built-in filesystem, git, shell, and smoke-test tools."""

import json
import re
import shutil
from pathlib import Path
from typing import Any

from colossus.adapters.context_tools import create_context_tools
from colossus.adapters.roadmap_tools import create_roadmap_tools
from colossus.adapters.skill_tools import create_skill_tools
from colossus.adapters.subprocess_broker import (
    SubprocessBroker,
    SubprocessCommand,
    SubprocessResult,
)
from colossus.adapters.workspace import Workspace
from colossus.application.context import ContextService
from colossus.application.decisions import DecisionService
from colossus.application.memories import MemoryService
from colossus.application.skill_authoring import SkillAuthoringService
from colossus.application.subagents import SubagentService
from colossus.application.tasks import TaskService
from colossus.application.tools import ToolHandler
from colossus.domain.errors import ToolExecutionError
from colossus.domain.tools import ToolPermission, ToolSpec
from colossus.domain.user_prompts import UserPromptChoice
from colossus.infrastructure.http_client import HttpClientConfig
from colossus.ports.model_provider import ModelProvider
from colossus.ports.research import McpGateway, SearchProvider
from colossus.ports.user_prompt import UserPromptHandler

JsonObject = dict[str, Any]
HandlerMap = dict[str, ToolHandler]

SHELL_WRAPPERS = frozenset({"sh", "bash", "zsh", "fish", "cmd", "cmd.exe", "powershell", "pwsh"})


def create_builtin_tools(
    workspace: Workspace,
    *,
    context_service: ContextService | None = None,
    context_provider: ModelProvider | None = None,
    context_model: str = "default",
    task_service: TaskService | None = None,
    decision_service: DecisionService | None = None,
    memory_service: MemoryService | None = None,
    subagent_service: SubagentService | None = None,
    include_agent_delegate: bool = True,
    user_prompt_handler: UserPromptHandler | None = None,
    search_provider: SearchProvider | None = None,
    mcp_gateway: McpGateway | None = None,
    http_client_config: HttpClientConfig | None = None,
    skill_authoring_service: SkillAuthoringService | None = None,
) -> tuple[tuple[ToolSpec, ...], HandlerMap]:
    broker = SubprocessBroker()
    handlers = BuiltinToolHandlers(workspace, broker, user_prompt_handler=user_prompt_handler)
    core_specs = (
        _filesystem_list_spec(),
        _filesystem_read_spec(),
        _filesystem_search_spec(),
        _filesystem_write_spec(),
        _filesystem_replace_spec(),
        _git_status_spec(),
        _git_diff_spec(),
        _git_show_spec(),
        _shell_run_spec(),
    )
    catalog: list[ToolSpec] = []
    roadmap_specs, roadmap_handlers = create_roadmap_tools(
        workspace,
        broker,
        lambda: tuple(catalog),
        task_service=task_service,
        decision_service=decision_service,
        memory_service=memory_service,
        subagent_service=subagent_service,
        include_agent_delegate=include_agent_delegate,
        search_provider=search_provider,
        mcp_gateway=mcp_gateway,
        http_client_config=http_client_config,
    )
    context_specs, context_handlers = create_context_tools(
        context_service,
        provider=context_provider,
        default_model=context_model,
    )
    skill_specs, skill_handlers = (
        create_skill_tools(skill_authoring_service)
        if skill_authoring_service is not None
        else ((), {})
    )
    user_prompt_specs = (_user_ask_spec(),) if user_prompt_handler is not None else ()
    specs = (
        *core_specs,
        *roadmap_specs,
        *context_specs,
        *skill_specs,
        *user_prompt_specs,
        _echo_spec(),
    )
    user_prompt_handlers = (
        {"user.ask": handlers.user_ask} if user_prompt_handler is not None else {}
    )
    catalog.extend(specs)
    return specs, {
        "filesystem.list": handlers.filesystem_list,
        "filesystem.read": handlers.filesystem_read,
        "filesystem.search": handlers.filesystem_search,
        "filesystem.write": handlers.filesystem_write,
        "filesystem.replace": handlers.filesystem_replace,
        "git.status": handlers.git_status,
        "git.diff": handlers.git_diff,
        "git.show": handlers.git_show,
        "shell.run": handlers.shell_run,
        **roadmap_handlers,
        **context_handlers,
        **skill_handlers,
        **user_prompt_handlers,
        "echo": handlers.echo,
    }


class BuiltinToolHandlers:
    def __init__(
        self,
        workspace: Workspace,
        broker: SubprocessBroker,
        *,
        user_prompt_handler: UserPromptHandler | None = None,
    ) -> None:
        self._workspace = workspace
        self._broker = broker
        self._user_prompt_handler = user_prompt_handler

    async def echo(self, arguments: JsonObject) -> str:
        return str(arguments.get("text", ""))

    async def user_ask(self, arguments: JsonObject) -> str:
        if self._user_prompt_handler is None:
            raise ToolExecutionError("user.ask is unavailable in this surface.")
        question = _required_string_arg(arguments, "question")
        choices = _prompt_choices_arg(arguments)
        allow_freeform = _bool_arg(arguments, "allow_freeform", True)
        if not choices and not allow_freeform:
            raise ToolExecutionError(
                "user.ask requires choices when free-form answers are disabled."
            )
        answer = await self._user_prompt_handler.ask(
            question=question,
            choices=choices,
            allow_freeform=allow_freeform,
        )
        return _json(answer.model_dump())

    async def filesystem_list(self, arguments: JsonObject) -> str:
        root = self._workspace.resolve(_string_arg(arguments, "path", "."))
        recursive = _bool_arg(arguments, "recursive", False)
        pattern = _string_arg(arguments, "glob", "*")
        max_entries = _int_arg(arguments, "max_entries", 200)
        candidates = root.rglob(pattern) if recursive else root.glob(pattern)
        entries: list[JsonObject] = []
        for candidate in sorted(candidates):
            try:
                relative = self._workspace.relative(candidate)
            except ToolExecutionError:
                continue
            entries.append(
                {
                    "path": relative,
                    "type": "directory" if candidate.is_dir() else "file",
                    "size": candidate.stat().st_size if candidate.is_file() else None,
                }
            )
            if len(entries) >= max_entries:
                break
        return _json({"root": self._workspace.relative(root), "entries": entries})

    async def filesystem_read(self, arguments: JsonObject) -> str:
        path = self._workspace.resolve(_required_string_arg(arguments, "path"))
        relative_path = self._workspace.relative(path)
        max_bytes = _int_arg(arguments, "max_bytes", 32_768)
        start_line = max(1, _int_arg(arguments, "start_line", 1))
        max_lines = arguments.get("max_lines")
        try:
            file_size = path.stat().st_size
            with path.open("rb") as handle:
                data = handle.read(max_bytes)
        except FileNotFoundError as exc:
            raise ToolExecutionError(f"filesystem.read file not found: {relative_path}") from exc
        except IsADirectoryError as exc:
            raise ToolExecutionError(
                f"filesystem.read path is not a file: {relative_path}"
            ) from exc
        except OSError as exc:
            reason = exc.strerror or exc.__class__.__name__
            raise ToolExecutionError(
                f"filesystem.read failed for {relative_path}: {reason}"
            ) from exc
        if b"\x00" in data:
            raise ToolExecutionError("Binary-looking files are not supported by filesystem.read.")
        try:
            text = data.decode("utf-8")
        except UnicodeDecodeError as exc:
            raise ToolExecutionError(
                "Only UTF-8 text files are supported by filesystem.read."
            ) from exc
        lines = text.splitlines()
        selected = lines[start_line - 1 :]
        if isinstance(max_lines, int):
            selected = selected[:max_lines]
        return _json(
            {
                "path": relative_path,
                "start_line": start_line,
                "line_count": len(selected),
                "content": "\n".join(selected),
                "truncated": file_size > max_bytes,
            }
        )

    async def filesystem_search(self, arguments: JsonObject) -> str:
        rg_path = shutil.which("rg")
        if rg_path is not None:
            return await self._filesystem_search_rg(rg_path, arguments)
        return await self._filesystem_search_python(arguments)

    async def filesystem_write(self, arguments: JsonObject) -> str:
        path = self._workspace.resolve(_required_string_arg(arguments, "path"))
        content = _required_string_arg(arguments, "content")
        mode = _string_arg(arguments, "mode", "overwrite")
        if mode not in {"create", "overwrite", "append"}:
            raise ToolExecutionError("filesystem.write mode must be create, overwrite, or append.")
        path.parent.mkdir(parents=True, exist_ok=True)
        if mode == "create" and path.exists():
            raise ToolExecutionError(
                "filesystem.write create mode refuses to overwrite existing files."
            )
        if mode == "append":
            with path.open("a", encoding="utf-8") as handle:
                handle.write(content)
        else:
            path.write_text(content, encoding="utf-8")
        return _json(
            {"path": self._workspace.relative(path), "bytes_written": len(content.encode())}
        )

    async def filesystem_replace(self, arguments: JsonObject) -> str:
        path = self._workspace.resolve(_required_string_arg(arguments, "path"))
        old = _required_string_arg(arguments, "old")
        new = _required_string_arg(arguments, "new")
        replace_all = _bool_arg(arguments, "replace_all", False)
        text = path.read_text(encoding="utf-8")
        occurrences = text.count(old)
        if occurrences == 0:
            raise ToolExecutionError("filesystem.replace old text was not found.")
        if occurrences > 1 and not replace_all:
            raise ToolExecutionError("filesystem.replace old text is ambiguous.")
        updated = text.replace(old, new) if replace_all else text.replace(old, new, 1)
        path.write_text(updated, encoding="utf-8")
        return _json(
            {
                "path": self._workspace.relative(path),
                "replacements": occurrences if replace_all else 1,
            }
        )

    async def git_status(self, arguments: JsonObject) -> str:
        _ = arguments
        result = await self._git(("status", "--porcelain=v1"))
        entries = []
        for line in result.stdout.splitlines():
            if not line:
                continue
            entries.append({"status": line[:2], "path": line[3:]})
        return _json({"entries": entries, "raw": result.stdout})

    async def git_diff(self, arguments: JsonObject) -> str:
        paths = _string_list_arg(arguments, "paths")
        argv = ("diff", "--", *paths) if paths else ("diff",)
        result = await self._git(argv)
        return _json(
            {"diff": result.stdout, "stderr": result.stderr, "exit_code": result.exit_code}
        )

    async def git_show(self, arguments: JsonObject) -> str:
        rev = _string_arg(arguments, "rev", "HEAD")
        path = arguments.get("path")
        argv: tuple[str, ...] = ("show", "--no-ext-diff", "--stat", "--patch", rev)
        if isinstance(path, str) and path:
            safe_path = self._workspace.relative(self._workspace.resolve(path))
            argv = (*argv, "--", safe_path)
        result = await self._git(argv)
        return _json(
            {"output": result.stdout, "stderr": result.stderr, "exit_code": result.exit_code}
        )

    async def shell_run(self, arguments: JsonObject) -> str:
        argv = _string_list_arg(arguments, "argv")
        if not argv:
            raise ToolExecutionError("shell.run requires a non-empty argv array.")
        executable = Path(argv[0]).name.lower()
        if executable in SHELL_WRAPPERS:
            raise ToolExecutionError(f"shell.run denies shell wrapper execution: {argv[0]}")
        cwd = self._workspace.resolve(_string_arg(arguments, "cwd", "."))
        env_arg = arguments.get("env")
        env = env_arg if isinstance(env_arg, dict) else {}
        env = {str(key): str(value) for key, value in env.items()}
        timeout = float(arguments.get("timeout_seconds", 30.0))
        max_output = _int_arg(arguments, "max_output_bytes", 32_768)
        result = await self._broker.run(
            SubprocessCommand(
                argv=tuple(argv),
                cwd=cwd,
                env=env,
                timeout_seconds=timeout,
                max_output_bytes=max_output,
            )
        )
        return _json(
            {
                "exit_code": result.exit_code,
                "stdout": result.stdout,
                "stderr": result.stderr,
                "cwd": self._workspace.relative(cwd),
            }
        )

    async def _filesystem_search_rg(self, rg_path: str, arguments: JsonObject) -> str:
        root = self._workspace.resolve(_string_arg(arguments, "path", "."))
        pattern = _required_string_arg(arguments, "pattern")
        max_matches = _int_arg(arguments, "max_matches", 100)
        argv = [rg_path, "--line-number", "--column", "--no-heading"]
        if not _bool_arg(arguments, "regex", True):
            argv.append("--fixed-strings")
        if not _bool_arg(arguments, "case_sensitive", True):
            argv.append("--ignore-case")
        glob_value = arguments.get("glob")
        if isinstance(glob_value, str) and glob_value:
            argv.extend(["--glob", glob_value])
        argv.extend(["--", pattern, str(root)])
        result = await self._broker.run(
            SubprocessCommand(
                argv=tuple(argv),
                cwd=self._workspace.root,
                timeout_seconds=15.0,
                max_output_bytes=64_000,
            )
        )
        matches = _parse_rg_output(result.stdout, self._workspace, max_matches)
        return _json({"matches": matches, "truncated": len(matches) >= max_matches})

    async def _filesystem_search_python(self, arguments: JsonObject) -> str:
        root = self._workspace.resolve(_string_arg(arguments, "path", "."))
        pattern = _required_string_arg(arguments, "pattern")
        glob_value = _string_arg(arguments, "glob", "**/*")
        regex = _bool_arg(arguments, "regex", True)
        case_sensitive = _bool_arg(arguments, "case_sensitive", True)
        max_matches = _int_arg(arguments, "max_matches", 100)
        compiled = re.compile(pattern, 0 if case_sensitive else re.IGNORECASE) if regex else None
        matches: list[JsonObject] = []
        for candidate in root.glob(glob_value):
            if not candidate.is_file():
                continue
            try:
                relative = self._workspace.relative(candidate)
                lines = candidate.read_text(encoding="utf-8").splitlines()
            except (ToolExecutionError, UnicodeDecodeError):
                continue
            for index, line in enumerate(lines, start=1):
                found = (
                    bool(compiled.search(line))
                    if compiled
                    else _contains(line, pattern, case_sensitive)
                )
                if found:
                    matches.append({"path": relative, "line": index, "column": 1, "text": line})
                    if len(matches) >= max_matches:
                        return _json({"matches": matches, "truncated": True})
        return _json({"matches": matches, "truncated": False})

    async def _git(self, argv: tuple[str, ...]) -> SubprocessResult:
        return await self._broker.run(
            SubprocessCommand(
                argv=("git", *argv),
                cwd=self._workspace.root,
                timeout_seconds=30.0,
                max_output_bytes=64_000,
            )
        )


def _parse_rg_output(output: str, workspace: Workspace, max_matches: int) -> list[JsonObject]:
    matches: list[JsonObject] = []
    for line in output.splitlines():
        path, line_no, column, text = _split_rg_line(line)
        if path is None:
            continue
        try:
            relative = workspace.relative(workspace.resolve(path))
        except ToolExecutionError:
            continue
        matches.append(
            {
                "path": relative,
                "line": int(line_no),
                "column": int(column),
                "text": text,
            }
        )
        if len(matches) >= max_matches:
            break
    return matches


def _split_rg_line(line: str) -> tuple[str | None, str, str, str]:
    parts = line.split(":", 3)
    if len(parts) != 4 or not parts[1].isdigit() or not parts[2].isdigit():
        return None, "0", "0", ""
    return parts[0], parts[1], parts[2], parts[3]


def _contains(line: str, pattern: str, case_sensitive: bool) -> bool:
    if case_sensitive:
        return pattern in line
    return pattern.lower() in line.lower()


def _required_string_arg(arguments: JsonObject, name: str) -> str:
    value = arguments.get(name)
    if not isinstance(value, str) or not value:
        raise ToolExecutionError(f"Argument {name} must be a non-empty string.")
    return value


def _string_arg(arguments: JsonObject, name: str, default: str) -> str:
    value = arguments.get(name, default)
    return value if isinstance(value, str) else default


def _bool_arg(arguments: JsonObject, name: str, default: bool) -> bool:
    value = arguments.get(name, default)
    return value if isinstance(value, bool) else default


def _int_arg(arguments: JsonObject, name: str, default: int) -> int:
    value = arguments.get(name, default)
    return value if isinstance(value, int) else default


def _string_list_arg(arguments: JsonObject, name: str) -> list[str]:
    value = arguments.get(name, [])
    if not isinstance(value, list):
        raise ToolExecutionError(f"Argument {name} must be an array of strings.")
    if not all(isinstance(item, str) for item in value):
        raise ToolExecutionError(f"Argument {name} must be an array of strings.")
    return value


def _prompt_choices_arg(arguments: JsonObject) -> tuple[UserPromptChoice, ...]:
    value = arguments.get("choices", [])
    if not isinstance(value, list):
        raise ToolExecutionError("Argument choices must be an array.")
    choices: list[UserPromptChoice] = []
    seen_ids: set[str] = set()
    for item in value:
        if not isinstance(item, dict):
            raise ToolExecutionError("Argument choices must contain objects.")
        choice_id = item.get("id")
        label = item.get("label")
        description = item.get("description", "")
        if not isinstance(choice_id, str) or not choice_id:
            raise ToolExecutionError("Choice id must be a non-empty string.")
        if not isinstance(label, str) or not label:
            raise ToolExecutionError("Choice label must be a non-empty string.")
        if not isinstance(description, str):
            raise ToolExecutionError("Choice description must be a string.")
        if choice_id in seen_ids:
            raise ToolExecutionError(f"Choice id is duplicated: {choice_id}")
        seen_ids.add(choice_id)
        choices.append(UserPromptChoice(id=choice_id, label=label, description=description))
    return tuple(choices)


def _json(value: JsonObject) -> str:
    return json.dumps(value, sort_keys=True)


def _object_schema(properties: JsonObject, required: list[str] | None = None) -> JsonObject:
    return {
        "type": "object",
        "properties": properties,
        "required": required or [],
        "additionalProperties": False,
    }


def _filesystem_list_spec() -> ToolSpec:
    return ToolSpec(
        name="filesystem.list",
        description="List files and directories inside the workspace.",
        input_schema=_object_schema(
            {
                "path": {"type": "string", "default": "."},
                "recursive": {"type": "boolean", "default": False},
                "glob": {"type": "string", "default": "*"},
                "max_entries": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 200},
            }
        ),
        output_schema=_object_schema({"root": {"type": "string"}, "entries": {"type": "array"}}),
        permissions=ToolPermission(filesystem="read", risk="low"),
    )


def _filesystem_read_spec() -> ToolSpec:
    return ToolSpec(
        name="filesystem.read",
        description="Read a UTF-8 text file inside the workspace.",
        input_schema=_object_schema(
            {
                "path": {"type": "string"},
                "start_line": {"type": "integer", "minimum": 1, "default": 1},
                "max_lines": {"type": "integer", "minimum": 1},
                "max_bytes": {"type": "integer", "minimum": 1, "maximum": 100000, "default": 32768},
            },
            ["path"],
        ),
        output_schema=_object_schema(
            {
                "path": {"type": "string"},
                "start_line": {"type": "integer"},
                "line_count": {"type": "integer"},
                "content": {"type": "string"},
                "truncated": {"type": "boolean"},
            }
        ),
        permissions=ToolPermission(filesystem="read", risk="low"),
    )


def _filesystem_search_spec() -> ToolSpec:
    return ToolSpec(
        name="filesystem.search",
        description="Search text files inside the workspace.",
        input_schema=_object_schema(
            {
                "pattern": {"type": "string"},
                "path": {"type": "string", "default": "."},
                "glob": {"type": "string"},
                "regex": {"type": "boolean", "default": True},
                "case_sensitive": {"type": "boolean", "default": True},
                "max_matches": {"type": "integer", "minimum": 1, "maximum": 1000, "default": 100},
            },
            ["pattern"],
        ),
        output_schema=_object_schema(
            {"matches": {"type": "array"}, "truncated": {"type": "boolean"}}
        ),
        permissions=ToolPermission(filesystem="read", risk="low"),
    )


def _filesystem_write_spec() -> ToolSpec:
    return ToolSpec(
        name="filesystem.write",
        description="Create, overwrite, or append UTF-8 text inside the workspace.",
        input_schema=_object_schema(
            {
                "path": {"type": "string"},
                "content": {"type": "string"},
                "mode": {"type": "string", "enum": ["create", "overwrite", "append"]},
            },
            ["path", "content", "mode"],
        ),
        output_schema=_object_schema(
            {"path": {"type": "string"}, "bytes_written": {"type": "integer"}}
        ),
        permissions=ToolPermission(
            filesystem="write",
            approval_required=True,
            mutation=True,
            risk="high",
        ),
    )


def _filesystem_replace_spec() -> ToolSpec:
    return ToolSpec(
        name="filesystem.replace",
        description="Replace exact text inside a UTF-8 workspace file.",
        input_schema=_object_schema(
            {
                "path": {"type": "string"},
                "old": {"type": "string"},
                "new": {"type": "string"},
                "replace_all": {"type": "boolean", "default": False},
            },
            ["path", "old", "new"],
        ),
        output_schema=_object_schema(
            {"path": {"type": "string"}, "replacements": {"type": "integer"}}
        ),
        permissions=ToolPermission(
            filesystem="write",
            approval_required=True,
            mutation=True,
            risk="high",
        ),
    )


def _git_status_spec() -> ToolSpec:
    return ToolSpec(
        name="git.status",
        description="Return git porcelain status for the workspace.",
        input_schema=_object_schema({}),
        output_schema=_object_schema({"entries": {"type": "array"}, "raw": {"type": "string"}}),
        permissions=ToolPermission(filesystem="read", risk="low"),
    )


def _git_diff_spec() -> ToolSpec:
    return ToolSpec(
        name="git.diff",
        description="Return a bounded git diff for the workspace or specific paths.",
        input_schema=_object_schema({"paths": {"type": "array", "items": {"type": "string"}}}),
        output_schema=_object_schema(
            {
                "diff": {"type": "string"},
                "stderr": {"type": "string"},
                "exit_code": {"type": "integer"},
            }
        ),
        permissions=ToolPermission(filesystem="read", risk="low"),
        max_output_bytes=64_000,
    )


def _git_show_spec() -> ToolSpec:
    return ToolSpec(
        name="git.show",
        description="Inspect a git revision or pathspec with bounded output.",
        input_schema=_object_schema({"rev": {"type": "string"}, "path": {"type": "string"}}),
        output_schema=_object_schema(
            {
                "output": {"type": "string"},
                "stderr": {"type": "string"},
                "exit_code": {"type": "integer"},
            }
        ),
        permissions=ToolPermission(filesystem="read", risk="low"),
        max_output_bytes=64_000,
    )


def _shell_run_spec() -> ToolSpec:
    return ToolSpec(
        name="shell.run",
        description=(
            "Run a local command inside the workspace as structured argv. "
            "Use direct executable arguments only; pipes, redirects, glob expansion, "
            "and shell wrappers are not available."
        ),
        input_schema=_object_schema(
            {
                "argv": {"type": "array", "items": {"type": "string"}, "minItems": 1},
                "cwd": {"type": "string", "default": "."},
                "env": {"type": "object", "additionalProperties": {"type": "string"}},
                "timeout_seconds": {"type": "number", "minimum": 0.1, "maximum": 300},
                "max_output_bytes": {"type": "integer", "minimum": 1, "maximum": 100000},
            },
            ["argv"],
        ),
        output_schema=_object_schema(
            {
                "exit_code": {"type": "integer"},
                "stdout": {"type": "string"},
                "stderr": {"type": "string"},
                "cwd": {"type": "string"},
            }
        ),
        permissions=ToolPermission(
            filesystem="write",
            approval_required=True,
            mutation=True,
            risk="high",
        ),
        timeout_seconds=30.0,
        max_output_bytes=64_000,
    )


def _user_ask_spec() -> ToolSpec:
    choice_schema = _object_schema(
        {
            "id": {"type": "string", "minLength": 1},
            "label": {"type": "string", "minLength": 1},
            "description": {"type": "string"},
        },
        ["id", "label"],
    )
    return ToolSpec(
        name="user.ask",
        description=(
            "Ask the user one structured question and return their selected choice or "
            "free-form answer. Use this when the next step depends on user preference "
            "or missing requirements."
        ),
        input_schema=_object_schema(
            {
                "question": {"type": "string", "minLength": 1},
                "choices": {"type": "array", "items": choice_schema, "maxItems": 10},
                "allow_freeform": {"type": "boolean", "default": True},
            },
            ["question"],
        ),
        output_schema=_object_schema(
            {
                "answer": {"type": "string"},
                "choice_id": {"type": ["string", "null"]},
            }
        ),
        permissions=ToolPermission(working_root_required=False, risk="low"),
        max_output_bytes=4_000,
    )


def _echo_spec() -> ToolSpec:
    return ToolSpec(
        name="echo",
        description="Echo text back to the model or user.",
        input_schema=_object_schema({"text": {"type": "string"}}, ["text"]),
        permissions=ToolPermission(working_root_required=False, risk="low"),
    )
