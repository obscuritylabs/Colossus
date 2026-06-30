import json
from pathlib import Path

import pytest
from prompt_toolkit.completion import CompleteEvent
from prompt_toolkit.document import Document
from rich.console import Console

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.echo_provider import EchoModelProvider
from colossus.adapters.skills_package import PackageSkillRepository
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.application.decisions import DecisionService
from colossus.application.defaults import default_agent
from colossus.application.memories import MemoryService
from colossus.application.model_router import ModelRoute, ModelRouter
from colossus.application.planning import PlanService
from colossus.application.sessions import SessionService
from colossus.application.skill_authoring import SkillAuthoringService
from colossus.application.skills import SkillResolver
from colossus.application.subagents import SubagentService
from colossus.domain.context import ContextStatus
from colossus.domain.decisions import KeyDecision
from colossus.domain.errors import ColossusError
from colossus.domain.memories import MemoryItem
from colossus.domain.messages import UserMessage
from colossus.domain.models import ResolvedModelProfile
from colossus.domain.plans import Plan
from colossus.domain.preferences import ReplPreferences
from colossus.domain.requests import AgentRunRequest, AgentRunResult
from colossus.domain.research import ResearchRun
from colossus.domain.sessions import SessionSummary
from colossus.domain.subagents import SubagentJob
from colossus.domain.tasks import Task
from colossus.domain.tools import ToolPermission, ToolSpec
from colossus.domain.user_prompts import UserPromptChoice
from colossus.interfaces.repl import (
    REPL_THEMES,
    ReplDisplayState,
    ReplInteractionMode,
    ReplWorkspaceServices,
    RichUserPromptHandler,
    SlashCommandCompleter,
    _composer_key_bindings,
    _events_mode,
    _format_repl_toolbar,
    _format_run_toolbar,
    _format_slash_suggestions,
    _format_submit_summary,
    _handle_agent_command,
    _handle_agents_command,
    _handle_decision_command,
    _handle_decisions_command,
    _handle_memories_command,
    _handle_memory_command,
    _handle_plan_command,
    _handle_research_command,
    _handle_resume_command,
    _handle_session_command,
    _handle_sessions_command,
    _handle_skill_command,
    _handle_workspace_command,
    _is_skill_mention_draft,
    _is_slash_command_draft,
    _match_user_prompt_answer,
    _multiline_mode,
    _parse_repl_integration_connect,
    _plan_agent,
    _preferences_from_state,
    _prompt_continuation,
    _prompt_for_plan_review,
    _prompt_message,
    _render_decisions,
    _render_help,
    _render_memories,
    _render_model,
    _render_plan,
    _render_plan_list,
    _render_repl_preferences,
    _render_repl_startup,
    _render_resumed_session,
    _render_status,
    _render_subagents,
    _render_tasks,
    _render_theme_preview,
    _render_themes,
    _render_tools,
    _resolve_workspace_argument,
    _right_prompt,
    _save_repl_plan,
    _show_submit_summary,
    _stream_mode_label,
    _stream_output_mode,
    _theme_by_name,
    _toggle_on_off,
    _trace_enabled,
    _trace_events_mode,
    _transcript_style_mode,
    load_user_repl_themes,
    parse_slash_command,
    repl_theme_names,
    validate_repl_theme,
)


class RecordingConsole(Console):
    def __init__(self) -> None:
        super().__init__(record=True, width=100)
        self.cleared = False

    def clear(self, *, home: bool = True) -> None:
        del home
        self.cleared = True


class FakeTraceRenderer:
    def __init__(self) -> None:
        self.rendered_model_output = False
        self.began = False
        self.ended = False
        self.final_answer = ""

    def begin_run(self, *, activity_context: str = "") -> None:
        del activity_context
        self.began = True

    def render_user_prompt(self, prompt: str) -> None:
        del prompt

    def end_run(self) -> None:
        self.ended = True

    def render_final_answer(self, output: str) -> None:
        self.final_answer = output


class FakePlanOrchestrator:
    def __init__(self) -> None:
        self.request: AgentRunRequest | None = None

    async def run(self, request: AgentRunRequest) -> AgentRunResult:
        self.request = request
        return AgentRunResult(
            run_id="run-plan",
            final_output="executed",
            events_recorded=0,
            session_id=request.session_id,
        )


class FakeResearchService:
    def __init__(self) -> None:
        self.question = ""

    async def run(self, *, question: str, session_id: str) -> ResearchRun:
        self.question = question
        return ResearchRun(
            id="research-1",
            session_id=session_id,
            question=question,
            status="completed",
            report="# Research Report\n\nFinding [R1]",
        )

    async def latest_run(self, session_id: str) -> ResearchRun | None:
        return ResearchRun(
            id="research-1",
            session_id=session_id,
            question="latest",
            status="completed",
            report="# Research Report\n\nLatest [R1]",
        )

    async def get_run(self, run_id: str) -> ResearchRun:
        return ResearchRun(
            id=run_id,
            session_id="session-research",
            question="shown",
            status="completed",
            report="# Research Report\n\nShown [R1]",
        )


class QueuedUserPromptHandler(RichUserPromptHandler):
    def __init__(self, console: Console, responses: list[str]) -> None:
        super().__init__(console)
        self._responses = responses

    def _ask_user(self, prompt: str) -> str:
        del prompt
        return self._responses.pop(0)


def test_parse_slash_command() -> None:
    parsed = parse_slash_command("/model gpt-5")

    assert parsed is not None
    assert parsed.command == "model"
    assert parsed.argument == "gpt-5"


def test_parse_non_command() -> None:
    assert parse_slash_command("hello") is None


def test_slash_command_completer_suggests_commands_while_typing() -> None:
    completer = SlashCommandCompleter()
    event = CompleteEvent(text_inserted=True)

    slash_matches = list(completer.get_completions(Document("/"), event))
    plan_matches = list(completer.get_completions(Document("/p"), event))
    plan_narrow_matches = list(completer.get_completions(Document("/pl"), event))
    resume_matches = list(completer.get_completions(Document("/res"), event))
    workspace_matches = list(completer.get_completions(Document("/w"), event))
    event_matches = list(completer.get_completions(Document("/e"), event))
    integration_matches = list(completer.get_completions(Document("/i"), event))
    argument_matches = list(completer.get_completions(Document("/events "), event))
    plain_matches = list(completer.get_completions(Document("hello"), event))

    assert {completion.text for completion in slash_matches} >= {"/events", "/exit"}
    assert [completion.text for completion in plan_matches] == ["/plan", "/packs"]
    assert [completion.text for completion in plan_narrow_matches] == ["/plan"]
    assert [completion.text for completion in resume_matches] == ["/resume", "/research"]
    assert [completion.text for completion in workspace_matches] == ["/workspace"]
    assert [completion.text for completion in event_matches] == ["/events", "/exit"]
    assert [completion.text for completion in integration_matches] == ["/integrations"]
    assert argument_matches == []
    assert plain_matches == []


