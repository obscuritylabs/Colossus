"""Research source adapters for local repo, web search, and MCP."""

import html
import importlib
import json
import re
from dataclasses import dataclass, field
from html.parser import HTMLParser
from pathlib import Path
from typing import Any
from urllib.parse import parse_qs, quote_plus, unquote, urlparse, urlunparse

import httpx

from colossus.adapters.workspace import Workspace
from colossus.domain.errors import ToolExecutionError
from colossus.domain.research import ResearchSourceDraft

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


class WorkspaceRepoResearchProvider:
    def __init__(self, workspace: Workspace) -> None:
        self._workspace = workspace

    async def collect(self, query: str, *, max_results: int) -> tuple[ResearchSourceDraft, ...]:
        tokens = _query_tokens(query)
        if not tokens:
            return ()
        scored: list[tuple[int, Path, list[str]]] = []
        for path in _iter_text_files(self._workspace.root):
            try:
                text = path.read_text(encoding="utf-8")
            except (OSError, UnicodeDecodeError):
                continue
            score, snippets = _score_text(text, tokens)
            if score <= 0:
                continue
            scored.append((score, path, snippets))
        drafts: list[ResearchSourceDraft] = []
        for _score, path, snippets in sorted(scored, key=lambda item: (-item[0], str(item[1])))[
            :max_results
        ]:
            try:
                relative = self._workspace.relative(path)
            except ToolExecutionError:
                continue
            content = "\n".join(snippets) or _head(path)
            drafts.append(
                ResearchSourceDraft(
                    kind="repo",
                    title=relative,
                    uri=relative,
                    content=content,
                    query=query,
                    metadata={"path": relative},
                )
            )
        return tuple(drafts)


class DisabledSearchProvider:
    @property
    def configured(self) -> bool:
        return False

    async def collect(self, query: str, *, max_results: int) -> tuple[ResearchSourceDraft, ...]:
        del query, max_results
        return ()


class DuckDuckGoSearchProvider:
    def __init__(
        self,
        *,
        endpoint: str = "https://duckduckgo.com/html/",
        user_agent: str = "colossus-agent/0.1",
        transport: httpx.AsyncBaseTransport | None = None,
    ) -> None:
        self._endpoint = endpoint
        self._user_agent = user_agent
        self._transport = transport

    @property
    def configured(self) -> bool:
        return True

    async def collect(self, query: str, *, max_results: int) -> tuple[ResearchSourceDraft, ...]:
        url = f"{self._endpoint}?q={quote_plus(query)}"
        async with httpx.AsyncClient(
            follow_redirects=True,
            timeout=20.0,
            transport=self._transport,
        ) as client:
            response = await client.get(url, headers={"User-Agent": self._user_agent})
            response.raise_for_status()
        parser = _DuckDuckGoParser(max_results)
        parser.feed(response.text)
        drafts = []
        for result in parser.results[:max_results]:
            drafts.append(
                ResearchSourceDraft(
                    kind="web",
                    title=result["title"],
                    uri=result["url"],
                    content=result["snippet"],
                    query=query,
                    metadata={"search_provider": "duckduckgo"},
                )
            )
        return tuple(drafts)


class SearxngSearchProvider:
    def __init__(
        self,
        *,
        endpoint: str = "http://localhost:8080/search",
        user_agent: str = "colossus-agent/0.1",
        api_key: str | None = None,
        auth_header: str = "Authorization",
        auth_scheme: str = "bearer",
        transport: httpx.AsyncBaseTransport | None = None,
    ) -> None:
        self._endpoint = _normalize_searxng_endpoint(endpoint)
        self._user_agent = user_agent
        self._api_key = api_key
        self._auth_header = auth_header
        self._auth_scheme = auth_scheme
        self._transport = transport

    @property
    def configured(self) -> bool:
        return True

    async def collect(self, query: str, *, max_results: int) -> tuple[ResearchSourceDraft, ...]:
        headers = {"User-Agent": self._user_agent}
        if self._api_key:
            headers[self._auth_header] = _auth_header_value(self._api_key, self._auth_scheme)
        async with httpx.AsyncClient(
            follow_redirects=True,
            timeout=20.0,
            transport=self._transport,
        ) as client:
            response = await client.get(
                self._endpoint,
                params={"q": query, "format": "json"},
                headers=headers,
            )
            response.raise_for_status()
        try:
            payload = response.json()
        except ValueError as exc:
            raise ToolExecutionError("SearXNG returned a non-JSON response.") from exc
        return _searxng_results(payload, query=query, max_results=max_results)


