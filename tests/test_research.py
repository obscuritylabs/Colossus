import httpx
import pytest

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.research_sources import SearxngSearchProvider
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.application.approvals import AllowAllApprovalHandler
from colossus.application.defaults import research_agent
from colossus.application.model_router import ModelRoute, ModelRouter
from colossus.application.research import ResearchService
from colossus.domain.errors import ToolExecutionError
from colossus.domain.events import FinalOutputEvent
from colossus.domain.messages import AssistantMessage, UserMessage
from colossus.domain.models import ResolvedModelProfile
from colossus.domain.requests import ModelRequest
from colossus.domain.research import ResearchSourceDraft
from colossus.infrastructure import container as container_module
from colossus.infrastructure.config import SearchConfig


class FakeRepoProvider:
    async def collect(self, query: str, *, max_results: int) -> tuple[ResearchSourceDraft, ...]:
        del max_results
        return (
            ResearchSourceDraft(
                kind="repo",
                title="docs/example.md",
                uri="docs/example.md",
                content=f"Deep research evidence for {query}",
                query=query,
            ),
        )


class FakeSearchProvider:
    @property
    def configured(self) -> bool:
        return True

    async def collect(self, query: str, *, max_results: int) -> tuple[ResearchSourceDraft, ...]:
        del max_results
        return (
            ResearchSourceDraft(
                kind="web",
                title="External result",
                uri="https://example.com/research",
                content=f"External evidence for {query}",
                query=query,
            ),
        )


class ShortReportProvider:
    name = "short-report"

    def capabilities(self) -> tuple[object, ...]:
        return ()

    async def check_readiness(self) -> object:
        raise NotImplementedError

    async def list_models(self) -> tuple[object, ...]:
        return ()

    async def stream(self, request: ModelRequest):
        if "search queries" in request.instructions:
            yield FinalOutputEvent(text="research report detail")
            return
        if "Extract one concise claim" in request.instructions:
            yield FinalOutputEvent(text="Research mode should produce a substantial report.")
            return
        yield FinalOutputEvent(text="Too short [R1]")


class CapturingResearchProvider:
    name = "capturing-research"

    def __init__(self) -> None:
        self.prompts: list[str] = []

    def capabilities(self) -> tuple[object, ...]:
        return ()

    async def check_readiness(self) -> object:
        raise NotImplementedError

    async def list_models(self) -> tuple[object, ...]:
        return ()

    async def stream(self, request: ModelRequest):
        prompt = request.messages[0].content
        self.prompts.append(prompt)
        if "search queries" in request.instructions:
            yield FinalOutputEvent(text="session aware search query")
            return
        if "Extract one concise claim" in request.instructions:
            yield FinalOutputEvent(text="The collected source supports the session-aware answer.")
            return
        yield FinalOutputEvent(text=_long_captured_report())


def _short_report_router() -> ModelRouter:
    profile = ResolvedModelProfile(
        role="research_synthesizer",
        profile_name="short",
        provider="echo",
        model="short-model",
    )
    provider = ShortReportProvider()
    return ModelRouter(
        {
            role: ModelRoute(
                role=role,
                profile_name="short",
                provider=provider,  # type: ignore[arg-type]
                profile=profile.model_copy(update={"role": role}),
            )
            for role in (
                "research_planner",
                "research_worker",
                "research_synthesizer",
            )
        }
    )


def _capturing_router(provider: CapturingResearchProvider) -> ModelRouter:
    profile = ResolvedModelProfile(
        role="research_synthesizer",
        profile_name="capturing",
        provider="echo",
        model="capturing-model",
    )
    return ModelRouter(
        {
            role: ModelRoute(
                role=role,
                profile_name="capturing",
                provider=provider,  # type: ignore[arg-type]
                profile=profile.model_copy(update={"role": role}),
            )
            for role in (
                "research_planner",
                "research_worker",
                "research_synthesizer",
            )
        }
    )


def _long_captured_report() -> str:
    body = (
        "The report uses the collected evidence while keeping the earlier session "
        "context as interpretive background. The source-backed finding remains tied "
        "to the persisted source label [R1]. "
    )
    return "\n\n".join(
        [
            "# Captured Research Report",
            "## Executive Summary\n" + body * 3,
            "## Methodology\n" + body * 2,
            "## Detailed Findings\n" + body * 3,
            "## Analysis\n" + body * 3,
            "## Caveats And Limitations\n" + body * 2,
            (
                "## Source Table\n| Label | Type | Title | URI |\n"
                "| --- | --- | --- | --- |\n"
                "| [R1] | repo | docs/example.md | docs/example.md |"
            ),
            "## Unresolved Questions\n" + body * 2,
        ]
    )