def test_skill_completer_suggests_canonical_skill_mentions() -> None:
    completer = SlashCommandCompleter(SkillResolver((PackageSkillRepository(),)))
    event = CompleteEvent(text_inserted=True)

    at = list(completer.get_completions(Document("@"), event))
    namespace_prefix = list(completer.get_completions(Document("@s"), event))
    namespace_prefix_long = list(completer.get_completions(Document("@sk"), event))
    bare = list(completer.get_completions(Document("@skill"), event))
    canonical_empty = list(completer.get_completions(Document("@skill:"), event))
    canonical = list(completer.get_completions(Document("@skill:cod"), event))
    shorthand = list(completer.get_completions(Document("please @off"), event))

    all_skill_mentions = {
        "@skill:coding ",
        "@skill:security-review ",
        "@skill:offline-dev ",
        "@skill:skill-creator ",
    }
    assert {completion.text for completion in at} >= {"@skill:coding "}
    assert {completion.text for completion in namespace_prefix} >= all_skill_mentions
    assert {completion.text for completion in namespace_prefix_long} >= all_skill_mentions
    assert {completion.text for completion in bare} >= {"@skill:coding "}
    assert {completion.text for completion in canonical_empty} >= {"@skill:coding "}
    assert [completion.text for completion in canonical] == ["@skill:coding "]
    assert [completion.text for completion in shorthand] == ["@skill:offline-dev "]
    assert canonical[0].display_meta_text


def test_skill_completion_key_bindings_open_menu_while_typing() -> None:
    keys = {binding.keys for binding in _composer_key_bindings().bindings}

    assert ("@",) in keys
    assert ("a",) in keys
    assert (":",) in keys
    assert ("-",) in keys
    assert _is_skill_mention_draft("@")
    assert _is_skill_mention_draft("@skill")


def test_slash_suggestions_show_in_toolbar_for_command_drafts() -> None:
    assert _format_slash_suggestions("/").startswith("commands: /model")
    assert _format_slash_suggestions("/p") == "commands: /plan /packs"
    assert _format_slash_suggestions("/pl") == "commands: /plan"
    assert _format_slash_suggestions("/res") == "commands: /resume /research"
    assert _format_slash_suggestions("/w") == "commands: /workspace"
    assert _format_slash_suggestions("/e") == "commands: /events /exit"
    assert _format_slash_suggestions("/i") == "commands: /integrations"
    assert _format_slash_suggestions("/events ") == ""
    assert _format_slash_suggestions("hello") == ""
    assert _format_slash_suggestions("/wat") == "commands: no matches"


def test_slash_command_draft_detection() -> None:
    assert _is_slash_command_draft("/")
    assert _is_slash_command_draft("/pl")
    assert not _is_slash_command_draft("/plan approve")
    assert not _is_slash_command_draft("hello /")


def test_parse_context_commands() -> None:
    compact = parse_slash_command("/compact")
    context = parse_slash_command("/context restore snapshot-1")
    stream = parse_slash_command("/stream off")
    events = parse_slash_command("/events verbose")
    reasoning = parse_slash_command("/reasoning off")
    transcript = parse_slash_command("/transcript compact")
    multiline = parse_slash_command("/multiline on")
    theme = parse_slash_command("/theme carrot")
    repl = parse_slash_command("/repl prefs")
    workspace = parse_slash_command("/workspace ../other")
    status = parse_slash_command("/status")
    resume = parse_slash_command("/resume 5")
    session = parse_slash_command("/session resume session-1")
    sessions = parse_slash_command("/sessions")
    tasks = parse_slash_command("/tasks all")
    decision = parse_slash_command("/decision archive kd_1")
    decisions = parse_slash_command("/decisions all")
    memory = parse_slash_command("/memory search pytest")
    memories = parse_slash_command("/memories all")
    plan = parse_slash_command("/plan approve")
    research = parse_slash_command("/research show")
    integrations = parse_slash_command("/integrations list")
    skill = parse_slash_command("/skill use coding")
    help_command = parse_slash_command("/help")

    assert compact is not None
    assert compact.command == "compact"
    assert context is not None
    assert context.command == "context"
    assert context.argument == "restore snapshot-1"
    assert stream is not None
    assert stream.command == "stream"
    assert stream.argument == "off"
    assert events is not None
    assert events.command == "events"
    assert events.argument == "verbose"
    assert reasoning is not None
    assert reasoning.command == "reasoning"
    assert reasoning.argument == "off"
    assert transcript is not None
    assert transcript.command == "transcript"
    assert transcript.argument == "compact"
    assert multiline is not None
    assert multiline.command == "multiline"
    assert multiline.argument == "on"
    assert theme is not None
    assert theme.command == "theme"
    assert theme.argument == "carrot"
    assert repl is not None
    assert repl.command == "repl"
    assert repl.argument == "prefs"
    assert workspace is not None
    assert workspace.command == "workspace"
    assert workspace.argument == "../other"
    assert status is not None
    assert status.command == "status"
    assert resume is not None
    assert resume.command == "resume"
    assert resume.argument == "5"
    assert session is not None
    assert session.command == "session"
    assert session.argument == "resume session-1"
    assert sessions is not None
    assert sessions.command == "sessions"
    assert tasks is not None
    assert tasks.command == "tasks"
    assert tasks.argument == "all"
    assert decision is not None
    assert decision.command == "decision"
    assert decision.argument == "archive kd_1"
    assert decisions is not None
    assert decisions.command == "decisions"
    assert decisions.argument == "all"
    assert memory is not None
    assert memory.command == "memory"
    assert memory.argument == "search pytest"
    assert memories is not None
    assert memories.command == "memories"
    assert memories.argument == "all"
    assert plan is not None
    assert plan.command == "plan"
    assert plan.argument == "approve"
    assert research is not None
    assert research.command == "research"
    assert integrations is not None
    assert integrations.command == "integrations"
    assert integrations.argument == "list"
    assert research.argument == "show"
    assert skill is not None
    assert skill.command == "skill"
    assert skill.argument == "use coding"
    assert help_command is not None
    assert help_command.command == "help"


