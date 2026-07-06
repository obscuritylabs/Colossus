"""Deep research orchestration service."""

import re
from collections.abc import Callable
from typing import Literal
from uuid import uuid4

from colossus.application.model_router import ModelRouter
from colossus.domain.errors import ColossusError
from colossus.domain.events import (
    FinalOutputEvent,
    ModelDeltaEvent,
    ResearchProgressEvent,
    ResearchStatusEvent,
    RunEvent,
)
from colossus.domain.messages import AssistantMessage, Message, UserMessage
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
SESSION_CONTEXT_MESSAGE_LIMIT = 12
SESSION_CONTEXT_MESSAGE_CHARS = 900
SESSION_CONTEXT_MAX_CHARS = 6_000


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
        question_text = question.strip()
        prior_messages = await self._state_store.list_messages(session_id)
        session_context = _session_context(prior_messages)
        run = ResearchRun(
            id=self._run_id_factory(),
            session_id=session_id,
            question=question_text,
            depth=depth,
            source_kinds=source_kinds,
        )
        await self._state_store.save_research_run(run)
        await self._state_store.append_message(
            session_id,
            run.id,
            UserMessage(content=question_text),
        )
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
            queries, planner_source = await self._plan_queries(
                run.question,
                depth,
                session_context=session_context,
            )
            await self._emit_progress(
                run,
                phase="planning",
                action="queries",
                status="completed",
                message=f"Planned {len(queries)} query item(s) via {planner_source}.",
                total=len(queries),
                details={
                    "queries": list(queries),
                    "depth": depth,
                    "planner_source": planner_source,
                },
            )
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
            await self._emit_progress(
                run,
                phase="collecting",
                action="sources",
                status="completed",
                message=f"Saved {len(sources)} source record(s).",
                sources_collected=len(sources),
                details={
                    "sources": _source_detail_preview(sources),
                    "total_sources": len(sources),
                },
            )
            await self._emit(
                run,
                phase="workers",
                message="Extracting source-backed claims.",
                sources_collected=len(sources),
            )
            claims = await self._build_claims(run, sources)
            for claim in claims:
                await self._state_store.save_research_claim(claim)
            await self._emit_progress(
                run,
                phase="workers",
                action="claims",
                status="completed",
                message=f"Extracted {len(claims)} source-backed claim(s).",
                sources_collected=len(sources),
                claims_collected=len(claims),
            )
            await self._emit(
                run,
                phase="synthesis",
                message="Synthesizing cited research report.",
                sources_collected=len(sources),
            )
            report = await self._synthesize_report(
                run,
                sources,
                claims,
                tuple(warnings),
                session_context=session_context,
            )
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
            await self._state_store.append_message(
                session_id,
                completed.id,
                AssistantMessage(content=report),
            )
            await self._emit(
                completed,
                phase="completed",
                message="Research report complete.",
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

    async def _plan_queries(
        self,
        question: str,
        depth: ResearchDepth,
        *,
        session_context: str = "",
    ) -> tuple[tuple[str, ...], str]:
        if depth == "quick":
            return (question,), "quick"
        planned = await self._model_text(
            "research_planner",
            (
                "Return 2-6 short search queries as plain lines. No numbering. "
                "Use prior session context only to resolve references in the question."
            ),
            _planning_prompt(question, session_context),
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
            return queries[:limit], "deterministic_fallback"
        return queries[:limit], "model"

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
        for query_index, query in enumerate(queries, start=1):
            if len(drafts) >= max_sources:
                break
            if "repo" in source_kinds:
                await self._emit_progress(
                    run,
                    phase="collecting",
                    action="repo",
                    status="started",
                    message="Collecting repository evidence.",
                    query=query,
                    source_kind="repo",
                    current=query_index,
                    total=len(queries),
                    sources_collected=len(drafts),
                    details={"max_results": source_budget},
                )
                before = len(drafts)
                repo_drafts = await self._repo_provider.collect(query, max_results=source_budget)
                _extend_unique(drafts, repo_drafts, seen, max_sources)
                await self._emit_progress(
                    run,
                    phase="collecting",
                    action="repo",
                    status="completed",
                    message=f"Repository collection returned {len(repo_drafts)} result(s).",
                    query=query,
                    source_kind="repo",
                    current=query_index,
                    total=len(queries),
                    sources_collected=len(drafts),
                    details={
                        "results": len(repo_drafts),
                        "added": len(drafts) - before,
                        "max_results": source_budget,
                    },
                )
            if "web" in source_kinds and len(drafts) < max_sources:
                if self._search_provider is None or not self._search_provider.configured:
                    _append_once(warnings, "web search is not configured")
                    await self._emit_progress(
                        run,
                        phase="collecting",
                        action="web",
                        status="skipped",
                        message="Web search is not configured.",
                        query=query,
                        source_kind="web",
                        current=query_index,
                        total=len(queries),
                        sources_collected=len(drafts),
                        details={"configured": False},
                    )
                elif await self._approve("web.search", run.id, {"query": query}):
                    await self._emit_progress(
                        run,
                        phase="collecting",
                        action="web",
                        status="started",
                        message="Collecting web search evidence.",
                        query=query,
                        source_kind="web",
                        current=query_index,
                        total=len(queries),
                        sources_collected=len(drafts),
                        details={
                            "configured": True,
                            "approved": True,
                            "max_results": source_budget,
                        },
                    )
                    before = len(drafts)
                    try:
                        web_drafts = await self._search_provider.collect(
                            query,
                            max_results=source_budget,
                        )
                    except Exception as exc:
                        _append_once(warnings, f"web search failed: {exc}")
                        await self._emit_progress(
                            run,
                            phase="collecting",
                            action="web",
                            status="failed",
                            message=f"Web search failed: {exc}",
                            query=query,
                            source_kind="web",
                            current=query_index,
                            total=len(queries),
                            sources_collected=len(drafts),
                            details={"configured": True, "approved": True, "error": str(exc)},
                        )
                    else:
                        _extend_unique(drafts, web_drafts, seen, max_sources)
                        await self._emit_progress(
                            run,
                            phase="collecting",
                            action="web",
                            status="completed",
                            message=f"Web search returned {len(web_drafts)} result(s).",
                            query=query,
                            source_kind="web",
                            current=query_index,
                            total=len(queries),
                            sources_collected=len(drafts),
                            details={
                                "configured": True,
                                "approved": True,
                                "results": len(web_drafts),
                                "added": len(drafts) - before,
                            },
                        )
                else:
                    _append_once(warnings, "web search was not approved")
                    await self._emit_progress(
                        run,
                        phase="collecting",
                        action="web",
                        status="skipped",
                        message="Web search was not approved.",
                        query=query,
                        source_kind="web",
                        current=query_index,
                        total=len(queries),
                        sources_collected=len(drafts),
                        details={"configured": True, "approved": False},
                    )
            if "mcp" in source_kinds and len(drafts) < max_sources:
                if self._mcp_gateway is None or not self._mcp_gateway.configured:
                    _append_once(warnings, "MCP research collection is not configured")
                    await self._emit_progress(
                        run,
                        phase="collecting",
                        action="mcp",
                        status="skipped",
                        message="MCP research collection is not configured.",
                        query=query,
                        source_kind="mcp",
                        current=query_index,
                        total=len(queries),
                        sources_collected=len(drafts),
                        details={"configured": False},
                    )
                elif await self._approve("mcp.call", run.id, {"query": query}):
                    await self._emit_progress(
                        run,
                        phase="collecting",
                        action="mcp",
                        status="started",
                        message="Collecting MCP-backed evidence.",
                        query=query,
                        source_kind="mcp",
                        current=query_index,
                        total=len(queries),
                        sources_collected=len(drafts),
                        details={
                            "configured": True,
                            "approved": True,
                            "max_results": source_budget,
                        },
                    )
                    before = len(drafts)
                    try:
                        mcp_drafts = await self._mcp_gateway.collect(
                            query,
                            max_results=source_budget,
                        )
                    except Exception as exc:
                        _append_once(warnings, f"MCP collection failed: {exc}")
                        await self._emit_progress(
                            run,
                            phase="collecting",
                            action="mcp",
                            status="failed",
                            message=f"MCP collection failed: {exc}",
                            query=query,
                            source_kind="mcp",
                            current=query_index,
                            total=len(queries),
                            sources_collected=len(drafts),
                            details={"configured": True, "approved": True, "error": str(exc)},
                        )
                    else:
                        _extend_unique(drafts, mcp_drafts, seen, max_sources)
                        await self._emit_progress(
                            run,
                            phase="collecting",
                            action="mcp",
                            status="completed",
                            message=f"MCP collection returned {len(mcp_drafts)} result(s).",
                            query=query,
                            source_kind="mcp",
                            current=query_index,
                            total=len(queries),
                            sources_collected=len(drafts),
                            details={
                                "configured": True,
                                "approved": True,
                                "results": len(mcp_drafts),
                                "added": len(drafts) - before,
                            },
                        )
                else:
                    _append_once(warnings, "MCP research collection was not approved")
                    await self._emit_progress(
                        run,
                        phase="collecting",
                        action="mcp",
                        status="skipped",
                        message="MCP research collection was not approved.",
                        query=query,
                        source_kind="mcp",
                        current=query_index,
                        total=len(queries),
                        sources_collected=len(drafts),
                        details={"configured": True, "approved": False},
                    )
        if not drafts:
            _append_once(warnings, "no evidence sources were collected")
        await self._emit_progress(
            run,
            phase="collecting",
            action="collection",
            status="completed" if drafts else "skipped",
            message=f"Collected {len(drafts)} draft source(s).",
            sources_collected=len(drafts),
            details={"warnings": warnings, "max_sources": max_sources},
        )
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
        run: ResearchRun,
        sources: tuple[ResearchSource, ...],
    ) -> tuple[ResearchClaim, ...]:
        claims: list[ResearchClaim] = []
        for index, source in enumerate(sources, start=1):
            await self._emit_progress(
                run,
                phase="workers",
                action="claim",
                status="started",
                message=f"Extracting claim from [{source.label}] {source.title}.",
                current=index,
                total=len(sources),
                sources_collected=len(sources),
                claims_collected=len(claims),
                details={
                    "label": source.label,
                    "title": source.title,
                    "kind": source.kind,
                },
            )
            summary = await self._source_summary(source)
            claims.append(
                ResearchClaim(
                    id=f"{run.id}:claim:{index}",
                    run_id=run.id,
                    text=summary,
                    source_labels=(source.label,),
                )
            )
            await self._emit_progress(
                run,
                phase="workers",
                action="claim",
                status="completed",
                message=f"Extracted claim from [{source.label}].",
                current=index,
                total=len(sources),
                sources_collected=len(sources),
                claims_collected=len(claims),
                details={
                    "label": source.label,
                    "title": source.title,
                    "kind": source.kind,
                },
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
        *,
        session_context: str = "",
    ) -> str:
        prompt = _synthesis_prompt(
            run,
            sources,
            claims,
            warnings,
            session_context=session_context,
        )
        await self._emit_progress(
            run,
            phase="synthesis",
            action="prompt",
            status="completed",
            message=(
                f"Prepared synthesis prompt from {len(sources)} source(s) and "
                f"{len(claims)} claim(s)."
            ),
            sources_collected=len(sources),
            claims_collected=len(claims),
            details={
                "sources": len(sources),
                "claims": len(claims),
                "warnings": len(warnings),
                "prompt_chars": len(prompt),
            },
        )
        await self._emit_progress(
            run,
            phase="synthesis",
            action="model_synthesis",
            status="started",
            message="Asking research synthesizer for a cited report.",
            sources_collected=len(sources),
            claims_collected=len(claims),
        )
        text = await self._model_text(
            "research_synthesizer",
            (
                "Write an extensive cited research report, not a short brief. Only cite "
                "labels present in the provided source table. Use [R1] style citations."
            ),
            prompt,
        )
        labels = {source.label for source in sources}
        if (
            text
            and not text.startswith("[echo:")
            and _report_is_detailed_enough(text, labels, run.depth)
        ):
            await self._emit_progress(
                run,
                phase="synthesis",
                action="model_synthesis",
                status="completed",
                message="Accepted model-generated cited report.",
                sources_collected=len(sources),
                claims_collected=len(claims),
                details={"report_chars": len(text)},
            )
            return text
        await self._emit_progress(
            run,
            phase="synthesis",
            action="model_synthesis",
            status="skipped",
            message="Model report was unavailable or did not pass citation/detail checks.",
            sources_collected=len(sources),
            claims_collected=len(claims),
            details={"draft_chars": len(text), "labels": sorted(labels)},
        )
        report = _deterministic_report(run, sources, claims, warnings)
        await self._emit_progress(
            run,
            phase="synthesis",
            action="deterministic_fallback",
            status="completed",
            message="Built deterministic cited research report.",
            sources_collected=len(sources),
            claims_collected=len(claims),
            details={"report_chars": len(report)},
        )
        return report

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

    async def _emit_progress(
        self,
        run: ResearchRun,
        *,
        phase: str,
        action: str,
        status: Literal["started", "completed", "skipped", "failed"],
        message: str = "",
        query: str | None = None,
        source_kind: ResearchSourceKind | None = None,
        current: int = 0,
        total: int = 0,
        sources_collected: int = 0,
        claims_collected: int = 0,
        details: dict[str, object] | None = None,
    ) -> None:
        event = ResearchProgressEvent(
            research_id=run.id,
            phase=phase,
            action=action,
            status=status,
            message=_truncate(message, 240),
            query=_truncate(query, 160) if query is not None else None,
            source_kind=source_kind,
            current=current,
            total=total,
            sources_collected=sources_collected,
            claims_collected=claims_collected,
            details=_bounded_progress_details(details or {}),
        )
        await self._state_store.append_event(run.id, event)
        if self._event_observer is not None:
            self._event_observer(event)


def research_agent_tools() -> tuple[str, ...]:
    return tuple(sorted(RESEARCH_READ_ONLY_TOOLS))


def _session_context(messages: tuple[Message, ...]) -> str:
    lines: list[str] = []
    for message in messages[-SESSION_CONTEXT_MESSAGE_LIMIT:]:
        content = message.content.strip()
        if not content:
            continue
        role_label: str = message.role
        if message.role == "tool":
            role_label = f"tool:{getattr(message, 'name', 'tool')}"
        lines.append(f"{role_label}: {_truncate(content, SESSION_CONTEXT_MESSAGE_CHARS)}")
    return _truncate("\n".join(lines), SESSION_CONTEXT_MAX_CHARS)


def _planning_prompt(question: str, session_context: str) -> str:
    if not session_context:
        return question
    return (
        "Prior session context:\n"
        f"{session_context}\n\n"
        "Research question:\n"
        f"{question}"
    )


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


def _source_detail_preview(sources: tuple[ResearchSource, ...]) -> list[dict[str, str]]:
    preview: list[dict[str, str]] = []
    for source in sources[:5]:
        preview.append(
            {
                "label": source.label,
                "kind": source.kind,
                "title": _truncate(source.title, 120),
                "uri": _truncate(source.uri, 160),
            }
        )
    return preview


def _bounded_progress_details(details: dict[str, object]) -> dict[str, object]:
    bounded: dict[str, object] = {}
    for index, (key, value) in enumerate(details.items()):
        if index >= 12:
            bounded["truncated_detail_keys"] = len(details) - index
            break
        bounded[str(key)] = _bounded_progress_value(value)
    return bounded


def _bounded_progress_value(value: object) -> object:
    if isinstance(value, str):
        return _truncate(value, 240)
    if isinstance(value, int | float | bool) or value is None:
        return value
    if isinstance(value, dict):
        bounded: dict[str, object] = {}
        for index, (key, nested) in enumerate(value.items()):
            if index >= 8:
                bounded["truncated_keys"] = len(value) - index
                break
            bounded[str(key)] = _bounded_progress_value(nested)
        return bounded
    if isinstance(value, list | tuple):
        items = [_bounded_progress_value(item) for item in value[:8]]
        if len(value) > 8:
            items.append({"truncated_items": len(value) - 8})
        return items
    return _truncate(str(value), 240)


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
    *,
    session_context: str = "",
) -> str:
    source_lines = [
        f"[{source.label}] {source.kind} {source.title} {source.uri}\n{source.content[:1400]}"
        for source in sources
    ]
    claim_lines = [
        f"- {claim.text} {' '.join(f'[{label}]' for label in claim.source_labels)}"
        for claim in claims
    ]
    warning_text = "\n".join(f"- {warning}" for warning in warnings) or "- none"
    context_text = (
        "Prior session context:\n"
        f"{session_context}\n\n"
        "Use this context to interpret the question, but cite only collected sources "
        "for factual findings.\n\n"
        if session_context
        else ""
    )
    return (
        f"Question: {run.question}\n\n"
        f"{context_text}"
        "Write a substantial Markdown research report for a technical decision maker. "
        "Do not collapse the result into a short summary. Include these sections, using "
        "clear headings: Executive Summary, Methodology, Detailed Findings, Analysis, "
        "Caveats And Limitations, Source Table, and Unresolved Questions. Every factual "
        "finding must cite collected source labels such as [R1]. Explain how evidence "
        "supports the answer, where evidence is thin, and what follow-up would reduce "
        "uncertainty.\n\n"
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
        "# Research Report",
        "",
        f"Question: {run.question}",
        "",
        "## Executive Summary",
    ]
    if claims:
        summary_claims = claims[: min(3, len(claims))]
        lines.append(
            "The collected evidence supports the following source-backed answer:"
        )
        for claim in summary_claims:
            labels = " ".join(f"[{label}]" for label in claim.source_labels)
            lines.append(f"- {claim.text} {labels}")
    else:
        lines.append(
            "No source-backed answer could be produced from the available evidence."
        )
    lines.extend(
        [
            "",
            "## Methodology",
            f"- Depth: `{run.depth}`.",
            f"- Source lanes requested: {', '.join(run.source_kinds)}.",
            f"- Sources collected: {len(sources)}.",
            "- Claims were extracted from persisted source records and then assembled into "
            "a cited report.",
            "",
            "## Detailed Findings",
        ]
    )
    if claims:
        for index, claim in enumerate(claims, start=1):
            labels = " ".join(f"[{label}]" for label in claim.source_labels)
            source = _source_for_claim(claim, sources)
            heading = source.title if source is not None else f"Finding {index}"
            lines.extend(
                [
                    "",
                    f"### Finding {index}: {heading}",
                    "",
                    f"{claim.text} {labels}",
                    "",
                    _finding_context(source, claim),
                ]
            )
    else:
        lines.append("- No claims were extracted.")
    lines.extend(["", "## Analysis"])
    if claims:
        kinds = ", ".join(sorted({source.kind for source in sources})) or "none"
        lines.extend(
            [
                f"The evidence base spans {len(sources)} collected source record(s) across "
                f"{kinds} source type(s). The strongest support comes from claims that are "
                "directly tied to persisted source labels, which keeps the final answer "
                "auditable instead of relying on unsupported synthesis.",
                "",
                "The report should be treated as a source-grounded working document: it "
                "identifies what the collected evidence supports, calls out source-lane "
                "limitations, and keeps follow-up questions explicit rather than hiding "
                "them behind a confident summary.",
            ]
        )
    else:
        lines.append(
            "The collection phase did not produce evidence, so no substantive analysis can "
            "be made without additional sources."
        )
    lines.extend(["", "## Caveats And Limitations"])
    if warnings:
        for warning in warnings:
            lines.append(f"- {warning}.")
    else:
        lines.append("- Source collection completed without recorded adapter warnings.")
    lines.extend(
        [
            "",
            "## Source Table",
            "",
            "| Label | Type | Title | URI |",
            "| --- | --- | --- | --- |",
        ]
    )
    for source in sources:
        lines.append(
            f"| [{source.label}] | {source.kind} | {_escape_table(source.title)} | "
            f"{_escape_table(source.uri)} |"
        )
    if not sources:
        lines.append("| - | - | No sources collected | - |")
    lines.extend(
        [
            "",
            "## Source Notes",
            "",
            "| Label | Contribution |",
            "| --- | --- |",
        ]
    )
    for source in sources:
        lines.append(
            f"| [{source.label}] | {_escape_table(_source_contribution(source))} |"
        )
    if not sources:
        lines.append("| - | No source notes available. |")
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


