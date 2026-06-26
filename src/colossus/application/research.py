"""Deep research orchestration service."""

import re
from collections.abc import Callable
from uuid import uuid4

from colossus.application.model_router import ModelRouter
from colossus.domain.errors import ColossusError
from colossus.domain.events import FinalOutputEvent, ModelDeltaEvent, ResearchStatusEvent, RunEvent
from colossus.domain.messages import UserMessage
from colossus.domain.policy import PolicyDecision
from colossus.domain.requests import ModelRequest
from colossus.domain.research import (
    ResearchClaim,
    ResearchDepth,
    ResearchRun,
    ResearchSource,
    ResearchSourceDraft,
    ResearchSourceKind,
    utc_now_iso,
)
from colossus.domain.tools import ToolCall
from colossus.ports.approval import ApprovalHandler
from colossus.ports.audit import AuditSink
from colossus.ports.research import McpGateway, RepoResearchProvider, SearchProvider
from colossus.ports.state import StateStore

ResearchEventObserver = Callable[[RunEvent], None]
ResearchIdFactory = Callable[[], str]

RESEARCH_READ_ONLY_TOOLS = frozenset(
    {
        "filesystem.list",
        "filesystem.read",
        "filesystem.search",
        "git.status",
        "git.diff",
        "git.show",
        "repo.map",
        "repo.symbol_search",
        "repo.references",
        "repo.file_summary",
        "task.create",
        "task.update",
        "task.list",
        "decision.list",
        "memory.list",
        "memory.search",
        "agent.result",
        "agent.list",
        "web.fetch",
        "web.search",
        "docs.fetch",
        "mcp.servers",
        "mcp.tools",
        "mcp.call",
        "context.show",
        "context.snapshots",
        "tool.search",
    }
)