def test_parse_repl_integration_connect_supports_searxng_config() -> None:
    name, credential_ref, credential_refs, scopes, config = _parse_repl_integration_connect(
        [
            "connect",
            "searxng",
            "--base-url",
            "http://localhost:8888",
            "--credential-ref",
            "env:SEARXNG_API_KEY",
            "--auth-header",
            "X-Searxng-Key",
            "--auth-scheme",
            "raw",
        ]
    )

    assert name == "searxng"
    assert credential_ref == "env:SEARXNG_API_KEY"
    assert credential_refs == {}
    assert scopes == ()
    assert config == {
        "base_url": "http://localhost:8888",
        "auth_header": "X-Searxng-Key",
        "auth_scheme": "raw",
    }


def test_parse_repl_integration_connect_supports_opensearch_config() -> None:
    name, credential_ref, credential_refs, scopes, config = _parse_repl_integration_connect(
        [
            "connect",
            "opensearch",
            "--base-url",
            "http://localhost:9200",
            "--auth-type",
            "basic",
            "--username-ref",
            "env:OPENSEARCH_USER",
            "--password-ref",
            "env:OPENSEARCH_PASSWORD",
        ]
    )

    assert name == "opensearch"
    assert credential_ref is None
    assert credential_refs == {
        "username": "env:OPENSEARCH_USER",
        "password": "env:OPENSEARCH_PASSWORD",
    }
    assert scopes == ()
    assert config == {"base_url": "http://localhost:9200", "auth_type": "basic"}


def test_trace_enabled_parses_toggle_arguments() -> None:
    assert _trace_enabled("on", current=False) is True
    assert _trace_enabled("off", current=True) is False
    assert _trace_enabled("", current=True) is False


def test_stream_and_reasoning_toggles_share_on_off_parser() -> None:
    assert _toggle_on_off("on", current=False) is True
    assert _toggle_on_off("off", current=True) is False
    assert _toggle_on_off("", current=False) is True


def test_multiline_mode_parser() -> None:
    assert _multiline_mode("on", current=False) is True
    assert _multiline_mode("off", current=True) is False
    assert _multiline_mode("toggle", current=False) is True
    assert _multiline_mode("", current=True) is False


def test_stream_output_mode_parser() -> None:
    state = ReplDisplayState(
        session_id="session-123456",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
    )

    assert _stream_output_mode("on", state) == (True, False)
    assert _stream_output_mode("markdown", state) == (True, False)
    assert _stream_output_mode("buffered", state) == (True, False)
    assert _stream_output_mode("raw", state) == (True, True)
    assert _stream_output_mode("live", state) == (True, True)
    assert _stream_output_mode("off", state) == (False, False)
    assert _stream_mode_label(state) == "markdown"

    state.raw_stream_model_output = True
    assert _stream_mode_label(state) == "raw"
    state.stream_model_output = False
    assert _stream_mode_label(state) == "off"


def test_theme_lookup_and_names() -> None:
    assert repl_theme_names() == ("default", "mono", "high-contrast", "carrot", "hacker")
    assert _theme_by_name("default").name == "default"
    assert _theme_by_name("CARROT").name == "carrot"
    assert _theme_by_name("HACKER").name == "hacker"


def test_builtin_themes_define_required_keys() -> None:
    for theme in REPL_THEMES.values():
        validate_repl_theme(theme)
    assert {theme.transcript.activity_spinner for theme in REPL_THEMES.values()} == {
        "dots",
        "line",
        "arc",
        "bouncingBar",
        "aesthetic",
    }


def test_user_theme_files_load_and_reject_unknown_keys(tmp_path) -> None:
    theme_dir = tmp_path / "themes"
    theme_dir.mkdir()
    (theme_dir / "ocean.json").write_text(
        json.dumps(
            {
                "name": "ocean",
                "title": "colossus",
                "caret": ">",
                "styles": {"prompt.caret": "#00ffff bold"},
                "trace": {"tool_call": "bold cyan"},
                "transcript": {"tool": "bold cyan", "activity_spinner": "line"},
            }
        ),
        encoding="utf-8",
    )
    (theme_dir / "invalid.toml").write_text(
        'name = "invalid"\n[styles]\nunknown = "bold"\n',
        encoding="utf-8",
    )

    with pytest.raises(ColossusError, match="unsupported style keys"):
        load_user_repl_themes((theme_dir,))

    (theme_dir / "invalid.toml").unlink()
    themes = load_user_repl_themes((theme_dir,))
    assert themes["ocean"].styles["prompt.caret"] == "#00ffff bold"
    assert themes["ocean"].trace.tool_call == "bold cyan"
    assert themes["ocean"].transcript.tool == "bold cyan"
    assert themes["ocean"].transcript.activity_spinner == "line"


def test_user_theme_files_reject_invalid_activity_spinner(tmp_path) -> None:
    theme_dir = tmp_path / "themes"
    theme_dir.mkdir()
    (theme_dir / "bad-spinner.json").write_text(
        json.dumps(
            {
                "name": "bad-spinner",
                "transcript": {"activity_spinner": "matrix"},
            }
        ),
        encoding="utf-8",
    )

    with pytest.raises(ColossusError, match="activity spinner is invalid"):
        load_user_repl_themes((theme_dir,))


def test_events_mode_parser() -> None:
    assert _events_mode("compact") == "compact"
    assert _events_mode("verbose") == "verbose"
    assert _events_mode("off") == "off"
    assert _events_mode("unknown") == "compact"


def test_trace_alias_maps_to_event_modes() -> None:
    assert _trace_events_mode("on", current="off") == "compact"
    assert _trace_events_mode("verbose", current="compact") == "verbose"
    assert _trace_events_mode("off", current="compact") == "off"
    assert _trace_events_mode("", current="off") == "compact"
    assert _trace_events_mode("", current="compact") == "off"


def test_transcript_style_parser() -> None:
    assert _transcript_style_mode("comfortable", current="compact") == "comfortable"
    assert _transcript_style_mode("cards", current="compact") == "comfortable"
    assert _transcript_style_mode("compact", current="comfortable") == "compact"
    assert _transcript_style_mode("clean", current="comfortable") == "compact"
    assert _transcript_style_mode("", current="comfortable") == "compact"


def test_prompt_message_reflects_composer_mode_and_theme() -> None:
    state = ReplDisplayState(
        session_id="session-123456",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
    )

    prompt = _prompt_message(state)
    assert ("class:prompt.title", "colossus") in prompt
    assert ("class:prompt.badge", " SINGLE ") in prompt
    assert ("class:prompt.caret", "> ") in prompt
    state.multiline = True
    prompt = _prompt_message(state)
    assert ("class:prompt.badge", " MULTI ") in prompt
    state.interaction_mode = "plan"
    prompt = _prompt_message(state)
    assert ("class:prompt.badge", " PLAN ") in prompt