@dataclass(frozen=True)
class McpResearchToolRuntime:
    server: str
    tool: str
    arguments: dict[str, object] = field(default_factory=dict)
    title: str = ""


@dataclass(frozen=True)
class McpServerRuntime:
    name: str
    command: str
    args: tuple[str, ...] = ()
    env: dict[str, str] = field(default_factory=dict)
    allowed_tools: tuple[str, ...] = ()
    research_tools: tuple[McpResearchToolRuntime, ...] = ()


class DisabledMcpGateway:
    @property
    def configured(self) -> bool:
        return False

    async def list_servers(self) -> tuple[dict[str, object], ...]:
        return ()

    async def list_tools(self, server: str | None = None) -> tuple[dict[str, object], ...]:
        del server
        return ()

    async def call_tool(
        self,
        *,
        server: str,
        tool: str,
        arguments: dict[str, object],
    ) -> dict[str, object]:
        del server, tool, arguments
        raise ToolExecutionError("MCP calls require an explicitly configured MCP adapter.")

    async def collect(self, query: str, *, max_results: int) -> tuple[ResearchSourceDraft, ...]:
        del query, max_results
        return ()


class McpSdkGateway:
    """MCP stdio gateway backed by the official MCP Python SDK when installed."""

    def __init__(self, servers: tuple[McpServerRuntime, ...]) -> None:
        self._servers = {server.name: server for server in servers}

    @property
    def configured(self) -> bool:
        return bool(self._servers)

    async def list_servers(self) -> tuple[dict[str, object], ...]:
        return tuple(
            {
                "name": server.name,
                "transport": "stdio",
                "command": server.command,
                "allowed_tools": list(server.allowed_tools),
            }
            for server in self._servers.values()
        )

    async def list_tools(self, server: str | None = None) -> tuple[dict[str, object], ...]:
        selected = self._selected_servers(server)
        tools: list[dict[str, object]] = []
        for runtime in selected:
            async with self._session(runtime) as session:
                result = await session.list_tools()
            for tool in getattr(result, "tools", ()):
                name = str(getattr(tool, "name", ""))
                if runtime.allowed_tools and name not in runtime.allowed_tools:
                    continue
                tools.append(
                    {
                        "server": runtime.name,
                        "name": name,
                        "description": str(getattr(tool, "description", "")),
                    }
                )
        return tuple(tools)

    async def call_tool(
        self,
        *,
        server: str,
        tool: str,
        arguments: dict[str, object],
    ) -> dict[str, object]:
        runtime = self._require_server(server)
        if runtime.allowed_tools and tool not in runtime.allowed_tools:
            raise ToolExecutionError(f"MCP tool is not allowlisted: {server}/{tool}")
        async with self._session(runtime) as session:
            result = await session.call_tool(tool, arguments)
        return {"server": server, "tool": tool, "result": _jsonable(result)}

    async def collect(self, query: str, *, max_results: int) -> tuple[ResearchSourceDraft, ...]:
        drafts: list[ResearchSourceDraft] = []
        for runtime in self._servers.values():
            for tool_config in runtime.research_tools:
                if len(drafts) >= max_results:
                    return tuple(drafts)
                if tool_config.tool not in runtime.allowed_tools:
                    continue
                arguments = _template_arguments(tool_config.arguments, query)
                result = await self.call_tool(
                    server=runtime.name,
                    tool=tool_config.tool,
                    arguments=arguments,
                )
                content = json.dumps(result["result"], sort_keys=True)
                drafts.append(
                    ResearchSourceDraft(
                        kind="mcp",
                        title=tool_config.title or f"{runtime.name}/{tool_config.tool}",
                        uri=f"mcp://{runtime.name}/{tool_config.tool}",
                        content=content[:20_000],
                        query=query,
                        metadata={"server": runtime.name, "tool": tool_config.tool},
                    )
                )
        return tuple(drafts)

    def _selected_servers(self, server: str | None) -> tuple[McpServerRuntime, ...]:
        if server:
            return (self._require_server(server),)
        return tuple(self._servers.values())

    def _require_server(self, server: str) -> McpServerRuntime:
        try:
            return self._servers[server]
        except KeyError as exc:
            raise ToolExecutionError(f"Unknown MCP server: {server}") from exc

    def _session(self, server: McpServerRuntime) -> Any:
        try:
            mcp = importlib.import_module("mcp")
            stdio = importlib.import_module("mcp.client.stdio")
        except ImportError as exc:
            raise ToolExecutionError(
                "MCP calls require the official MCP Python SDK to be installed."
            ) from exc
        client_session = mcp.ClientSession
        parameters_type = mcp.StdioServerParameters
        stdio_client = stdio.stdio_client
        parameters = parameters_type(
            command=server.command,
            args=list(server.args),
            env=server.env or None,
        )
        return _McpSessionContext(stdio_client, client_session, parameters)


