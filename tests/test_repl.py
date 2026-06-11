import json

import pytest
from prompt_toolkit.completion import CompleteEvent
from prompt_toolkit.document import Document
from rich.console import Console

from colossus.adapters.echo_provider import EchoModelProvider
from colossus.application.defaults import default_agent
from colossus.application.model_router import ModelRoute, ModelRouter
from colossus.domain.context import ContextStatus
from colossus.domain.errors import ColossusError
from colossus.domain.models import ResolvedModelProfile
from colossus.domain.preferences import ReplPreferences
from colossus.domain.tasks import Task
from colossus.domain.tools import ToolPermission, ToolSpec
from colossus.interfaces.repl import (
    REPL_THEMES,
    ReplDisplayState,
    SlashCommandCompleter,
    _events_mode,
    _format_repl_toolbar,
    _format_run_toolbar,
    _format_slash_suggestions,
    _format_submit_summary,
    _multiline_mode,
    _preferences_from_state,
    _prompt_continuation,
    _prompt_message,
    _render_help,
    _render_model,
    _render_repl_preferences,
    _render_repl_startup,
    _render_status,
    _render_tasks,
    _render_theme_preview,
    _render_themes,
    _render_tools,
    _right_prompt,
    _show_submit_summary,
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
    event_matches = list(completer.get_completions(Document("/e"), event))
    argument_matches = list(completer.get_completions(Document("/events "), event))
    plain_matches = list(completer.get_completions(Document("hello"), event))

    assert {completion.text for completion in slash_matches} >= {"/events", "/exit"}
    assert [completion.text for completion in event_matches] == ["/events", "/exit"]
    assert argument_matches == []
    assert plain_matches == []


def test_slash_suggestions_show_in_toolbar_for_command_drafts() -> None:
    assert _format_slash_suggestions("/").startswith("commands: /model")
    assert _format_slash_suggestions("/e") == "commands: /events /exit"
    assert _format_slash_suggestions("/events ") == ""
    assert _format_slash_suggestions("hello") == ""
    assert _format_slash_suggestions("/wat") == "commands: no matches"


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
    status = parse_slash_command("/status")
    tasks = parse_slash_command("/tasks all")
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
    assert status is not None
    assert status.command == "status"
    assert tasks is not None
    assert tasks.command == "tasks"
    assert tasks.argument == "all"
    assert help_command is not None
    assert help_command.command == "help"


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
    assert "theme=hacker" in output
    assert "events=off" in output


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

    toolbar = _format_repl_toolbar(state, "hello\nthere", 2, 3)

    assert "mode=single" in toolbar
    assert "model=primary:very-long-local-model-nam..." in toolbar
    assert "theme=default" in toolbar
    assert "approval=risk-auto" in toolbar
    assert "stream=on" in toolbar
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
    assert "last=done:run-abcd" in toolbar

    state.last_status = "running"
    run_toolbar = _format_run_toolbar(state, "hello\nthere")
    assert "model=primary:very-long-local-model..." in run_toolbar
    assert "ctx=350/700(50%)" in run_toolbar
    assert "session=session-" in run_toolbar
    assert "tasks=2/5" in run_toolbar
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
    )

    _render_status(status_console, state)
    _render_help(help_console, state)

    status_output = status_console.export_text()
    help_output = help_console.export_text()
    assert "composer_mode" in status_output
    assert "multiline" in status_output
    assert "approval_mode" in status_output
    assert "theme" in status_output
    assert "activity_spinner" in status_output
    assert "transcript" in status_output
    assert "tasks" in status_output
    assert "/multiline on|off|toggle" in help_output
    assert "/transcript comfortable|compact" in help_output
    assert "/theme [NAME]" in help_output
    assert "/repl prefs|save|reset" in help_output
    assert "/tasks [open|all|STATUS]" in help_output
    assert "Current" in help_output
    assert "primary:model-a" in help_output
    assert "off" in help_output
    assert "verbose" in help_output
    assert "compact" in help_output
    assert "multiline" in help_output
    assert "hacker" in help_output
    assert "Esc+Enter" in help_output


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