def test_repl_startup_clears_terminal_and_renders_banner() -> None:
    state = ReplDisplayState(
        session_id="session-123456",
        active_model_role="primary",
        model="model-a",
        approval_mode="risk-auto",
        theme=_theme_by_name("hacker"),
        events_mode="off",
    )
    console = RecordingConsole()

    _render_repl_startup(console, state)

    output = console.export_text()
    assert console.cleared is True
    assert "Colossus REPL" in output
    assert "session_id=session-123456" in output
    assert "mode=chat" in output
    assert "composer=single" in output
    assert "theme=hacker" in output
    assert "events=off" in output


def test_render_resumed_session_shows_compact_summary() -> None:
    console = Console(record=True, width=140)
    summary = SessionSummary(
        id="session-123456",
        title="Hello",
        created_at="2026-01-01",
        updated_at="2026-01-02",
        message_count=4,
        last_run_id="run-1",
        last_user_preview="continue this",
    )

    _render_resumed_session(console, summary)

    output = console.export_text()
    assert "Resumed session session-123456" in output
    assert "messages=4" in output
    assert "last_user=continue this" in output


def test_right_prompt_and_continuation_reflect_mode_and_theme() -> None:
    state = ReplDisplayState(
        session_id="session-123456",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
        theme=_theme_by_name("carrot"),
    )

    assert _right_prompt(state) == [("class:prompt.rprompt", "Enter sends")]
    state.multiline = True
    assert _right_prompt(state) == [("class:prompt.rprompt", "Esc+Enter sends")]
    assert _prompt_continuation(state, 4, 2, 0) == [("class:prompt.continuation", " | ")]


def test_toolbar_shows_operational_state_and_prompt_metrics() -> None:
    state = ReplDisplayState(
        session_id="session-123456",
        active_model_role="primary",
        model="very-long-local-model-name-for-toolbar",
        approval_mode="risk-auto",
        context_status=ContextStatus(
            session_id="session-123456",
            model="very-long-local-model-name-for-toolbar",
            message_count=3,
            token_estimate=350,
            raw_token_estimate=1200,
            context_window_tokens=1000,
            threshold_tokens=700,
            target_tokens=450,
            latest_snapshot_id="snapshot-abcdef",
            compacted=True,
            auto_compaction=True,
        ),
    )
    state.last_run_id = "run-abcdef"
    state.last_status = "done"
    state.task_summary = "tasks=2/5"
    state.interaction_mode = "plan"
    state.active_plan_id = "plan-abcdef"
    state.active_plan_status = "draft"

    toolbar = _format_repl_toolbar(state, "hello\nthere", 2, 3)

    assert "mode=plan" in toolbar
    assert "model=primary:very-long-local-model-nam..." in toolbar
    assert "theme=default" in toolbar
    assert "approval=risk-auto" in toolbar
    assert "stream=markdown" in toolbar
    assert "events=compact" in toolbar
    assert "transcript=comfortable" in toolbar
    assert "reasoning=on" in toolbar
    assert "session=session-" in toolbar
    assert "pos=2:3" in toolbar
    assert "chars=11" in toolbar
    assert "lines=2" in toolbar
    assert "ctx=350/700(50%)" in toolbar
    assert "raw=1200" in toolbar
    assert "msgs=3" in toolbar
    assert "snap=snapshot" in toolbar
    assert "tasks=2/5" in toolbar
    assert "plan=draft:plan-ab" in toolbar
    assert "last=done:run-abcd" in toolbar

    state.last_status = "running"
    run_toolbar = _format_run_toolbar(state, "hello\nthere")
    assert "model=primary:very-long-local-model..." in run_toolbar
    assert "ctx=350/700(50%)" in run_toolbar
    assert "session=session-" in run_toolbar
    assert "tasks=2/5" in run_toolbar
    assert "plan=draft:plan-ab" in run_toolbar
    assert "chars=11" in run_toolbar
    assert "lines=2" in run_toolbar
    assert "last=running" not in run_toolbar


def test_preferences_from_state_captures_repl_choices() -> None:
    state = ReplDisplayState(
        session_id="session-123456",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
        theme=_theme_by_name("carrot"),
        multiline=True,
        stream_model_output=False,
        events_mode="verbose",
        show_reasoning=False,
        transcript_style="compact",
    )

    assert _preferences_from_state(state) == ReplPreferences(
        theme="carrot",
        multiline=True,
        stream_model_output=False,
        events_mode="verbose",
        show_reasoning=False,
        transcript_style="compact",
    )


def test_submit_summary_includes_prompt_and_context_metrics() -> None:
    state = ReplDisplayState(
        session_id="session-123456",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
        context_status=ContextStatus(
            session_id="session-123456",
            model="model-a",
            message_count=2,
            token_estimate=100,
            context_window_tokens=1000,
            threshold_tokens=700,
            target_tokens=450,
        ),
    )

    summary = _format_submit_summary(state, "one\ntwo")

    assert "submit chars=7 lines=2" in summary
    assert "model=primary:model-a" in summary
    assert "session=session-" in summary
    assert "ctx=100/700(14%)" in summary


def test_submit_summary_only_shows_in_verbose_events_mode() -> None:
    state = ReplDisplayState(
        session_id="session-123456",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
    )

    assert _show_submit_summary(state) is False
    state.events_mode = "off"
    assert _show_submit_summary(state) is False
    state.events_mode = "verbose"
    assert _show_submit_summary(state) is True


def test_render_status_and_help_show_composer_details() -> None:
    status_console = Console(record=True, width=140)
    help_console = Console(record=True, width=140)
    state = ReplDisplayState(
        session_id="session-123456",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
        theme=_theme_by_name("hacker"),
        stream_model_output=False,
        events_mode="verbose",
        show_reasoning=False,
        transcript_style="compact",
        multiline=True,
        interaction_mode="plan",
        active_plan_id="plan-123456",
        active_plan_status="approved",
    )

    _render_status(status_console, state)
    _render_help(help_console, state)

    status_output = status_console.export_text()
    help_output = help_console.export_text()
    assert "composer_mode" in status_output
    assert "multiline" in status_output
    assert "mode" in status_output
    assert "skill_mode" in status_output
    assert "sticky_skills" in status_output
    assert "plan" in status_output
    assert "active_plan" in status_output
    assert "plan-123456" in status_output
    assert "approval_mode" in status_output
    assert "theme" in status_output
    assert "activity_spinner" in status_output
    assert "transcript" in status_output
    assert "tasks" in status_output
    assert "workspace" in status_output
    assert "/multiline on|off|toggle" in help_output
    assert "/transcript comfortable|compact" in help_output
    assert "/theme [NAME]" in help_output
    assert "/repl prefs|save|reset" in help_output
    assert "/workspace [PATH]" in help_output
    assert "/tasks [open|all|STATUS]" in help_output
    assert "/decisions [all|STATUS]" in help_output
    assert "/decision [archive|supersede|TEXT]" in help_output
    assert "/memories [all|STATUS]" in help_output
    assert "/memory [archive|search|supersede|TEXT]" in help_output
    assert "/plan [on|off|show|approve|execute|list|discard]" in help_output
    assert "/skill [on|off|show|use|drop|clear|new|validate]" in help_output
    assert "Current" in help_output
    assert "primary:model-a" in help_output
    assert "off" in help_output
    assert "verbose" in help_output
    assert "compact" in help_output
    assert "multiline" in help_output
    assert "hacker" in help_output
    assert "approved:plan-12" in help_output
    assert "Esc+Enter" in help_output