class _McpSessionContext:
    def __init__(self, stdio_client: Any, client_session: Any, parameters: Any) -> None:
        self._stdio_client = stdio_client
        self._client_session = client_session
        self._parameters = parameters
        self._stdio_context: Any = None
        self._session_context: Any = None
        self._session: Any = None

    async def __aenter__(self) -> Any:
        self._stdio_context = self._stdio_client(self._parameters)
        read, write = await self._stdio_context.__aenter__()
        self._session_context = self._client_session(read, write)
        self._session = await self._session_context.__aenter__()
        await self._session.initialize()
        return self._session

    async def __aexit__(self, exc_type: object, exc: object, tb: object) -> None:
        if self._session_context is not None:
            await self._session_context.__aexit__(exc_type, exc, tb)
        if self._stdio_context is not None:
            await self._stdio_context.__aexit__(exc_type, exc, tb)


class _DuckDuckGoParser(HTMLParser):
    def __init__(self, max_results: int) -> None:
        super().__init__()
        self.results: list[dict[str, str]] = []
        self._max_results = max_results
        self._capture_title = False
        self._capture_snippet = False
        self._current_url = ""
        self._current_title: list[str] = []
        self._current_snippet: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        attr = {name: value or "" for name, value in attrs}
        classes = set(attr.get("class", "").split())
        if tag == "a" and "result__a" in classes and len(self.results) < self._max_results:
            self._capture_title = True
            self._current_url = _normalize_duckduckgo_url(attr.get("href", ""))
            self._current_title = []
            self._current_snippet = []
        elif "result__snippet" in classes and self._current_url:
            self._capture_snippet = True

    def handle_endtag(self, tag: str) -> None:
        if tag == "a" and self._capture_title:
            self._capture_title = False
            title = html.unescape("".join(self._current_title)).strip()
            if title and self._current_url:
                self.results.append({"title": title, "url": self._current_url, "snippet": ""})
            return
        if self._capture_snippet and tag in {"a", "div"}:
            self._capture_snippet = False
            snippet = html.unescape("".join(self._current_snippet)).strip()
            if snippet and self.results:
                self.results[-1]["snippet"] = snippet

    def handle_data(self, data: str) -> None:
        if self._capture_title:
            self._current_title.append(data)
        elif self._capture_snippet:
            self._current_snippet.append(data)


def _normalize_duckduckgo_url(value: str) -> str:
    if value.startswith("//"):
        value = f"https:{value}"
    parsed = urlparse(value)
    if parsed.netloc.endswith("duckduckgo.com") and parsed.path.startswith("/l/"):
        target = parse_qs(parsed.query).get("uddg", [""])[0]
        if target:
            return unquote(target)
    return value


def _normalize_searxng_endpoint(value: str) -> str:
    parsed = urlparse(value)
    path = parsed.path.rstrip("/")
    if path.endswith("/search"):
        return urlunparse(parsed._replace(path=path))
    normalized_path = f"{path}/search" if path else "/search"
    return urlunparse(parsed._replace(path=normalized_path))


def _auth_header_value(api_key: str, auth_scheme: str) -> str:
    if auth_scheme == "raw":
        return api_key
    return f"Bearer {api_key}"