@pytest.mark.asyncio
async def test_searxng_provider_requests_json_and_normalizes_results() -> None:
    captured: dict[str, str] = {}

    def handler(request: httpx.Request) -> httpx.Response:
        captured["url"] = str(request.url)
        captured["user_agent"] = request.headers["user-agent"]
        return httpx.Response(
            200,
            json={
                "results": [
                    {
                        "title": "First result",
                        "url": "https://example.com/one",
                        "content": "First snippet",
                        "engine": "brave",
                        "category": "general",
                    },
                    {
                        "url": "https://example.com/two",
                        "engines": ["duckduckgo", "brave"],
                    },
                    {
                        "title": "Trimmed result",
                        "url": "https://example.com/three",
                        "content": "Trimmed",
                    },
                ]
            },
        )

    provider = SearxngSearchProvider(
        endpoint="https://search.example.test",
        user_agent="colossus-test/1.0",
        transport=httpx.MockTransport(handler),
    )

    drafts = await provider.collect("deep research", max_results=2)

    assert "https://search.example.test/search?" in captured["url"]
    assert "q=deep+research" in captured["url"]
    assert "format=json" in captured["url"]
    assert captured["user_agent"] == "colossus-test/1.0"
    assert len(drafts) == 2
    assert drafts[0].title == "First result"
    assert drafts[0].uri == "https://example.com/one"
    assert drafts[0].content == "First snippet"
    assert drafts[0].metadata == {
        "search_provider": "searxng",
        "engine": "brave",
        "category": "general",
    }
    assert drafts[1].title == "https://example.com/two"
    assert drafts[1].content == ""
    assert drafts[1].metadata == {
        "search_provider": "searxng",
        "engines": "duckduckgo,brave",
    }


@pytest.mark.asyncio
async def test_searxng_provider_supports_env_resolved_auth_without_leaking_secret() -> None:
    def handler(request: httpx.Request) -> httpx.Response:
        assert request.headers["x-searxng-key"] == "secret-token"
        return httpx.Response(
            200,
            json={
                "results": [
                    {
                        "title": "Authenticated result",
                        "url": "https://example.com/auth",
                        "content": "Authenticated snippet",
                    }
                ]
            },
        )

    provider = SearxngSearchProvider(
        endpoint="https://search.example.test/search",
        api_key="secret-token",
        auth_header="X-Searxng-Key",
        auth_scheme="raw",
        transport=httpx.MockTransport(handler),
    )

    drafts = await provider.collect("private query", max_results=5)

    assert drafts[0].uri == "https://example.com/auth"
    assert "secret-token" not in str(drafts[0].metadata)
    assert "secret-token" not in drafts[0].content


@pytest.mark.asyncio
async def test_searxng_provider_rejects_malformed_json_response() -> None:
    provider = SearxngSearchProvider(
        endpoint="https://search.example.test/search",
        transport=httpx.MockTransport(
            lambda _request: httpx.Response(200, content=b"not json")
        ),
    )

    with pytest.raises(ToolExecutionError, match="non-JSON"):
        await provider.collect("bad response", max_results=5)


def test_search_provider_factory_resolves_searxng_key_from_env(monkeypatch) -> None:
    captured: dict[str, object] = {}

    class CapturingSearxngProvider:
        def __init__(self, **kwargs: object) -> None:
            captured.update(kwargs)

        @property
        def configured(self) -> bool:
            return True

        async def collect(
            self,
            query: str,
            *,
            max_results: int,
        ) -> tuple[ResearchSourceDraft, ...]:
            del query, max_results
            return ()

    monkeypatch.setenv("SEARXNG_API_KEY", "secret-token")
    monkeypatch.setattr(container_module, "SearxngSearchProvider", CapturingSearxngProvider)

    provider = container_module.create_search_provider(
        SearchConfig(
            kind="searxng",
            endpoint="https://search.example.test",
            api_key_env="SEARXNG_API_KEY",
            auth_header="X-Searxng-Key",
            auth_scheme="raw",
        )
    )

    assert provider.configured is True
    assert captured == {
        "endpoint": "https://search.example.test",
        "user_agent": "colossus-agent/0.1",
        "api_key": "secret-token",
        "auth_header": "X-Searxng-Key",
        "auth_scheme": "raw",
        "http_client_config": None,
    }