def test_workspace_command_shows_and_switches_root(tmp_path: Path) -> None:
    current = tmp_path / "current"
    next_root = tmp_path / "next"
    current.mkdir()
    next_root.mkdir()
    state = ReplDisplayState(
        session_id="session-workspace",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
        workspace_root=current.resolve(),
    )
    console = Console(record=True, width=140)
    captured: dict[str, Path] = {}

    def workspace_factory(root: Path, model_role: str) -> ReplWorkspaceServices:
        assert model_role == "primary"
        captured["root"] = root
        return ReplWorkspaceServices(
            workspace_root=root,
            orchestrator=FakePlanOrchestrator(),  # type: ignore[arg-type]
            context_service=None,
            research_service=None,
        )

    _handle_workspace_command(console, state, "show", workspace_factory, "primary")
    services = _handle_workspace_command(console, state, "../next", workspace_factory, "primary")

    assert services is not None
    assert captured["root"] == next_root.resolve()
    assert state.workspace_root == next_root.resolve()
    output = console.export_text()
    assert str(current.resolve()) in output
    assert str(next_root.resolve()) in output


def test_resolve_workspace_argument_rejects_missing_path(tmp_path: Path) -> None:
    with pytest.raises(ColossusError, match="Workspace does not exist"):
        _resolve_workspace_argument("missing", tmp_path)


def test_render_tasks_shows_session_task_rows() -> None:
    console = Console(record=True, width=140)
    tasks = (
        Task(
            id="task-1",
            session_id="session-1",
            title="Persist tasks",
            description="Make tasks visible in the REPL",
            status="in_progress",
            created_at="2026-06-10T00:00:00+00:00",
            updated_at="2026-06-10T00:00:00+00:00",
        ),
        Task(
            id="task-2",
            session_id="session-1",
            title="Ship",
            status="completed",
            created_at="2026-06-10T00:00:00+00:00",
            updated_at="2026-06-10T00:00:00+00:00",
        ),
    )

    _render_tasks(console, tasks)

    output = console.export_text()
    assert "in_progress" in output
    assert "[~]" in output
    assert "completed" in output
    assert "[x]" in output
    assert "Persist tasks" in output


def test_render_decisions_shows_key_decision_rows() -> None:
    console = Console(record=True, width=140)
    decisions = (
        KeyDecision(
            id="kd_1",
            session_id="session-1",
            source="agent",
            priority="critical",
            title="Durable commitments",
            decision="Key decisions are durable commitments, not memories.",
        ),
    )

    _render_decisions(console, decisions)

    output = console.export_text()
    assert "critical" in output
    assert "kd_1" in output
    assert "durable commitments" in output


def test_render_memories_shows_memory_rows() -> None:
    console = Console(record=True, width=140)
    memories = (
        MemoryItem(
            id="mem_1",
            scope="repo",
            kind="preference",
            source="user",
            text="Run pytest before declaring completion.",
            repo_root="/repo",
        ),
    )

    _render_memories(console, memories)

    output = console.export_text()
    assert "repo" in output
    assert "preference" in output
    assert "mem_1" in output
    assert "Run pytest" in output


@pytest.mark.asyncio
async def test_handle_decision_commands_manage_lifecycle(tmp_path) -> None:
    console = Console(record=True, width=140)
    service = DecisionService(
        SQLiteStateStore(tmp_path / "state.sqlite3"),
        JsonlAuditSink(tmp_path / "audit.jsonl"),
    )

    await _handle_decision_command(
        console,
        service,
        "session-1",
        "Key decisions are durable commitments, not memories.",
    )
    created = (await service.list_decisions(session_id="session-1"))[0]
    await _handle_decision_command(
        console,
        service,
        "session-1",
        f"supersede {created.id} Active key decisions are injected before snapshots.",
    )
    replacement = (await service.list_decisions(session_id="session-1"))[0]
    await _handle_decision_command(console, service, "session-1", f"archive {replacement.id}")
    await _handle_decisions_command(console, service, "session-1", "all")

    decisions = await service.list_decisions(session_id="session-1", status=None)
    output = console.export_text()
    assert {decision.status for decision in decisions} == {"superseded", "archived"}
    assert "Created decision" in output
    assert "Superseded with decision" in output
    assert "Archived decision" in output
    assert replacement.id in output


@pytest.mark.asyncio
async def test_handle_memory_commands_manage_lifecycle(tmp_path) -> None:
    console = Console(record=True, width=140)
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = MemoryService(state, JsonlAuditSink(tmp_path / "audit.jsonl"), state)
    repo_root = tmp_path / "repo"

    await _handle_memory_command(
        console,
        service,
        "session-1",
        repo_root,
        "Run pytest before declaring completion.",
    )
    created = (await service.list_memories(repo_root=str(repo_root)))[0]
    await _handle_memory_command(console, service, "session-1", repo_root, "search pytest")
    await _handle_memory_command(
        console,
        service,
        "session-1",
        repo_root,
        f"supersede {created.id} Run pytest and ruff before declaring completion.",
    )
    replacement = (await service.list_memories(repo_root=str(repo_root)))[0]
    await _handle_memory_command(
        console,
        service,
        "session-1",
        repo_root,
        f"archive {replacement.id}",
    )
    await _handle_memories_command(console, service, "session-1", repo_root, "all")

    memories = await service.list_memories(status=None)
    output = console.export_text()
    assert {memory.status for memory in memories} == {"superseded", "archived"}
    assert "Saved memory" in output
    assert "Superseded with memory" in output
    assert "Archived memory" in output
    assert replacement.id in output


@pytest.mark.asyncio
async def test_handle_agents_command_lists_session_jobs(tmp_path) -> None:
    console = Console(record=True, width=140)
    state = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = SubagentService(state, JsonlAuditSink(tmp_path / "audit.jsonl"))
    await state.save_subagent_job(
        SubagentJob(
            id="agent-1",
            session_id="session-1",
            parent_run_id="run-1",
            parent_call_id="call-1",
            task="Review docs",
            child_session_id="session-1:subagent:agent-1",
        )
    )

    await _handle_agents_command(console, service, "session-1", "")

    output = console.export_text()
    assert "agent-1" in output
    assert "Review docs" in output