class ResearchService:
    def __init__(
        self,
        state_store: StateStore,
        audit_sink: AuditSink,
        *,
        repo_provider: RepoResearchProvider,
        model_router: ModelRouter | None = None,
        search_provider: SearchProvider | None = None,
        mcp_gateway: McpGateway | None = None,
        approval_handler: ApprovalHandler | None = None,
        auto_approve_network: bool = False,
        event_observer: ResearchEventObserver | None = None,
        run_id_factory: ResearchIdFactory | None = None,
    ) -> None:
        self._state_store = state_store
        self._audit_sink = audit_sink
        self._repo_provider = repo_provider
        self._model_router = model_router
        self._search_provider = search_provider
        self._mcp_gateway = mcp_gateway
        self._approval_handler = approval_handler
        self._auto_approve_network = auto_approve_network
        self._event_observer = event_observer
        self._run_id_factory = run_id_factory or (lambda: f"research-{uuid4().hex[:12]}")

    def set_event_observer(self, event_observer: ResearchEventObserver | None) -> None:
        self._event_observer = event_observer

    async def run(
        self,
        *,
        question: str,
        session_id: str,
        depth: ResearchDepth = "standard",
        source_kinds: tuple[ResearchSourceKind, ...] = ("repo", "web", "mcp"),
        max_sources: int = 20,
    ) -> ResearchRun:
        if not question.strip():
            raise ColossusError("Research question is required.")
        if max_sources < 1:
            raise ColossusError("max_sources must be at least 1.")
        run = ResearchRun(
            id=self._run_id_factory(),
            session_id=session_id,
            question=question.strip(),
            depth=depth,
            source_kinds=source_kinds,
        )
        await self._state_store.save_research_run(run)
        await self._audit_sink.record(
            "agent",
            "research.started",
            {
                "research_id": run.id,
                "session_id": session_id,
                "depth": depth,
                "sources": list(source_kinds),
            },
        )
        warnings: list[str] = []
        try:
            await self._emit(run, phase="planning", message="Planning research queries.")
            queries = await self._plan_queries(run.question, depth)
            await self._emit(
                run,
                phase="collecting",
                message=f"Collecting evidence for {len(queries)} query item(s).",
            )
            drafts, warnings = await self._collect_sources(
                run,
                queries,
                source_kinds=source_kinds,
                max_sources=max_sources,
            )
            sources = await self._save_sources(run.id, drafts[:max_sources])
            await self._emit(
                run,
                phase="workers",
                message="Extracting source-backed claims.",
                sources_collected=len(sources),
            )
            claims = await self._build_claims(run.id, sources)
            for claim in claims:
                await self._state_store.save_research_claim(claim)
            await self._emit(
                run,
                phase="synthesis",
                message="Synthesizing cited research brief.",
                sources_collected=len(sources),
            )
            report = await self._synthesize_report(run, sources, claims, tuple(warnings))
            completed = run.model_copy(
                update={
                    "status": "completed",
                    "report": report,
                    "warnings": tuple(warnings),
                    "updated_at": utc_now_iso(),
                    "completed_at": utc_now_iso(),
                }
            )
            await self._state_store.save_research_run(completed)
            await self._emit(
                completed,
                phase="completed",
                message="Research brief complete.",
                sources_collected=len(sources),
            )
            await self._audit_sink.record(
                "agent",
                "research.completed",
                {
                    "research_id": completed.id,
                    "session_id": session_id,
                    "sources": len(sources),
                    "claims": len(claims),
                },
            )
            return completed
        except Exception as exc:
            failed = run.model_copy(
                update={
                    "status": "failed",
                    "warnings": (*warnings, str(exc)),
                    "updated_at": utc_now_iso(),
                    "completed_at": utc_now_iso(),
                }
            )
            await self._state_store.save_research_run(failed)
            await self._emit(failed, phase="failed", message=str(exc))
            await self._audit_sink.record(
                "agent",
                "research.failed",
                {"research_id": failed.id, "session_id": session_id, "error": str(exc)},
            )
            if isinstance(exc, ColossusError):
                raise
            raise ColossusError(str(exc)) from exc

    async def get_run(self, run_id: str) -> ResearchRun:
        run = await self._state_store.get_research_run(run_id)
        if run is None:
            raise ColossusError(f"Research run not found: {run_id}")
        return run

    async def latest_run(self, session_id: str) -> ResearchRun | None:
        runs = await self._state_store.list_research_runs(session_id=session_id)
        return runs[0] if runs else None

    async def list_runs(self, session_id: str | None = None) -> tuple[ResearchRun, ...]:
        return await self._state_store.list_research_runs(session_id=session_id)

    async def list_sources(self, run_id: str) -> tuple[ResearchSource, ...]:
        return await self._state_store.list_research_sources(run_id)

    async def list_claims(self, run_id: str) -> tuple[ResearchClaim, ...]:
        return await self._state_store.list_research_claims(run_id)

    async def _plan_queries(self, question: str, depth: ResearchDepth) -> tuple[str, ...]:
        if depth == "quick":
            return (question,)
        planned = await self._model_text(
            "research_planner",
            "Return 2-6 short search queries as plain lines. No numbering.",
            question,
        )
        queries = tuple(
            dict.fromkeys(
                line.strip(" -0123456789.\t")
                for line in planned.splitlines()
                if line.strip(" -0123456789.\t")
            )
        )
        limit = 3 if depth == "standard" else 6
        if not queries or any(query.startswith("[echo:") for query in queries):
            queries = _deterministic_queries(question, limit)
        return queries[:limit]

    async def _collect_sources(
        self,
        run: ResearchRun,
        queries: tuple[str, ...],
        *,
        source_kinds: tuple[ResearchSourceKind, ...],
        max_sources: int,
    ) -> tuple[list[ResearchSourceDraft], list[str]]:
        drafts: list[ResearchSourceDraft] = []
        warnings: list[str] = []
        seen: set[tuple[str, str]] = set()
        source_budget = max(1, max_sources // max(len(source_kinds), 1))
        for query in queries:
            if len(drafts) >= max_sources:
                break
            if "repo" in source_kinds:
                repo_drafts = await self._repo_provider.collect(query, max_results=source_budget)
                _extend_unique(drafts, repo_drafts, seen, max_sources)
            if "web" in source_kinds and len(drafts) < max_sources:
                if self._search_provider is None or not self._search_provider.configured:
                    _append_once(warnings, "web search is not configured")
                elif await self._approve("web.search", run.id, {"query": query}):
                    try:
                        web_drafts = await self._search_provider.collect(
                            query,
                            max_results=source_budget,
                        )
                    except Exception as exc:
                        _append_once(warnings, f"web search failed: {exc}")
                    else:
                        _extend_unique(drafts, web_drafts, seen, max_sources)
                else:
                    _append_once(warnings, "web search was not approved")
            if "mcp" in source_kinds and len(drafts) < max_sources:
                if self._mcp_gateway is None or not self._mcp_gateway.configured:
                    _append_once(warnings, "MCP research collection is not configured")
                elif await self._approve("mcp.call", run.id, {"query": query}):
                    try:
                        mcp_drafts = await self._mcp_gateway.collect(
                            query,
                            max_results=source_budget,
                        )
                    except Exception as exc:
                        _append_once(warnings, f"MCP collection failed: {exc}")
                    else:
                        _extend_unique(drafts, mcp_drafts, seen, max_sources)
                else:
                    _append_once(warnings, "MCP research collection was not approved")
        if not drafts:
            _append_once(warnings, "no evidence sources were collected")
        return drafts, warnings

    async def _save_sources(
        self,
        run_id: str,
        drafts: list[ResearchSourceDraft],
    ) -> tuple[ResearchSource, ...]:
        sources: list[ResearchSource] = []
        for index, draft in enumerate(drafts, start=1):
            source = ResearchSource(
                id=f"{run_id}:source:{index}",
                run_id=run_id,
                label=f"R{index}",
                kind=draft.kind,
                title=draft.title or f"{draft.kind} source {index}",
                uri=draft.uri,
                content=_truncate(draft.content, 20_000),
                query=draft.query,
                metadata=draft.metadata,
            )
            await self._state_store.save_research_source(source)
            sources.append(source)
        return tuple(sources)

    async def _build_claims(
        self,
        run_id: str,
        sources: tuple[ResearchSource, ...],
    ) -> tuple[ResearchClaim, ...]:
        claims: list[ResearchClaim] = []
        for index, source in enumerate(sources, start=1):
            summary = await self._source_summary(source)
            claims.append(
                ResearchClaim(
                    id=f"{run_id}:claim:{index}",
                    run_id=run_id,
                    text=summary,
                    source_labels=(source.label,),
                )
            )
        return tuple(claims)

    async def _source_summary(self, source: ResearchSource) -> str:
        prompt = (
            f"Summarize this source in one concise factual claim.\n"
            f"Label: [{source.label}]\n"
            f"Title: {source.title}\n"
            f"Content:\n{source.content[:4000]}"
        )
        text = await self._model_text(
            "research_worker",
            "Extract one concise claim from a source. Do not invent facts.",
            prompt,
        )
        if text and not text.startswith("[echo:"):
            return _strip_citations(_first_line(text))
        return _fallback_claim(source)

    async def _synthesize_report(
        self,
        run: ResearchRun,
        sources: tuple[ResearchSource, ...],
        claims: tuple[ResearchClaim, ...],
        warnings: tuple[str, ...],
    ) -> str:
        prompt = _synthesis_prompt(run, sources, claims, warnings)
        text = await self._model_text(
            "research_synthesizer",
            (
                "Write a concise cited research brief. Only cite labels present in the "
                "provided source table. Use [R1] style citations."
            ),
            prompt,
        )
        labels = {source.label for source in sources}
        if text and not text.startswith("[echo:") and _citations_valid(text, labels):
            return text
        return _deterministic_report(run, sources, claims, warnings)

    async def _model_text(self, role: str, instructions: str, prompt: str) -> str:
        if self._model_router is None:
            return ""
        try:
            route = self._model_router.resolve(role)
        except ColossusError:
            return ""
        chunks: list[str] = []
        final = ""
        request = ModelRequest(
            model=route.profile.model,
            instructions=instructions,
            messages=(UserMessage(content=prompt),),
            tools=(),
        )
        try:
            async for event in route.provider.stream(request):
                if isinstance(event, ModelDeltaEvent):
                    chunks.append(event.text)
                elif isinstance(event, FinalOutputEvent):
                    final = event.text
        except Exception:
            return ""
        return final or "".join(chunks)

    async def _approve(self, tool: str, research_id: str, arguments: dict[str, object]) -> bool:
        if self._auto_approve_network:
            await self._audit_sink.record(
                "agent",
                "research.auto_approved",
                {"research_id": research_id, "tool": tool},
            )
            return True
        if self._approval_handler is None:
            await self._audit_sink.record(
                "agent",
                "research.approval_unavailable",
                {"research_id": research_id, "tool": tool},
            )
            return False
        call = ToolCall(
            call_id=f"{research_id}:{tool}",
            name=tool,
            arguments=arguments,
        )
        decision = PolicyDecision(
            decision="requires_approval",
            reason=f"Research source collection may access network through {tool}.",
        )
        approved = await self._approval_handler.approve(call, decision)
        await self._audit_sink.record(
            "agent",
            "research.approval",
            {"research_id": research_id, "tool": tool, "approved": approved},
        )
        return approved

    async def _emit(
        self,
        run: ResearchRun,
        *,
        phase: str,
        message: str = "",
        sources_collected: int = 0,
    ) -> None:
        event = ResearchStatusEvent(
            research_id=run.id,
            status=run.status,
            phase=phase,
            message=message,
            sources_collected=sources_collected,
        )
        await self._state_store.append_event(run.id, event)
        if self._event_observer is not None:
            self._event_observer(event)


def research_agent_tools() -> tuple[str, ...]:
    return tuple(sorted(RESEARCH_READ_ONLY_TOOLS))


def _deterministic_queries(question: str, limit: int) -> tuple[str, ...]:
    tokens = re.findall(r"[A-Za-z0-9_./-]+", question)
    compact = " ".join(tokens[:8]) or question
    queries = (question, compact, f"{compact} evidence", f"{compact} docs")
    return tuple(dict.fromkeys(queries))[:limit]


def _extend_unique(
    target: list[ResearchSourceDraft],
    drafts: tuple[ResearchSourceDraft, ...],
    seen: set[tuple[str, str]],
    limit: int,
) -> None:
    for draft in drafts:
        key = (draft.kind, draft.uri or draft.title)
        if key in seen:
            continue
        seen.add(key)
        target.append(draft)
        if len(target) >= limit:
            return


def _append_once(values: list[str], value: str) -> None:
    if value not in values:
        values.append(value)


def _fallback_claim(source: ResearchSource) -> str:
    for line in source.content.splitlines():
        stripped = line.strip()
        if stripped:
            return _truncate(stripped, 240)
    return _truncate(f"{source.title} is relevant to the research question.", 240)


def _first_line(value: str) -> str:
    for line in value.splitlines():
        if line.strip():
            return line.strip()
    return ""


def _strip_citations(value: str) -> str:
    return re.sub(r"\s*\[R\d+\]", "", value).strip()


def _synthesis_prompt(
    run: ResearchRun,
    sources: tuple[ResearchSource, ...],
    claims: tuple[ResearchClaim, ...],
    warnings: tuple[str, ...],
) -> str:
    source_lines = [
        f"[{source.label}] {source.kind} {source.title} {source.uri}\n{source.content[:1200]}"
        for source in sources
    ]
    claim_lines = [
        f"- {claim.text} {' '.join(f'[{label}]' for label in claim.source_labels)}"
        for claim in claims
    ]
    warning_text = "\n".join(f"- {warning}" for warning in warnings) or "- none"
    return (
        f"Question: {run.question}\n\n"
        f"Sources:\n{chr(10).join(source_lines)}\n\n"
        f"Candidate claims:\n{chr(10).join(claim_lines)}\n\n"
        f"Collection warnings:\n{warning_text}"
    )


def _deterministic_report(
    run: ResearchRun,
    sources: tuple[ResearchSource, ...],
    claims: tuple[ResearchClaim, ...],
    warnings: tuple[str, ...],
) -> str:
    lines = [
        "# Research Brief",
        "",
        f"Question: {run.question}",
        "",
        "## Answer",
    ]
    if claims:
        first = claims[0]
        labels = " ".join(f"[{label}]" for label in first.source_labels)
        lines.append(f"The collected evidence most directly supports: {first.text} {labels}")
    else:
        lines.append("No source-backed answer could be produced from the available evidence.")
    lines.extend(["", "## Key Findings"])
    if claims:
        for claim in claims[:8]:
            labels = " ".join(f"[{label}]" for label in claim.source_labels)
            lines.append(f"- {claim.text} {labels}")
    else:
        lines.append("- No claims were extracted.")
    lines.extend(["", "## Caveats"])
    if warnings:
        for warning in warnings:
            lines.append(f"- {warning}.")
    else:
        lines.append("- Source collection completed without recorded adapter warnings.")
    lines.extend(
        ["", "## Sources", "", "| Label | Type | Title | URI |", "| --- | --- | --- | --- |"]
    )
    for source in sources:
        lines.append(
            f"| [{source.label}] | {source.kind} | {_escape_table(source.title)} | "
            f"{_escape_table(source.uri)} |"
        )
    if not sources:
        lines.append("| - | - | No sources collected | - |")
    lines.extend(["", "## Unresolved Questions"])
    if "web" in run.source_kinds and any("web search" in warning for warning in warnings):
        lines.append(
            "- Fresh external corroboration is still needed once web search is configured."
        )
    elif "mcp" in run.source_kinds and any("MCP" in warning for warning in warnings):
        lines.append("- Connected MCP sources could add private or organization-specific evidence.")
    else:
        lines.append("- None identified from the collected evidence.")
    return "\n".join(lines).strip()


def _citations_valid(report: str, labels: set[str]) -> bool:
    citations = {match.strip("[]") for match in re.findall(r"\[R\d+\]", report)}
    return bool(citations) and citations.issubset(labels)


def _escape_table(value: str) -> str:
    return value.replace("|", "\\|")


def _truncate(value: str, limit: int) -> str:
    if len(value) <= limit:
        return value
    return f"{value[: max(0, limit - 3)]}..."