@pytest.mark.asyncio
async def test_research_service_persists_cited_report_sources_and_claims(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = ResearchService(
        state,
        JsonlAuditSink(tmp_path / "audit.jsonl"),
        repo_provider=FakeRepoProvider(),
        run_id_factory=lambda: "research-1",
    )

    run = await service.run(
        question="How should deep research work?",
        session_id="session-1",
        source_kinds=("repo", "web", "mcp"),
        max_sources=5,
    )

    assert run.status == "completed"
    assert "[R1]" in run.report
    assert "# Research Report" in run.report
    assert "## Detailed Findings" in run.report
    assert "## Source Notes" in run.report
    assert "web search is not configured" in run.warnings
    assert "MCP research collection is not configured" in run.warnings
    assert await state.get_research_run("research-1") == run
    sources = await state.list_research_sources("research-1")
    claims = await state.list_research_claims("research-1")
    events = await state.list_events("research-1")
    messages = await state.list_messages("session-1")
    assert sources[0].label == "R1"
    assert claims[0].source_labels == ("R1",)
    assert any(event.type == "research.status" for event in events)
    assert [message.role for message in messages] == ["user", "assistant"]
    assert messages[0].content == "How should deep research work?"
    assert "# Research Report" in messages[1].content


@pytest.mark.asyncio
async def test_research_service_uses_prior_session_context_and_appends_report(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    await state.append_message(
        "session-1",
        "prior-run",
        UserMessage(content="Earlier we decided web.search should use self-hosted SearXNG."),
    )
    await state.append_message(
        "session-1",
        "prior-run",
        AssistantMessage(content="SearXNG keeps search provider details out of tool input."),
    )
    provider = CapturingResearchProvider()
    service = ResearchService(
        state,
        JsonlAuditSink(tmp_path / "audit.jsonl"),
        repo_provider=FakeRepoProvider(),
        model_router=_capturing_router(provider),
        run_id_factory=lambda: "research-context",
    )

    run = await service.run(
        question="What should we document next?",
        session_id="session-1",
        source_kinds=("repo",),
        max_sources=3,
    )

    planner_prompt = provider.prompts[0]
    synthesis_prompt = provider.prompts[-1]
    messages = await state.list_messages("session-1")
    assert "self-hosted SearXNG" in planner_prompt
    assert "keeps search provider details out of tool input" in synthesis_prompt
    assert [message.role for message in messages] == [
        "user",
        "assistant",
        "user",
        "assistant",
    ]
    assert messages[-2].content == "What should we document next?"
    assert messages[-1].content == run.report
    assert "# Captured Research Report" in messages[-1].content


@pytest.mark.asyncio
async def test_research_service_uses_approved_web_sources(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = ResearchService(
        state,
        JsonlAuditSink(tmp_path / "audit.jsonl"),
        repo_provider=FakeRepoProvider(),
        search_provider=FakeSearchProvider(),
        approval_handler=AllowAllApprovalHandler(),
        run_id_factory=lambda: "research-2",
    )

    run = await service.run(
        question="Need external corroboration",
        session_id="session-1",
        source_kinds=("web",),
        max_sources=3,
    )

    sources = await state.list_research_sources(run.id)
    assert sources[0].kind == "web"
    assert sources[0].uri == "https://example.com/research"
    assert "not approved" not in " ".join(run.warnings)


@pytest.mark.asyncio
async def test_research_service_rejects_tiny_synthesized_reports(tmp_path) -> None:
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = ResearchService(
        state,
        JsonlAuditSink(tmp_path / "audit.jsonl"),
        repo_provider=FakeRepoProvider(),
        model_router=_short_report_router(),
        run_id_factory=lambda: "research-3",
    )

    run = await service.run(
        question="How detailed should research mode be?",
        session_id="session-1",
        source_kinds=("repo",),
        max_sources=3,
    )

    assert "Too short [R1]" not in run.report
    assert "# Research Report" in run.report
    assert "## Executive Summary" in run.report
    assert "## Methodology" in run.report
    assert "## Detailed Findings" in run.report
    assert "## Analysis" in run.report
    assert "## Source Notes" in run.report


def test_research_agent_exposes_only_read_only_research_tools() -> None:
    agent = research_agent("model-a")

    assert "filesystem.read" in agent.tools
    assert "web.search" in agent.tools
    assert "mcp.call" in agent.tools
    assert "filesystem.write" not in agent.tools
    assert "patch.apply" not in agent.tools
    assert "shell.run" not in agent.tools