@pytest.mark.asyncio
async def test_handle_session_commands_list_resume_and_start_new_session(tmp_path) -> None:
    console = Console(record=True, width=140)
    state_store = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = SessionService(state_store, session_id_factory=lambda: "session-fresh")
    state = ReplDisplayState(
        session_id="session-current",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
        last_run_id="run-old",
        last_status="done",
    )
    await state_store.append_message("session-new", "run-new", UserMessage(content="resume me"))

    await _handle_sessions_command(console, service, "")
    await _handle_session_command(console, service, state, "latest", None, None)
    assert state.session_id == "session-new"
    assert state.last_run_id is None
    assert state.last_status == "idle"

    await _handle_session_command(console, service, state, "new", None, None)
    assert state.session_id == "session-fresh"

    output = console.export_text()
    assert "session-new" in output
    assert "resume me" in output
    assert "Started new session session-fresh" in output


@pytest.mark.asyncio
async def test_handle_resume_command_prompts_for_session_choice(tmp_path) -> None:
    console = Console(record=True, width=140)
    state_store = SQLiteStateStore(tmp_path / "state.sqlite3")
    service = SessionService(state_store)
    state = ReplDisplayState(
        session_id="session-current",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
    )
    await state_store.append_message("session-choice", "run-1", UserMessage(content="pick me"))

    await _handle_resume_command(
        console,
        service,
        state,
        "",
        None,
        None,
        QueuedUserPromptHandler(console, ["1"]),
    )

    output = console.export_text()
    assert state.session_id == "session-choice"
    assert "Choose a session to resume." in output
    assert "session-choice" in output
    assert "pick me" in output
    assert "Resumed session session-choice" in output


def test_render_subagents_shows_status_markers() -> None:
    console = Console(record=True, width=140)
    jobs = (
        SubagentJob(
            id="agent-1",
            session_id="session-1",
            parent_run_id="run-1",
            parent_call_id="call-1",
            task="Review docs",
            status="running",
            child_session_id="session-1:subagent:agent-1",
        ),
    )

    _render_subagents(console, jobs)

    output = console.export_text()
    assert "agent-1" in output
    assert "[~]" in output


def test_match_user_prompt_answer_accepts_choice_number_or_id_or_freeform() -> None:
    choices = (
        UserPromptChoice(id="minimal", label="Minimal", description="Smallest change"),
        UserPromptChoice(id="full", label="Full", description="Complete workflow"),
    )

    by_number = _match_user_prompt_answer("2", choices, allow_freeform=False)
    by_id = _match_user_prompt_answer("minimal", choices, allow_freeform=False)
    freeform = _match_user_prompt_answer("something else", choices, allow_freeform=True)

    assert by_number is not None
    assert by_number.answer == "Full"
    assert by_number.choice_id == "full"
    assert by_id is not None
    assert by_id.answer == "Minimal"
    assert by_id.choice_id == "minimal"
    assert freeform is not None
    assert freeform.answer == "something else"
    assert freeform.choice_id is None
    assert _match_user_prompt_answer("", choices, allow_freeform=True) is None
    assert _match_user_prompt_answer("other", choices, allow_freeform=False) is None


@pytest.mark.asyncio
async def test_rich_user_prompt_handler_renders_choices_and_returns_selection() -> None:
    console = Console(record=True, width=120)
    handler = QueuedUserPromptHandler(console, ["2"])

    answer = await handler.ask(
        question="Which path?",
        choices=(
            UserPromptChoice(id="minimal", label="Minimal", description="Smallest change"),
            UserPromptChoice(id="full", label="Full", description="Complete workflow"),
        ),
        allow_freeform=False,
    )

    output = console.export_text()
    assert answer.answer == "Full"
    assert answer.choice_id == "full"
    assert "Which path?" in output
    assert "Minimal" in output
    assert "Complete workflow" in output
    assert "[bold cyan]" not in output


@pytest.mark.asyncio
async def test_rich_user_prompt_handler_retries_invalid_choice() -> None:
    console = Console(record=True, width=120)
    handler = QueuedUserPromptHandler(console, ["not-valid", "minimal"])

    answer = await handler.ask(
        question="Which path?",
        choices=(UserPromptChoice(id="minimal", label="Minimal"),),
        allow_freeform=False,
    )

    assert answer.choice_id == "minimal"
    assert "Please choose one of the listed options." in console.export_text()


def test_plan_agent_adds_plan_only_instructions() -> None:
    agent = default_agent("model-a")

    planned = _plan_agent(agent)

    assert planned.model == agent.model
    assert "REPL Plan Mode" in planned.instructions
    assert "do not make code changes" in planned.instructions
    assert "task.create" in planned.instructions
    assert "clear trackable work" in planned.instructions


def test_handle_agent_command_sets_max_turns() -> None:
    console = Console(record=True, width=100)
    agent = default_agent("model-a")

    updated = _handle_agent_command(console, agent, "max-turns 40")
    invalid = _handle_agent_command(console, updated or agent, "max-turns 101")

    assert updated is not None
    assert updated.max_turns == 40
    assert updated.model == "model-a"
    assert invalid is None
    output = console.export_text()
    assert "Agent max turns set to 40." in output
    assert "between 1 and 100" in output


def test_handle_skill_command_manages_runtime_skill_mode() -> None:
    console = Console(record=True, width=140)
    resolver = SkillResolver((PackageSkillRepository(),))
    state = ReplDisplayState(
        session_id="session-1",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
    )
    agent = default_agent("model-a")

    _handle_skill_command(console, resolver, state, agent, "use coding")
    _handle_skill_command(console, resolver, state, agent, "show")
    _handle_skill_command(console, resolver, state, agent, "show coding")
    _handle_skill_command(console, resolver, state, agent, "off")
    _handle_skill_command(console, resolver, state, agent, "drop coding")
    _handle_skill_command(console, resolver, state, agent, "clear")

    output = console.export_text()
    assert state.skill_mode_enabled is False
    assert state.sticky_skills == ()
    assert "Sticky skill added: coding" in output
    assert "available_count" in output
    assert "General software implementation workflow" in output
    assert "Skill Mode is off." in output