def _report_is_detailed_enough(
    report: str,
    labels: set[str],
    depth: ResearchDepth,
) -> bool:
    if not _citations_valid(report, labels):
        return False
    heading_count = len(re.findall(r"^##\s+", report, flags=re.MULTILINE))
    minimum_headings = 4 if depth == "quick" else 5
    minimum_chars = {"quick": 600, "standard": 1000, "deep": 1400}[depth]
    return heading_count >= minimum_headings and len(report.strip()) >= minimum_chars


def _citations_valid(report: str, labels: set[str]) -> bool:
    citations = {match.strip("[]") for match in re.findall(r"\[R\d+\]", report)}
    return bool(citations) and citations.issubset(labels)


def _source_for_claim(
    claim: ResearchClaim,
    sources: tuple[ResearchSource, ...],
) -> ResearchSource | None:
    labels = set(claim.source_labels)
    for source in sources:
        if source.label in labels:
            return source
    return None


def _finding_context(source: ResearchSource | None, claim: ResearchClaim) -> str:
    if source is None:
        return "This finding is source-backed, but the source record could not be resolved."
    contribution = _source_contribution(source)
    labels = " ".join(f"[{label}]" for label in claim.source_labels)
    return (
        f"Evidence context: {contribution} The finding is tied to the persisted "
        f"{source.kind} source record {labels}."
    )


def _source_contribution(source: ResearchSource) -> str:
    content = " ".join(source.content.split())
    if not content:
        return f"{source.title} was collected as {source.kind} evidence."
    return _truncate(
        f"{source.title} contributed this evidence signal: {content}",
        420,
    )


def _escape_table(value: str) -> str:
    return value.replace("|", "\\|")


def _truncate(value: str, limit: int) -> str:
    if len(value) <= limit:
        return value
    return f"{value[: max(0, limit - 3)]}..."