def _searxng_results(
    payload: object,
    *,
    query: str,
    max_results: int,
) -> tuple[ResearchSourceDraft, ...]:
    if not isinstance(payload, dict):
        raise ToolExecutionError("SearXNG response must be a JSON object.")
    raw_results = payload.get("results", ())
    if not isinstance(raw_results, list):
        raise ToolExecutionError("SearXNG response field 'results' must be an array.")
    drafts: list[ResearchSourceDraft] = []
    for item in raw_results:
        if len(drafts) >= max_results:
            break
        if not isinstance(item, dict):
            continue
        uri = _string_value(item.get("url"))
        if not uri:
            continue
        title = _string_value(item.get("title")) or uri
        content = _string_value(item.get("content")) or _string_value(item.get("snippet"))
        drafts.append(
            ResearchSourceDraft(
                kind="web",
                title=title,
                uri=uri,
                content=content,
                query=query,
                metadata=_searxng_metadata(item),
            )
        )
    return tuple(drafts)


def _searxng_metadata(item: dict[object, object]) -> dict[str, str]:
    metadata = {"search_provider": "searxng"}
    engine = _string_value(item.get("engine"))
    if engine:
        metadata["engine"] = engine
    engines = item.get("engines")
    if isinstance(engines, list):
        engine_names = [value for value in (_string_value(entry) for entry in engines) if value]
        if engine_names:
            metadata["engines"] = ",".join(engine_names)
    category = _string_value(item.get("category"))
    if category:
        metadata["category"] = category
    return metadata


def _string_value(value: object) -> str:
    return value.strip() if isinstance(value, str) else ""


def _query_tokens(query: str) -> tuple[str, ...]:
    stop = {"the", "and", "for", "with", "that", "this", "into", "from", "how", "what"}
    tokens = []
    for token in re.findall(r"[A-Za-z0-9_./-]+", query.lower()):
        if len(token) < 3 or token in stop:
            continue
        tokens.append(token)
    return tuple(dict.fromkeys(tokens[:12]))


def _iter_text_files(root: Path) -> list[Path]:
    paths: list[Path] = []
    for path in sorted(root.rglob("*")):
        if not path.is_file() or _is_excluded(root, path):
            continue
        try:
            data = path.read_bytes()[:2048]
        except OSError:
            continue
        if b"\x00" in data:
            continue
        paths.append(path)
    return paths


def _is_excluded(root: Path, path: Path) -> bool:
    try:
        parts = path.relative_to(root).parts
    except ValueError:
        return True
    return bool(EXCLUDED_REPO_DIRS.intersection(parts))


def _score_text(text: str, tokens: tuple[str, ...]) -> tuple[int, list[str]]:
    lower_lines = text.lower().splitlines()
    original_lines = text.splitlines()
    score = 0
    snippets: list[str] = []
    for index, lower in enumerate(lower_lines):
        line_score = sum(1 for token in tokens if token in lower)
        if line_score <= 0:
            continue
        score += line_score
        if len(snippets) < 8:
            snippets.append(f"{index + 1}: {original_lines[index].strip()}")
    return score, snippets


def _head(path: Path) -> str:
    try:
        return "\n".join(path.read_text(encoding="utf-8").splitlines()[:12])
    except (OSError, UnicodeDecodeError):
        return ""


def _jsonable(value: Any) -> object:
    if hasattr(value, "model_dump"):
        return value.model_dump(mode="json")
    if isinstance(value, dict):
        return {str(key): _jsonable(item) for key, item in value.items()}
    if isinstance(value, list | tuple):
        return [_jsonable(item) for item in value]
    if isinstance(value, str | int | float | bool) or value is None:
        return value
    return str(value)


def _template_arguments(value: object, query: str) -> dict[str, object]:
    templated = _template_value(value, query)
    if not isinstance(templated, dict):
        raise ToolExecutionError("MCP research tool arguments must be an object.")
    return {str(key): item for key, item in templated.items()}


def _template_value(value: object, query: str) -> object:
    if isinstance(value, str):
        return value.replace("{query}", query)
    if isinstance(value, list):
        return [_template_value(item, query) for item in value]
    if isinstance(value, dict):
        return {str(key): _template_value(item, query) for key, item in value.items()}
    return value