def test_handle_skill_command_scaffolds_and_validates_skills(tmp_path) -> None:
    console = Console(record=True, width=140)
    resolver = SkillResolver((PackageSkillRepository(),))
    state = ReplDisplayState(
        session_id="session-1",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
    )
    agent = default_agent("model-a")
    service = SkillAuthoringService(tmp_path / "skills")

    _handle_skill_command(
        console,
        resolver,
        state,
        agent,
        (
            f"new demo-skill --pack {tmp_path / 'pack'} "
            "--resources references,scripts --agent-compatible"
        ),
        skill_authoring_service=service,
    )
    skill_dir = tmp_path / "pack" / "skills" / "demo-skill"
    _handle_skill_command(
        console,
        resolver,
        state,
        agent,
        f"validate {skill_dir}",
        skill_authoring_service=service,
    )

    output = console.export_text()
    assert (skill_dir / "manifest.json").is_file()
    assert (skill_dir / "references").is_dir()
    assert (skill_dir / "scripts").is_dir()
    assert (skill_dir / "SKILL.md").read_text(encoding="utf-8").startswith(
        "---\nname: demo-skill\n"
    )
    assert "Wrote skill demo-skill" in output
    assert "Skill is valid: demo-skill" in output


def test_render_plan_prefers_markdown_content() -> None:
    console = Console(record=True, width=120)
    plan = Plan(
        id="plan-1",
        session_id="session-1",
        prompt="ship it",
        content="# Ship It\n\n- Inspect\n- Implement",
    )

    _render_plan(console, plan)

    output = console.export_text()
    assert "Plan: plan-1" in output
    assert "Ship It" in output
    assert "Inspect" in output
    assert "Step" not in output


def test_render_plan_list_marks_active_plan() -> None:
    console = Console(record=True, width=120)
    plans = (
        Plan(id="plan-1", session_id="session-1", prompt="first"),
        Plan(id="plan-2", session_id="session-1", prompt="second"),
    )

    _render_plan_list(console, plans, active_plan_id="plan-2")

    output = console.export_text()
    assert "plan-1" in output
    assert "plan-2" in output
    assert "*" in output


@pytest.mark.asyncio
async def test_save_repl_plan_creates_and_replaces_active_draft(tmp_path) -> None:
    service = PlanService(
        SQLiteStateStore(tmp_path / "state.sqlite3"),
        JsonlAuditSink(tmp_path / "audit.jsonl"),
    )
    state = ReplDisplayState(
        session_id="session-plan",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
    )

    first = await _save_repl_plan(service, state, prompt="build it", content="# Plan A")
    second = await _save_repl_plan(service, state, prompt="build it better", content="# Plan B")

    assert first.id == second.id
    assert state.active_plan_id == first.id
    assert state.active_plan_status == "draft"
    assert (await service.get_plan(first.id)).prompt == "build it better"
    assert (await service.get_plan(first.id)).content == "# Plan B"


@pytest.mark.asyncio
async def test_plan_command_toggles_lists_shows_approves_and_discards(tmp_path) -> None:
    service = PlanService(
        SQLiteStateStore(tmp_path / "state.sqlite3"),
        JsonlAuditSink(tmp_path / "audit.jsonl"),
    )
    state = ReplDisplayState(
        session_id="session-plan",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
    )
    console = Console(record=True, width=140)
    orchestrator = FakePlanOrchestrator()
    trace = FakeTraceRenderer()

    await _handle_plan_command(
        console,
        service,
        state,
        "",
        orchestrator,  # type: ignore[arg-type]
        default_agent("model-a"),
        trace,  # type: ignore[arg-type]
    )
    plan = await _save_repl_plan(service, state, prompt="ship it", content="# Ship")
    await _handle_plan_command(
        console,
        service,
        state,
        "list",
        orchestrator,  # type: ignore[arg-type]
        default_agent("model-a"),
        trace,  # type: ignore[arg-type]
    )
    await _handle_plan_command(
        console,
        service,
        state,
        "show",
        orchestrator,  # type: ignore[arg-type]
        default_agent("model-a"),
        trace,  # type: ignore[arg-type]
    )
    await _handle_plan_command(
        console,
        service,
        state,
        "approve",
        orchestrator,  # type: ignore[arg-type]
        default_agent("model-a"),
        trace,  # type: ignore[arg-type]
    )
    await _handle_plan_command(
        console,
        service,
        state,
        "discard",
        orchestrator,  # type: ignore[arg-type]
        default_agent("model-a"),
        trace,  # type: ignore[arg-type]
    )

    output = console.export_text()
    assert state.interaction_mode == "plan"
    assert state.active_plan_id is None
    assert state.active_plan_status is None
    assert f"Approved plan {plan.id}" in output
    assert "Cleared active plan." in output


@pytest.mark.asyncio
async def test_plan_execute_refuses_draft_and_executed_plans(tmp_path) -> None:
    service = PlanService(
        SQLiteStateStore(tmp_path / "state.sqlite3"),
        JsonlAuditSink(tmp_path / "audit.jsonl"),
    )
    state = ReplDisplayState(
        session_id="session-plan",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
    )
    console = Console(record=True, width=140)
    orchestrator = FakePlanOrchestrator()
    trace = FakeTraceRenderer()
    plan = await _save_repl_plan(service, state, prompt="ship it", content="# Ship")

    await _handle_plan_command(
        console,
        service,
        state,
        "execute",
        orchestrator,  # type: ignore[arg-type]
        default_agent("model-a"),
        trace,  # type: ignore[arg-type]
    )
    approved = await service.approve_plan(plan.id)
    await service.mark_executed(approved.id, "run-old")
    await _handle_plan_command(
        console,
        service,
        state,
        "execute",
        orchestrator,  # type: ignore[arg-type]
        default_agent("model-a"),
        trace,  # type: ignore[arg-type]
    )

    output = console.export_text()
    assert "Approve first with /plan approve." in output
    assert "Plan has already been executed." in output
    assert orchestrator.request is None


@pytest.mark.asyncio
async def test_plan_execute_runs_approved_active_plan(tmp_path) -> None:
    service = PlanService(
        SQLiteStateStore(tmp_path / "state.sqlite3"),
        JsonlAuditSink(tmp_path / "audit.jsonl"),
    )
    state = ReplDisplayState(
        session_id="session-plan",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
        interaction_mode="plan",
    )
    console = Console(record=True, width=140)
    orchestrator = FakePlanOrchestrator()
    trace = FakeTraceRenderer()
    plan = await _save_repl_plan(service, state, prompt="ship it", content="# Ship")
    await service.approve_plan(plan.id)

    await _handle_plan_command(
        console,
        service,
        state,
        "execute",
        orchestrator,  # type: ignore[arg-type]
        default_agent("model-a"),
        trace,  # type: ignore[arg-type]
    )

    assert orchestrator.request is not None
    assert orchestrator.request.plan_id == plan.id
    assert "Execute the approved plan." in orchestrator.request.prompt
    assert "# Ship" in orchestrator.request.prompt
    assert (await service.get_plan(plan.id)).status == "executed"
    assert state.active_plan_status == "executed"
    assert state.interaction_mode == "chat"
    assert trace.began is True
    assert trace.ended is True
    assert trace.final_answer == "executed"
    assert f"Executed plan {plan.id}." in console.export_text()


@pytest.mark.asyncio
async def test_research_command_toggles_and_runs_query() -> None:
    state = ReplDisplayState(
        session_id="session-research",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
    )
    console = Console(record=True, width=140)
    service = FakeResearchService()
    trace = FakeTraceRenderer()

    await _handle_research_command(
        console,
        service,  # type: ignore[arg-type]
        state,
        "on",
        trace,  # type: ignore[arg-type]
    )
    await _handle_research_command(
        console,
        service,  # type: ignore[arg-type]
        state,
        "What is stable?",
        trace,  # type: ignore[arg-type]
    )

    assert state.interaction_mode == "research"
    assert state.active_research_id == "research-1"
    assert state.active_research_status == "completed"
    assert service.question == "What is stable?"
    assert trace.final_answer == "# Research Report\n\nFinding [R1]"


@pytest.mark.asyncio
async def test_plan_review_prompt_approves_and_executes(tmp_path, monkeypatch) -> None:
    service = PlanService(
        SQLiteStateStore(tmp_path / "state.sqlite3"),
        JsonlAuditSink(tmp_path / "audit.jsonl"),
    )
    state = ReplDisplayState(
        session_id="session-plan",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
        interaction_mode="plan",
    )
    console = Console(record=True, width=140)
    orchestrator = FakePlanOrchestrator()
    trace = FakeTraceRenderer()
    plan = await _save_repl_plan(service, state, prompt="ship it", content="# Ship")
    monkeypatch.setattr(
        "colossus.interfaces.repl.RichUserPromptHandler",
        lambda console: QueuedUserPromptHandler(console, ["1"]),
    )

    await _prompt_for_plan_review(
        console,
        service,
        state,
        plan,
        orchestrator,  # type: ignore[arg-type]
        default_agent("model-a"),
        trace,  # type: ignore[arg-type]
    )

    assert orchestrator.request is not None
    assert orchestrator.request.plan_id == plan.id
    assert (await service.get_plan(plan.id)).status == "executed"
    assert state.active_plan_status == "executed"
    assert state.interaction_mode == "chat"
    output = console.export_text()
    assert "Approve this plan?" in output
    assert "Approved plan" in output
    assert "Executed plan" in output


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("choice", "expected_mode", "expected_active", "expected_text"),
    (
        ("2", "chat", True, "Kept draft plan."),
        ("3", "plan", True, "Plan Mode is still on."),
        ("4", "chat", False, "Discarded active plan."),
    ),
)
async def test_plan_review_prompt_handles_non_execute_choices(
    tmp_path,
    monkeypatch,
    choice: str,
    expected_mode: ReplInteractionMode,
    expected_active: bool,
    expected_text: str,
) -> None:
    service = PlanService(
        SQLiteStateStore(tmp_path / "state.sqlite3"),
        JsonlAuditSink(tmp_path / "audit.jsonl"),
    )
    state = ReplDisplayState(
        session_id="session-plan",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
        interaction_mode="plan",
    )
    console = Console(record=True, width=140)
    orchestrator = FakePlanOrchestrator()
    trace = FakeTraceRenderer()
    plan = await _save_repl_plan(service, state, prompt="ship it", content="# Ship")
    monkeypatch.setattr(
        "colossus.interfaces.repl.RichUserPromptHandler",
        lambda console: QueuedUserPromptHandler(console, [choice]),
    )

    await _prompt_for_plan_review(
        console,
        service,
        state,
        plan,
        orchestrator,  # type: ignore[arg-type]
        default_agent("model-a"),
        trace,  # type: ignore[arg-type]
    )

    assert orchestrator.request is None
    assert state.interaction_mode == expected_mode
    assert (state.active_plan_id is not None) is expected_active
    assert expected_text in console.export_text()
    assert (await service.get_plan(plan.id)).status == "draft"


def test_render_themes_lists_available_theme_pack() -> None:
    console = Console(record=True, width=140)

    _render_themes(console, _theme_by_name("carrot"))

    output = console.export_text()
    assert "default" in output
    assert "mono" in output
    assert "high-contrast" in output
    assert "carrot" in output
    assert "hacker" in output
    assert "bouncingBar" in output
    assert "aesthetic" in output
    assert "yes" in output


def test_render_theme_preview_and_repl_preferences() -> None:
    preview_console = Console(record=True, width=140)
    prefs_console = Console(record=True, width=140)
    state = ReplDisplayState(
        session_id="session-123456",
        active_model_role="primary",
        model="model-a",
        approval_mode="ask",
        theme=_theme_by_name("mono"),
        saved_preferences=ReplPreferences(theme="carrot"),
    )

    _render_theme_preview(preview_console, REPL_THEMES, ("default", "mono"))
    _render_repl_preferences(prefs_console, state)

    preview_output = preview_console.export_text()
    prefs_output = prefs_console.export_text()
    assert "default" in preview_output
    assert "thinking" in preview_output
    assert "tool call" in preview_output
    assert "risk assessment" in preview_output
    assert "done" in preview_output
    assert "dots" in preview_output
    assert "line" in preview_output
    assert "you agent tool" in preview_output
    assert "Preference" in prefs_output
    assert "mono" in prefs_output
    assert "carrot" in prefs_output
    assert "transcript_style" in prefs_output


def test_render_tools_lists_permission_summary() -> None:
    console = Console(record=True, width=120)
    spec = ToolSpec(
        name="filesystem.write",
        description="Write a file.",
        input_schema={"type": "object"},
        permissions=ToolPermission(
            filesystem="write",
            network="deny",
            approval_required=True,
            mutation=True,
            risk="high",
        ),
    )

    _render_tools(console, (spec,))

    output = console.export_text()
    assert "filesystem.write" in output
    assert "Write a file." in output
    assert "write" in output
    assert "True" in output
    assert "high" in output


def test_render_model_shows_active_role_and_profile() -> None:
    console = Console(record=True, width=120)
    router = ModelRouter(
        {
            "risk_evaluator": ModelRoute(
                role="risk_evaluator",
                profile_name="risk",
                provider=EchoModelProvider(),
                profile=ResolvedModelProfile(
                    role="risk_evaluator",
                    profile_name="risk",
                    provider="echo",
                    model="risk-model",
                ),
            )
        }
    )

    _render_model(console, default_agent("risk-model"), "risk_evaluator", router)

    output = console.export_text()
    assert "risk_evaluator" in output
    assert "risk" in output
    assert "risk-model" in output
