"""Interactive REPL surface."""

import asyncio
import json
import tomllib
from collections.abc import Callable, Iterable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Literal
from uuid import uuid4

from prompt_toolkit import PromptSession
from prompt_toolkit.application.current import get_app_or_none
from prompt_toolkit.completion import CompleteEvent, Completer, Completion
from prompt_toolkit.document import Document
from prompt_toolkit.filters import Condition
from prompt_toolkit.formatted_text import AnyFormattedText
from prompt_toolkit.history import FileHistory
from prompt_toolkit.key_binding import KeyBindings
from prompt_toolkit.styles import Style
from rich.console import Console
from rich.markdown import Markdown
from rich.prompt import Prompt
from rich.spinner import Spinner
from rich.table import Table
from rich.text import Text

from colossus.application.context import ContextService
from colossus.application.defaults import default_agent
from colossus.application.model_router import ModelRouter
from colossus.application.orchestrator import AgentOrchestrator
from colossus.application.planning import PlanService
from colossus.application.preferences import ReplPreferencesService
from colossus.application.skills import SkillResolver
from colossus.application.tasks import TaskService
from colossus.domain.agents import AgentSpec
from colossus.domain.context import ContextStatus
from colossus.domain.errors import ColossusError
from colossus.domain.plans import Plan
from colossus.domain.preferences import ReplPreferences, TranscriptStylePreference
from colossus.domain.requests import AgentRunRequest
from colossus.domain.tasks import Task, TaskStatus
from colossus.domain.tools import ToolSpec
from colossus.domain.user_prompts import UserPromptAnswer, UserPromptChoice
from colossus.interfaces.trace import EventDisplayMode, TraceRenderTheme
from colossus.interfaces.transcript import TranscriptRenderer, TranscriptRenderTheme
from colossus.ports.model_provider import ModelProvider

SlashCommand = Literal[
    "model",
    "agent",
    "tools",
    "skills",
    "trace",
    "stream",
    "events",
    "reasoning",
    "transcript",
    "multiline",
    "theme",
    "repl",
    "status",
    "tasks",
    "plan",
    "help",
    "audit",
    "compact",
    "context",
    "clear",
    "exit",
]
RunStatus = Literal["idle", "running", "done", "failed"]
ReplInteractionMode = Literal["chat", "plan"]

PLAN_MODE_INSTRUCTIONS = (
    "You are Colossus operating in REPL Plan Mode. Create a concise markdown "
    "implementation plan only. You may inspect context with tools when useful, but do "
    "not make code changes, do not apply patches, do not run mutating commands, and do "
    "not claim that implementation is complete. If a missing requirement or user "
    "preference materially changes the plan, ask exactly one structured question with "
    "user.ask before finalizing. When the plan contains clear trackable work, use "
    "task.create to create concise session tasks that mirror the major work items; "
    "skip task creation for trivial plans or when tasks already exist. Include a title, "
    "summary, key changes, tests, and assumptions."
)


@dataclass(frozen=True)
class ParsedReplCommand:
    command: SlashCommand
    argument: str = ""


SLASH_COMMANDS: tuple[str, ...] = (
    "/model",
    "/agent",
    "/tools",
    "/skills",
    "/trace",
    "/stream",
    "/events",
    "/reasoning",
    "/transcript",
    "/multiline",
    "/theme",
    "/repl",
    "/status",
    "/tasks",
    "/plan",
    "/help",
    "/audit",
    "/compact",
    "/context",
    "/clear",
    "/exit",
)

SLASH_COMMAND_DESCRIPTIONS: dict[str, str] = {
    "/model": "Show or switch the active model role.",
    "/agent": "Show the active agent spec.",
    "/tools": "List currently registered tools.",
    "/skills": "List available skills.",
    "/trace": "Compatibility toggle for compact events.",
    "/stream": "Toggle live assistant token streaming.",
    "/events": "Control tool/risk/activity event detail.",
    "/reasoning": "Toggle provider-supplied reasoning summaries.",
    "/transcript": "Switch transcript spacing and blocks.",
    "/multiline": "Toggle multiline composer mode.",
    "/theme": "Show, preview, switch, save, or reset REPL theme.",
    "/repl": "Show, save, or reset REPL preferences.",
    "/status": "Show full REPL, model, session, and context status.",
    "/tasks": "Show session task records.",
    "/plan": "Toggle or manage REPL Plan Mode.",
    "/help": "Show REPL commands.",
    "/audit": "Reserved audit view.",
    "/compact": "Create a context snapshot for the current session.",
    "/context": "Inspect or restore context snapshots.",
    "/clear": "Clear the terminal.",
    "/exit": "Leave the REPL.",
}


class SlashCommandCompleter(Completer):
    def get_completions(
        self,
        document: Document,
        complete_event: CompleteEvent,
    ) -> Iterable[Completion]:
        del complete_event
        text = document.text_before_cursor
        if not _is_slash_command_draft(text):
            return
        prefix = text.lower()
        for command in SLASH_COMMANDS:
            if command.lower().startswith(prefix):
                yield Completion(
                    command,
                    start_position=-len(text),
                    display=command,
                    display_meta=SLASH_COMMAND_DESCRIPTIONS.get(command, ""),
                )


def parse_slash_command(value: str) -> ParsedReplCommand | None:
    stripped = value.strip()
    if not stripped.startswith("/"):
        return None
    command, _, argument = stripped[1:].partition(" ")
    if command not in {item[1:] for item in SLASH_COMMANDS}:
        return None
    return ParsedReplCommand(command=command, argument=argument.strip())  # type: ignore[arg-type]


@dataclass(frozen=True)
class ReplTheme:
    name: str
    title: str
    caret: str
    continuation: str
    styles: dict[str, str]
    trace: TraceRenderTheme = field(default_factory=TraceRenderTheme)
    transcript: TranscriptRenderTheme = field(default_factory=TranscriptRenderTheme)


REQUIRED_THEME_STYLE_KEYS: frozenset[str] = frozenset(
    {
        "prompt.band",
        "prompt.title",
        "prompt.badge",
        "prompt.model",
        "prompt.caret",
        "prompt.rprompt",
        "prompt.continuation",
        "bottom-toolbar",
        "bottom-toolbar.key",
        "bottom-toolbar.warn",
    }
)
REQUIRED_TRACE_STYLE_KEYS: frozenset[str] = frozenset(
    {
        "thinking",
        "done",
        "tool_call",
        "tool_result",
        "approval_requested",
        "approval_auto_granted",
        "risk_assessment",
    }
)
REQUIRED_TRANSCRIPT_STYLE_KEYS: frozenset[str] = frozenset(
    {
        "user",
        "assistant",
        "reasoning",
        "tool",
        "tool_output",
        "approval",
        "risk",
        "error",
        "meta",
        "border",
        "activity_spinner",
    }
)


REPL_THEMES: dict[str, ReplTheme] = {
    "default": ReplTheme(
        name="default",
        title="colossus",
        caret=">",
        continuation="|",
        styles={
            "prompt.band": "bg:#3a3f45 #d7dee8",
            "prompt.title": "bg:#3a3f45 #5fd7ff bold",
            "prompt.badge": "bg:#5f6b7a #ffffff bold",
            "prompt.model": "bg:#3a3f45 #b7c3d0",
            "prompt.caret": "#5fd7ff bold",
            "prompt.rprompt": "#7f8790",
            "prompt.continuation": "#7f8790",
            "bottom-toolbar": "bg:#30343a #aeb7c2",
            "bottom-toolbar.key": "bg:#30343a #ffffff bold",
            "bottom-toolbar.warn": "bg:#30343a #ffdf5d bold",
        },
        trace=TraceRenderTheme(
            thinking="bold cyan",
            done="bold green",
            tool_call="bold blue",
            tool_result="bold green",
            approval_requested="bold yellow",
            approval_auto_granted="bold green",
            risk_assessment="bold magenta",
        ),
        transcript=TranscriptRenderTheme(
            user="white on #30343a",
            assistant="#e6edf3",
            reasoning="italic #8b949e",
            tool="bold #58a6ff",
            tool_output="#9ece6a",
            approval="bold #ffdf5d",
            risk="bold #ff79c6",
            error="bold #ff5f5f",
            meta="#7f8790",
            border="#5f6b7a",
            activity_spinner="dots",
        ),
    ),
    "mono": ReplTheme(
        name="mono",
        title="colossus",
        caret=">",
        continuation="|",
        styles={
            "prompt.band": "",
            "prompt.title": "bold",
            "prompt.badge": "bold",
            "prompt.model": "",
            "prompt.caret": "bold",
            "prompt.rprompt": "",
            "prompt.continuation": "",
            "bottom-toolbar": "",
            "bottom-toolbar.key": "bold",
            "bottom-toolbar.warn": "bold",
        },
        trace=TraceRenderTheme(),
        transcript=TranscriptRenderTheme(
            user="bold",
            assistant="",
            reasoning="italic dim",
            tool="bold",
            tool_output="",
            approval="bold",
            risk="bold",
            error="bold",
            meta="dim",
            border="dim",
            activity_spinner="line",
        ),
    ),
    "high-contrast": ReplTheme(
        name="high-contrast",
        title="colossus",
        caret=">",
        continuation="|",
        styles={
            "prompt.band": "bg:#000000 #ffffff",
            "prompt.title": "bg:#000000 #ffff00 bold",
            "prompt.badge": "bg:#ffffff #000000 bold",
            "prompt.model": "bg:#000000 #ffffff",
            "prompt.caret": "#ffff00 bold",
            "prompt.rprompt": "#ffffff",
            "prompt.continuation": "#ffff00",
            "bottom-toolbar": "bg:#000000 #ffffff",
            "bottom-toolbar.key": "bg:#000000 #ffff00 bold",
            "bottom-toolbar.warn": "bg:#000000 #ff0000 bold",
        },
        trace=TraceRenderTheme(
            thinking="bold yellow",
            done="bold white",
            tool_call="bold white",
            tool_result="bold white",
            approval_requested="bold yellow",
            approval_auto_granted="bold white",
            risk_assessment="bold red",
        ),
        transcript=TranscriptRenderTheme(
            user="black on #ffffff",
            assistant="white",
            reasoning="italic yellow",
            tool="bold cyan",
            tool_output="white",
            approval="bold yellow",
            risk="bold red",
            error="bold red",
            meta="white",
            border="white",
            activity_spinner="arc",
        ),
    ),
    "carrot": ReplTheme(
        name="carrot",
        title="colossus",
        caret=">",
        continuation="|",
        styles={
            "prompt.band": "bg:#3a2d24 #ffe7d1",
            "prompt.title": "bg:#3a2d24 #ffaf5f bold",
            "prompt.badge": "bg:#d75f00 #ffffff bold",
            "prompt.model": "bg:#3a2d24 #ffd7af",
            "prompt.caret": "#ff8700 bold",
            "prompt.rprompt": "#b88b6a",
            "prompt.continuation": "#d7875f",
            "bottom-toolbar": "bg:#2b2521 #ffd7af",
            "bottom-toolbar.key": "bg:#2b2521 #ffaf5f bold",
            "bottom-toolbar.warn": "bg:#2b2521 #ffdf5d bold",
        },
        trace=TraceRenderTheme(
            thinking="bold #ffaf5f",
            done="bold #5faf5f",
            tool_call="bold #ff8700",
            tool_result="bold #5faf5f",
            approval_requested="bold #ffdf5d",
            approval_auto_granted="bold #5faf5f",
            risk_assessment="bold #ff5f5f",
        ),
        transcript=TranscriptRenderTheme(
            user="#ffe7d1 on #3a2d24",
            assistant="#fff0df",
            reasoning="italic #b88b6a",
            tool="bold #ffaf5f",
            tool_output="#9fd77a",
            approval="bold #ffdf5d",
            risk="bold #ff5f5f",
            error="bold #ff5f5f",
            meta="#b88b6a",
            border="#d7875f",
            activity_spinner="bouncingBar",
        ),
    ),
    "hacker": ReplTheme(
        name="hacker",
        title="colossus",
        caret=">",
        continuation="|",
        styles={
            "prompt.band": "bg:#06110a #d7ffd7",
            "prompt.title": "bg:#06110a #00ff66 bold",
            "prompt.badge": "bg:#00aa44 #001b0a bold",
            "prompt.model": "bg:#06110a #8cffb0",
            "prompt.caret": "#00ff66 bold",
            "prompt.rprompt": "#3fbf6a",
            "prompt.continuation": "#00aa44",
            "bottom-toolbar": "bg:#020806 #9cffb8",
            "bottom-toolbar.key": "bg:#020806 #00ff66 bold",
            "bottom-toolbar.warn": "bg:#020806 #ffd75f bold",
        },
        trace=TraceRenderTheme(
            thinking="bold #00ff66",
            done="bold #7fff00",
            tool_call="bold #00d7ff",
            tool_result="bold #00ff66",
            approval_requested="bold #ffd75f",
            approval_auto_granted="bold #7fff00",
            risk_assessment="bold #ff5f5f",
        ),
        transcript=TranscriptRenderTheme(
            user="#d7ffd7 on #12331d",
            assistant="#d7ffd7",
            reasoning="italic #5faf87",
            tool="bold #00d7ff",
            tool_output="#00ff66",
            approval="bold #ffd75f",
            risk="bold #ff5f5f",
            error="bold #ff5f5f",
            meta="#3fbf6a",
            border="#00aa44",
            activity_spinner="aesthetic",
        ),
    ),
}
REPL_THEME_NAMES: tuple[str, ...] = tuple(REPL_THEMES)


@dataclass
class ReplDisplayState:
    session_id: str
    active_model_role: str
    model: str
    approval_mode: str
    stream_model_output: bool = True
    interaction_mode: ReplInteractionMode = "chat"
    active_plan_id: str | None = None
    active_plan_status: str | None = None
    events_mode: EventDisplayMode = "compact"
    show_reasoning: bool = True
    transcript_style: TranscriptStylePreference = "comfortable"
    multiline: bool = False
    last_run_id: str | None = None
    last_status: RunStatus = "idle"
    context_status: ContextStatus | None = None
    context_error: str | None = None
    task_summary: str = "tasks=n/a"
    theme: ReplTheme = field(default_factory=lambda: REPL_THEMES["default"])
    saved_preferences: ReplPreferences = field(default_factory=ReplPreferences)


@dataclass
class RichUserPromptHandler:
    console: Console

    async def ask(
        self,
        *,
        question: str,
        choices: tuple[UserPromptChoice, ...] = (),
        allow_freeform: bool = True,
    ) -> UserPromptAnswer:
        self.console.print("[bold cyan]question[/bold cyan]", question)
        if choices:
            table = Table("Option", "ID", "Label", "Description")
            for index, choice in enumerate(choices, start=1):
                table.add_row(str(index), choice.id, choice.label, choice.description)
            self.console.print(table)
        while True:
            response = self._ask_user("Choose an option or type an answer").strip()
            answer = _match_user_prompt_answer(response, choices, allow_freeform)
            if answer is not None:
                return answer
            if choices and not allow_freeform:
                self.console.print("Please choose one of the listed options.")
            else:
                self.console.print("Please enter an answer.")

    def _ask_user(self, prompt: str) -> str:
        return Prompt.ask(prompt, console=self.console)


async def run_repl(
    orchestrator: AgentOrchestrator,
    skills: SkillResolver,
    context_service: ContextService | None = None,
    provider: ModelProvider | None = None,
    agent: AgentSpec | None = None,
    *,
    task_service: TaskService | None = None,
    plan_service: PlanService | None = None,
    model_router: ModelRouter | None = None,
    active_model_role: str = "primary",
    orchestrator_factory: Callable[[str], AgentOrchestrator] | None = None,
    context_model: str | None = None,
    approval_mode: str = "ask",
    history_path: Path | None = None,
    theme_name: str | None = None,
    preferences_service: ReplPreferencesService | None = None,
    theme_dirs: tuple[Path, ...] = (),
) -> None:
    user_themes = load_user_repl_themes(theme_dirs)
    themes = {**REPL_THEMES, **user_themes}
    preferences = await _load_preferences(preferences_service)
    selected_theme = theme_name or preferences.theme
    console = Console()
    try:
        startup_theme = _theme_by_name(selected_theme, themes)
    except ColossusError:
        if theme_name is not None:
            raise
        console.print(
            f"[yellow]Saved REPL theme {selected_theme} is unavailable; "
            "using default for this session.[/yellow]"
        )
        startup_theme = REPL_THEMES["default"]
    session: PromptSession[str] = PromptSession(
        completer=SlashCommandCompleter(),
        complete_while_typing=True,
        reserve_space_for_menu=8,
        history=FileHistory(str(history_path)) if history_path is not None else None,
        erase_when_done=True,
    )
    agent = agent or default_agent()
    display_state = ReplDisplayState(
        session_id=str(uuid4()),
        active_model_role=active_model_role,
        model=agent.model,
        approval_mode=approval_mode,
        stream_model_output=preferences.stream_model_output,
        events_mode=preferences.events_mode,
        show_reasoning=preferences.show_reasoning,
        transcript_style=preferences.transcript_style,
        multiline=preferences.multiline,
        theme=startup_theme,
        saved_preferences=preferences,
    )
    trace_renderer = TranscriptRenderer(
        console,
        events_mode=display_state.events_mode,
        stream_model_output=display_state.stream_model_output,
        show_reasoning=display_state.show_reasoning,
        transcript_style=display_state.transcript_style,
        theme=display_state.theme.transcript,
    )
    orchestrator.set_event_observer(trace_renderer.render)
    _render_repl_startup(console, display_state)
    key_bindings = _composer_key_bindings()
    while True:
        try:
            await _refresh_context_status(display_state, context_service)
            await _refresh_task_status(display_state, task_service)
            line = await session.prompt_async(
                _prompt_message(display_state),
                multiline=display_state.multiline,
                key_bindings=key_bindings,
                bottom_toolbar=lambda: _bottom_toolbar(display_state),
                rprompt=lambda: _right_prompt(display_state),
                prompt_continuation=lambda width, line, wrap: _prompt_continuation(
                    display_state,
                    width,
                    line,
                    wrap,
                ),
                enable_history_search=True,
                style=_style_for_theme(display_state.theme),
            )
        except (EOFError, KeyboardInterrupt):
            console.print()
            return
        command = parse_slash_command(line)
        if command is not None:
            if command.command == "exit":
                return
            if command.command == "skills":
                for skill in skills.list_skills():
                    console.print(f"{skill.manifest.name} {skill.manifest.version}")
                continue
            if command.command == "model":
                if not command.argument:
                    _render_model(console, agent, display_state.active_model_role, model_router)
                    continue
                if model_router is None or orchestrator_factory is None:
                    agent = default_agent(command.argument)
                    display_state.model = agent.model
                    console.print(f"Model set to {agent.model}.")
                    continue
                try:
                    route = model_router.resolve(command.argument)
                except ColossusError as exc:
                    console.print(f"[red]Model switch failed:[/red] {exc}")
                    continue
                display_state.active_model_role = command.argument
                display_state.model = route.profile.model
                agent = default_agent(route.profile.model)
                orchestrator = orchestrator_factory(display_state.active_model_role)
                orchestrator.set_event_observer(trace_renderer.render)
                console.print(
                    f"Model role {route.role} -> {route.profile.model} "
                    f"({route.profile.provider}/{route.profile_name})"
                )
                continue
            if command.command == "agent":
                console.print(agent.model_dump_json(indent=2))
                continue
            if command.command == "tools":
                _render_tools(console, orchestrator.tool_specs())
                continue
            if command.command == "clear":
                console.clear()
                continue
            if command.command == "trace":
                trace_renderer.events_mode = _trace_events_mode(
                    command.argument,
                    trace_renderer.events_mode,
                )
                display_state.events_mode = trace_renderer.events_mode
                console.print(f"Events are {trace_renderer.events_mode}.")
                continue
            if command.command == "stream":
                trace_renderer.stream_model_output = _toggle_on_off(
                    command.argument,
                    trace_renderer.stream_model_output,
                )
                display_state.stream_model_output = trace_renderer.stream_model_output
                console.print(f"Stream is {'on' if trace_renderer.stream_model_output else 'off'}.")
                continue
            if command.command == "events":
                trace_renderer.events_mode = _events_mode(command.argument)
                display_state.events_mode = trace_renderer.events_mode
                console.print(f"Events are {trace_renderer.events_mode}.")
                continue
            if command.command == "reasoning":
                trace_renderer.show_reasoning = _toggle_on_off(
                    command.argument,
                    trace_renderer.show_reasoning,
                )
                display_state.show_reasoning = trace_renderer.show_reasoning
                status = "on" if trace_renderer.show_reasoning else "off"
                console.print(f"Reasoning summaries are {status}.")
                continue
            if command.command == "transcript":
                display_state.transcript_style = _transcript_style_mode(
                    command.argument,
                    display_state.transcript_style,
                )
                trace_renderer.transcript_style = display_state.transcript_style
                console.print(f"Transcript style is {display_state.transcript_style}.")
                continue
            if command.command == "multiline":
                display_state.multiline = _multiline_mode(
                    command.argument,
                    display_state.multiline,
                )
                mode = "multiline" if display_state.multiline else "single-line"
                console.print(f"Composer mode is {mode}.")
                continue
            if command.command == "theme":
                await _handle_theme_command(
                    console,
                    command.argument,
                    display_state,
                    themes,
                    trace_renderer,
                    preferences_service,
                )
                continue
            if command.command == "repl":
                await _handle_repl_command(
                    console,
                    command.argument,
                    display_state,
                    trace_renderer,
                    preferences_service,
                )
                continue
            if command.command == "status":
                await _refresh_context_status(display_state, context_service)
                await _refresh_task_status(display_state, task_service)
                _render_status(console, display_state)
                continue
            if command.command == "tasks":
                await _handle_tasks_command(
                    console,
                    task_service,
                    display_state.session_id,
                    command.argument,
                )
                await _refresh_task_status(display_state, task_service)
                continue
            if command.command == "plan":
                await _handle_plan_command(
                    console,
                    plan_service,
                    display_state,
                    command.argument,
                    orchestrator,
                    agent,
                    trace_renderer,
                )
                continue
            if command.command == "help":
                _render_help(console, display_state)
                continue
            if command.command == "compact":
                if context_service is None:
                    console.print("Context service is not configured.")
                    continue
                try:
                    snapshot = await context_service.compact_session(
                        session_id=display_state.session_id,
                        model=agent.model,
                        provider=provider,
                        summary_model=context_model,
                    )
                except ColossusError as exc:
                    console.print(f"Context compaction failed: {exc}")
                    continue
                await _refresh_context_status(display_state, context_service)
                console.print(f"Compacted into snapshot {snapshot.id}")
                continue
            if command.command == "context":
                if context_service is None:
                    console.print("Context service is not configured.")
                    continue
                await _handle_context_command(
                    console,
                    context_service,
                    display_state.session_id,
                    agent.model,
                    command.argument,
                )
                await _refresh_context_status(display_state, context_service)
                continue
            console.print(f"/{command.command} is reserved for the full runtime.")
            continue
        try:
            trace_renderer.render_user_prompt(line)
            if _show_submit_summary(display_state):
                console.print(f"[dim]{_format_submit_summary(display_state, line)}[/dim]")
            display_state.last_status = "running"
            trace_renderer.begin_run(activity_context=_format_run_toolbar(display_state, line))
            if display_state.interaction_mode == "plan":
                result = await orchestrator.run(
                    AgentRunRequest(
                        prompt=line,
                        agent=_plan_agent(agent),
                        session_id=display_state.session_id,
                    )
                )
                plan = await _save_repl_plan(
                    plan_service,
                    display_state,
                    prompt=line,
                    content=result.final_output,
                )
                trace_renderer.end_run()
                display_state.last_run_id = result.run_id
                display_state.last_status = "done"
                if not trace_renderer.rendered_model_output:
                    trace_renderer.render_final_answer(result.final_output)
                await _prompt_for_plan_review(
                    console,
                    plan_service,
                    display_state,
                    plan,
                    orchestrator,
                    agent,
                    trace_renderer,
                )
                await _refresh_context_status(display_state, context_service)
                await _refresh_task_status(display_state, task_service)
                continue
            result = await orchestrator.run(
                AgentRunRequest(
                    prompt=line,
                    agent=agent,
                    session_id=display_state.session_id,
                )
            )
        except ColossusError as exc:
            trace_renderer.end_run()
            display_state.last_status = "failed"
            await _refresh_context_status(display_state, context_service)
            await _refresh_task_status(display_state, task_service)
            console.print(f"[red]Run failed:[/red] {exc}")
            continue
        trace_renderer.end_run()
        display_state.last_run_id = result.run_id
        display_state.last_status = "done"
        await _refresh_context_status(display_state, context_service)
        await _refresh_task_status(display_state, task_service)
        if not trace_renderer.rendered_model_output:
            trace_renderer.render_final_answer(result.final_output)
        if not trace_renderer.rendered_model_output:
            trace_renderer.render_empty_response()


async def _handle_context_command(
    console: Console,
    context_service: ContextService,
    session_id: str,
    model: str,
    argument: str,
) -> None:
    if argument == "snapshots":
        snapshots = await context_service.list_snapshots(session_id)
        for snapshot in snapshots:
            console.print(
                f"{snapshot.id}\t{snapshot.strategy}\t"
                f"{snapshot.source_message_range[0]}-{snapshot.source_message_range[1]}"
            )
        if not snapshots:
            console.print("No context snapshots.")
        return
    if argument.startswith("restore "):
        snapshot_id = argument.removeprefix("restore ").strip()
        try:
            snapshot = await context_service.restore_snapshot(snapshot_id)
        except ColossusError as exc:
            console.print(f"Context restore failed: {exc}")
            return
        console.print(f"Restored snapshot {snapshot.id}")
        return
    status = await context_service.status(session_id, model)
    console.print(status.model_dump_json(indent=2))


def run_repl_sync(
    orchestrator: AgentOrchestrator,
    skills: SkillResolver,
    context_service: ContextService | None = None,
    provider: ModelProvider | None = None,
    agent: AgentSpec | None = None,
    *,
    task_service: TaskService | None = None,
    plan_service: PlanService | None = None,
    model_router: ModelRouter | None = None,
    active_model_role: str = "primary",
    orchestrator_factory: Callable[[str], AgentOrchestrator] | None = None,
    context_model: str | None = None,
    approval_mode: str = "ask",
    history_path: Path | None = None,
    theme_name: str | None = None,
    preferences_service: ReplPreferencesService | None = None,
    theme_dirs: tuple[Path, ...] = (),
) -> None:
    asyncio.run(
        run_repl(
            orchestrator,
            skills,
            context_service,
            provider,
            agent,
            task_service=task_service,
            plan_service=plan_service,
            model_router=model_router,
            active_model_role=active_model_role,
            orchestrator_factory=orchestrator_factory,
            context_model=context_model,
            approval_mode=approval_mode,
            history_path=history_path,
            theme_name=theme_name,
            preferences_service=preferences_service,
            theme_dirs=theme_dirs,
        )
    )


def _render_repl_startup(console: Console, state: ReplDisplayState) -> None:
    console.clear()
    console.print("[bold]Colossus REPL[/bold]  Type /exit to leave.")
    console.print(
        f"[dim]session_id={state.session_id} "
        f"mode={state.interaction_mode} "
        f"composer={'multi' if state.multiline else 'single'} "
        f"theme={state.theme.name} stream={_on_off(state.stream_model_output)} "
        f"events={state.events_mode} "
        f"transcript={state.transcript_style} "
        f"reasoning={_on_off(state.show_reasoning)}[/dim]"
    )


def _trace_enabled(argument: str, current: bool) -> bool:
    normalized = argument.strip().lower()
    if normalized in {"on", "true", "1", "yes"}:
        return True
    if normalized in {"off", "false", "0", "no"}:
        return False
    return not current


def _toggle_on_off(argument: str, current: bool) -> bool:
    return _trace_enabled(argument, current)


def _multiline_mode(argument: str, current: bool) -> bool:
    normalized = argument.strip().lower()
    if normalized in {"on", "true", "1", "yes", "multi", "multiline"}:
        return True
    if normalized in {"off", "false", "0", "no", "single", "single-line"}:
        return False
    return not current


def _transcript_style_mode(
    argument: str,
    current: TranscriptStylePreference,
) -> TranscriptStylePreference:
    normalized = argument.strip().lower()
    if normalized in {"comfortable", "cards"}:
        return "comfortable"
    if normalized in {"compact", "clean"}:
        return "compact"
    return "compact" if current == "comfortable" else "comfortable"


def _theme_by_name(name: str, themes: dict[str, ReplTheme] | None = None) -> ReplTheme:
    normalized = name.strip().lower()
    available = themes or REPL_THEMES
    theme = available.get(normalized)
    if theme is None:
        names = ", ".join(available)
        raise ColossusError(f"Unknown REPL theme: {name}. Available themes: {names}.")
    validate_repl_theme(theme)
    return theme


def repl_theme_names() -> tuple[str, ...]:
    return REPL_THEME_NAMES


def load_user_repl_themes(theme_dirs: tuple[Path, ...]) -> dict[str, ReplTheme]:
    themes: dict[str, ReplTheme] = {}
    for directory in theme_dirs:
        if not directory.exists():
            continue
        for path in sorted((*directory.glob("*.json"), *directory.glob("*.toml"))):
            theme = _load_theme_file(path)
            themes[theme.name] = theme
    return themes


def validate_repl_theme(theme: ReplTheme) -> None:
    missing_style_keys = REQUIRED_THEME_STYLE_KEYS - set(theme.styles)
    extra_style_keys = set(theme.styles) - REQUIRED_THEME_STYLE_KEYS
    if missing_style_keys or extra_style_keys:
        raise ColossusError(
            "Theme style keys are invalid: "
            f"missing={sorted(missing_style_keys)} extra={sorted(extra_style_keys)}"
        )
    trace_keys = set(theme.trace.__dict__)
    missing_trace_keys = REQUIRED_TRACE_STYLE_KEYS - trace_keys
    extra_trace_keys = trace_keys - REQUIRED_TRACE_STYLE_KEYS
    if missing_trace_keys or extra_trace_keys:
        raise ColossusError(
            "Theme trace style keys are invalid: "
            f"missing={sorted(missing_trace_keys)} extra={sorted(extra_trace_keys)}"
        )
    transcript_keys = set(theme.transcript.__dict__)
    missing_transcript_keys = REQUIRED_TRANSCRIPT_STYLE_KEYS - transcript_keys
    extra_transcript_keys = transcript_keys - REQUIRED_TRANSCRIPT_STYLE_KEYS
    if missing_transcript_keys or extra_transcript_keys:
        raise ColossusError(
            "Theme transcript style keys are invalid: "
            f"missing={sorted(missing_transcript_keys)} extra={sorted(extra_transcript_keys)}"
        )
    try:
        Spinner(theme.transcript.activity_spinner)
    except KeyError as exc:
        raise ColossusError(
            "Theme transcript activity spinner is invalid: "
            f"{theme.transcript.activity_spinner}. "
            f"Available examples: {', '.join(_activity_spinner_examples())}."
        ) from exc


def _activity_spinner_examples() -> tuple[str, ...]:
    return ("dots", "line", "arc", "bouncingBar", "aesthetic")


def _load_theme_file(path: Path) -> ReplTheme:
    if path.suffix == ".json":
        data = json.loads(path.read_text(encoding="utf-8"))
    elif path.suffix == ".toml":
        data = tomllib.loads(path.read_text(encoding="utf-8"))
    else:  # pragma: no cover - guarded by caller glob
        raise ColossusError(f"Unsupported theme file: {path}")
    if not isinstance(data, dict):
        raise ColossusError(f"Theme file must contain an object: {path}")
    return _theme_from_mapping(data, path)


def _theme_from_mapping(data: dict[str, object], path: Path) -> ReplTheme:
    name_value = data.get("name", path.stem)
    if not isinstance(name_value, str) or not name_value.strip():
        raise ColossusError(f"Theme name must be a non-empty string: {path}")
    name = name_value.strip().lower()
    if any(character in name for character in ("/", "\\", " ")):
        raise ColossusError(f"Theme name must be a simple identifier: {path}")
    styles_value = data.get("styles", {})
    if not isinstance(styles_value, dict):
        raise ColossusError(f"Theme styles must be an object: {path}")
    extra_style_keys = set(styles_value) - REQUIRED_THEME_STYLE_KEYS
    if extra_style_keys:
        keys = ", ".join(sorted(extra_style_keys))
        raise ColossusError(f"Theme has unsupported style keys in {path}: {keys}")
    styles = {**REPL_THEMES["default"].styles}
    for key, value in styles_value.items():
        if not isinstance(key, str) or not isinstance(value, str):
            raise ColossusError(f"Theme style keys and values must be strings: {path}")
        styles[key] = value
    trace_value = data.get("trace", data.get("trace_styles", {}))
    if not isinstance(trace_value, dict):
        raise ColossusError(f"Theme trace styles must be an object: {path}")
    extra_trace_keys = set(trace_value) - REQUIRED_TRACE_STYLE_KEYS
    if extra_trace_keys:
        keys = ", ".join(sorted(extra_trace_keys))
        raise ColossusError(f"Theme has unsupported trace style keys in {path}: {keys}")
    trace_data = TraceRenderTheme().__dict__.copy()
    for key, value in trace_value.items():
        if not isinstance(key, str) or not isinstance(value, str):
            raise ColossusError(f"Theme trace style keys and values must be strings: {path}")
        trace_data[key] = value
    transcript_value = data.get("transcript", data.get("transcript_styles", {}))
    if not isinstance(transcript_value, dict):
        raise ColossusError(f"Theme transcript styles must be an object: {path}")
    extra_transcript_keys = set(transcript_value) - REQUIRED_TRANSCRIPT_STYLE_KEYS
    if extra_transcript_keys:
        keys = ", ".join(sorted(extra_transcript_keys))
        raise ColossusError(f"Theme has unsupported transcript style keys in {path}: {keys}")
    transcript_data = TranscriptRenderTheme().__dict__.copy()
    for key, value in transcript_value.items():
        if not isinstance(key, str) or not isinstance(value, str):
            raise ColossusError(
                f"Theme transcript style keys and values must be strings: {path}"
            )
        transcript_data[key] = value
    theme = ReplTheme(
        name=name,
        title=_optional_string(data.get("title"), "colossus", path),
        caret=_optional_string(data.get("caret"), ">", path),
        continuation=_optional_string(data.get("continuation"), "|", path),
        styles=styles,
        trace=TraceRenderTheme(**trace_data),
        transcript=TranscriptRenderTheme(**transcript_data),
    )
    validate_repl_theme(theme)
    return theme


def _optional_string(value: object, default: str, path: Path) -> str:
    if value is None:
        return default
    if not isinstance(value, str):
        raise ColossusError(f"Theme field must be a string: {path}")
    return value


async def _load_preferences(
    preferences_service: ReplPreferencesService | None,
) -> ReplPreferences:
    if preferences_service is None:
        return ReplPreferences()
    return await preferences_service.load()


def _preferences_from_state(state: ReplDisplayState) -> ReplPreferences:
    return ReplPreferences(
        theme=state.theme.name,
        multiline=state.multiline,
        stream_model_output=state.stream_model_output,
        events_mode=state.events_mode,
        show_reasoning=state.show_reasoning,
        transcript_style=state.transcript_style,
    )


async def _save_preferences(
    state: ReplDisplayState,
    preferences_service: ReplPreferencesService | None,
) -> ReplPreferences:
    preferences = _preferences_from_state(state)
    if preferences_service is not None:
        preferences = await preferences_service.save(preferences)
    state.saved_preferences = preferences
    return preferences


async def _reset_preferences(
    state: ReplDisplayState,
    trace_renderer: TranscriptRenderer,
    preferences_service: ReplPreferencesService | None,
) -> ReplPreferences:
    preferences = (
        await preferences_service.reset()
        if preferences_service is not None
        else ReplPreferences()
    )
    _apply_preferences(state, preferences, REPL_THEMES)
    _sync_renderer(trace_renderer, state)
    state.saved_preferences = preferences
    return preferences


def _apply_preferences(
    state: ReplDisplayState,
    preferences: ReplPreferences,
    themes: dict[str, ReplTheme],
) -> None:
    state.theme = _theme_by_name(preferences.theme, themes)
    state.multiline = preferences.multiline
    state.stream_model_output = preferences.stream_model_output
    state.events_mode = preferences.events_mode
    state.show_reasoning = preferences.show_reasoning
    state.transcript_style = preferences.transcript_style


def _sync_renderer(renderer: TranscriptRenderer, state: ReplDisplayState) -> None:
    renderer.events_mode = state.events_mode
    renderer.stream_model_output = state.stream_model_output
    renderer.show_reasoning = state.show_reasoning
    renderer.transcript_style = state.transcript_style
    renderer.theme = state.theme.transcript
    renderer.sync_theme()


async def _handle_theme_command(
    console: Console,
    argument: str,
    state: ReplDisplayState,
    themes: dict[str, ReplTheme],
    trace_renderer: TranscriptRenderer,
    preferences_service: ReplPreferencesService | None,
) -> None:
    action, _, rest = argument.partition(" ")
    action = action.strip().lower()
    value = rest.strip()
    if not action:
        _render_themes(console, state.theme, themes)
        return
    if action == "preview":
        names = (value,) if value else tuple(themes)
        _render_theme_preview(console, themes, names)
        return
    if action == "save":
        if value:
            state.theme = _theme_by_name(value, themes)
        _sync_renderer(trace_renderer, state)
        await _save_preferences(state, preferences_service)
        console.print(f"Saved REPL theme {state.theme.name}.")
        return
    if action == "reset":
        state.theme = REPL_THEMES["default"]
        _sync_renderer(trace_renderer, state)
        await _save_preferences(state, preferences_service)
        console.print("Saved REPL theme default.")
        return
    state.theme = _theme_by_name(argument, themes)
    _sync_renderer(trace_renderer, state)
    console.print(f"Theme set to {state.theme.name}.")


async def _handle_repl_command(
    console: Console,
    argument: str,
    state: ReplDisplayState,
    trace_renderer: TranscriptRenderer,
    preferences_service: ReplPreferencesService | None,
) -> None:
    normalized = argument.strip().lower()
    if normalized in {"prefs", "preferences", ""}:
        _render_repl_preferences(console, state)
        return
    if normalized == "save":
        await _save_preferences(state, preferences_service)
        console.print("Saved REPL preferences.")
        return
    if normalized == "reset":
        await _reset_preferences(state, trace_renderer, preferences_service)
        console.print("Reset REPL preferences.")
        return
    console.print("Use /repl prefs, /repl save, or /repl reset.")


async def _handle_plan_command(
    console: Console,
    plan_service: PlanService | None,
    state: ReplDisplayState,
    argument: str,
    orchestrator: AgentOrchestrator,
    agent: AgentSpec,
    trace_renderer: TranscriptRenderer,
) -> None:
    if plan_service is None:
        console.print("Plan service is not configured.")
        return
    action = argument.strip().lower()
    if action in {"", "toggle"}:
        state.interaction_mode = "chat" if state.interaction_mode == "plan" else "plan"
        console.print(f"Plan Mode is {state.interaction_mode}.")
        return
    if action == "on":
        state.interaction_mode = "plan"
        console.print("Plan Mode is on.")
        return
    if action == "off":
        state.interaction_mode = "chat"
        console.print("Plan Mode is off.")
        return
    if action == "discard":
        state.active_plan_id = None
        state.active_plan_status = None
        console.print("Cleared active plan.")
        return
    if action == "list":
        _render_plan_list(
            console,
            await plan_service.list_plans(state.session_id),
            active_plan_id=state.active_plan_id,
        )
        return
    if action == "show":
        plan = await _active_plan(console, plan_service, state)
        if plan is not None:
            _render_plan(console, plan)
        return
    if action == "approve":
        plan = await _active_plan(console, plan_service, state)
        if plan is None:
            return
        if plan.status == "executed":
            console.print("Plan has already been executed.")
            return
        approved = await plan_service.approve_plan(plan.id)
        state.active_plan_status = approved.status
        console.print(f"Approved plan {approved.id}.")
        return
    if action == "execute":
        await _execute_active_plan(
            console,
            plan_service,
            state,
            orchestrator,
            agent,
            trace_renderer,
        )
        return
    console.print("Use /plan [on|off|show|approve|execute|list|discard].")


async def _save_repl_plan(
    plan_service: PlanService | None,
    state: ReplDisplayState,
    *,
    prompt: str,
    content: str,
) -> Plan:
    if plan_service is None:
        raise ColossusError("Plan service is not configured.")
    if state.active_plan_id is not None:
        try:
            current = await plan_service.get_plan(state.active_plan_id)
        except ColossusError:
            current = None
        if current is not None and current.status == "draft":
            plan = await plan_service.replace_draft_plan(current.id, prompt, content)
            state.active_plan_status = plan.status
            return plan
    plan = await plan_service.create_plan(prompt, state.session_id, content=content)
    state.active_plan_id = plan.id
    state.active_plan_status = plan.status
    return plan


async def _prompt_for_plan_review(
    console: Console,
    plan_service: PlanService | None,
    state: ReplDisplayState,
    plan: Plan,
    orchestrator: AgentOrchestrator,
    agent: AgentSpec,
    trace_renderer: TranscriptRenderer,
) -> None:
    if plan_service is None:
        console.print(
            f"Saved draft plan {plan.id}. Next: /plan approve, /plan execute, or /plan off."
        )
        return
    console.print(f"Saved draft plan {plan.id}.")
    handler = RichUserPromptHandler(console)
    answer = await handler.ask(
        question="Approve this plan?",
        choices=(
            UserPromptChoice(
                id="approve_execute",
                label="Approve and execute",
                description="Approve the active draft and immediately run it.",
            ),
            UserPromptChoice(
                id="keep_draft",
                label="Keep draft",
                description="Leave the plan saved without executing it.",
            ),
            UserPromptChoice(
                id="revise",
                label="Revise plan",
                description="Stay in Plan Mode so your next message replaces the draft.",
            ),
            UserPromptChoice(
                id="discard",
                label="Discard",
                description="Clear the active plan from this REPL session.",
            ),
        ),
        allow_freeform=False,
    )
    if answer.choice_id == "approve_execute":
        approved = await plan_service.approve_plan(plan.id)
        state.active_plan_status = approved.status
        console.print(f"Approved plan {approved.id}.")
        await _execute_active_plan(
            console,
            plan_service,
            state,
            orchestrator,
            agent,
            trace_renderer,
        )
        return
    if answer.choice_id == "keep_draft":
        state.interaction_mode = "chat"
        console.print("Kept draft plan. Use /plan show, /plan approve, or /plan execute.")
        return
    if answer.choice_id == "revise":
        state.interaction_mode = "plan"
        console.print("Plan Mode is still on. Send the revision request next.")
        return
    if answer.choice_id == "discard":
        state.active_plan_id = None
        state.active_plan_status = None
        state.interaction_mode = "chat"
        console.print("Discarded active plan.")


async def _active_plan(
    console: Console,
    plan_service: PlanService,
    state: ReplDisplayState,
) -> Plan | None:
    if state.active_plan_id is None:
        console.print("No active plan.")
        return None
    try:
        plan = await plan_service.get_plan(state.active_plan_id)
    except ColossusError as exc:
        state.active_plan_id = None
        state.active_plan_status = None
        console.print(f"Active plan is unavailable: {exc}")
        return None
    state.active_plan_status = plan.status
    return plan


async def _execute_active_plan(
    console: Console,
    plan_service: PlanService,
    state: ReplDisplayState,
    orchestrator: AgentOrchestrator,
    agent: AgentSpec,
    trace_renderer: TranscriptRenderer,
) -> None:
    plan = await _active_plan(console, plan_service, state)
    if plan is None:
        return
    if plan.status == "draft":
        console.print("Approve first with /plan approve.")
        return
    if plan.status == "executed":
        console.print("Plan has already been executed.")
        return
    approved_plan = await plan_service.require_approved(plan.id)
    prompt = _execution_prompt(approved_plan)
    state.last_status = "running"
    trace_renderer.begin_run(activity_context=_format_run_toolbar(state, prompt))
    try:
        result = await orchestrator.run(
            AgentRunRequest(
                prompt=prompt,
                agent=agent,
                session_id=state.session_id,
                plan_id=plan.id,
            )
        )
    except ColossusError as exc:
        trace_renderer.end_run()
        state.last_status = "failed"
        console.print(f"[red]Plan execution failed:[/red] {exc}")
        return
    trace_renderer.end_run()
    state.last_run_id = result.run_id
    state.last_status = "done"
    executed = await plan_service.mark_executed(plan.id, result.run_id)
    state.active_plan_status = executed.status
    state.interaction_mode = "chat"
    if not trace_renderer.rendered_model_output:
        trace_renderer.render_final_answer(result.final_output)
    console.print(f"Executed plan {plan.id}.")


def _plan_agent(agent: AgentSpec) -> AgentSpec:
    return agent.model_copy(
        update={"instructions": f"{agent.instructions}\n\n{PLAN_MODE_INSTRUCTIONS}"}
    )


def _execution_prompt(plan: Plan) -> str:
    if plan.content.strip():
        return (
            "Execute the approved plan.\n\n"
            f"Original request:\n{plan.prompt}\n\n"
            f"Plan:\n{plan.content}"
        )
    return f"Execute the approved plan.\n\nOriginal request:\n{plan.prompt}"


def _match_user_prompt_answer(
    response: str,
    choices: tuple[UserPromptChoice, ...],
    allow_freeform: bool,
) -> UserPromptAnswer | None:
    if not response:
        return None
    for index, choice in enumerate(choices, start=1):
        if response == str(index) or response == choice.id:
            return UserPromptAnswer(answer=choice.label, choice_id=choice.id)
    if allow_freeform:
        return UserPromptAnswer(answer=response)
    return None


def _render_plan(console: Console, plan: Plan) -> None:
    console.print(f"[bold]Plan:[/bold] {plan.id}")
    console.print(f"Status: {plan.status}")
    console.print(f"Prompt: {plan.prompt}")
    if plan.content.strip():
        console.print(Markdown(plan.content))
        return
    table = Table("Step", "Title", "Mutation", "Detail")
    for step in plan.steps:
        table.add_row(str(step.index), step.title, str(step.requires_mutation), step.detail)
    console.print(table)


def _render_plan_list(
    console: Console,
    plans: tuple[Plan, ...],
    *,
    active_plan_id: str | None = None,
) -> None:
    if not plans:
        console.print("No plans.")
        return
    table = Table("Active", "Status", "ID", "Prompt")
    for plan in plans:
        table.add_row(
            "*" if plan.id == active_plan_id else "",
            plan.status,
            plan.id,
            _short_text(plan.prompt, 80),
        )
    console.print(table)


def _events_mode(argument: str) -> EventDisplayMode:
    normalized = argument.strip().lower()
    if normalized in {"compact", "verbose", "off"}:
        return normalized  # type: ignore[return-value]
    return "compact"


def _trace_events_mode(argument: str, current: EventDisplayMode) -> EventDisplayMode:
    normalized = argument.strip().lower()
    if normalized in {"on", "true", "1", "yes", "compact"}:
        return "compact"
    if normalized in {"verbose"}:
        return "verbose"
    if normalized in {"off", "false", "0", "no"}:
        return "off"
    return "compact" if current == "off" else "off"


def _prompt_message(state: ReplDisplayState) -> AnyFormattedText:
    mode = "MULTI" if state.multiline else "SINGLE"
    if state.interaction_mode == "plan":
        mode = "PLAN"
    model = _short_text(f"{state.active_model_role}:{state.model}", 36)
    theme = state.theme
    return [
        ("class:prompt.band", " "),
        ("class:prompt.title", theme.title),
        ("class:prompt.band", " "),
        ("class:prompt.badge", f" {mode} "),
        ("class:prompt.model", f" {model} "),
        ("class:prompt.band", "\n"),
        ("class:prompt.caret", f"{theme.caret} "),
    ]


def _composer_key_bindings() -> KeyBindings:
    bindings = KeyBindings()

    @bindings.add("/")
    def _slash(event) -> None:  # type: ignore[no-untyped-def]
        before_cursor = event.current_buffer.document.text_before_cursor
        event.current_buffer.insert_text("/")
        after_cursor = event.current_buffer.document.text_before_cursor
        if not before_cursor and _is_slash_command_draft(after_cursor):
            event.current_buffer.start_completion(select_first=False)

    for key in "abcdefghijklmnopqrstuvwxyz":
        _bind_slash_completion_key(bindings, key)

    @bindings.add("escape", "enter")
    def _accept(event) -> None:  # type: ignore[no-untyped-def]
        event.current_buffer.validate_and_handle()

    return bindings


def _bind_slash_completion_key(bindings: KeyBindings, key: str) -> None:
    @bindings.add(key, filter=Condition(_current_buffer_is_slash_command_draft))
    def _slash_command_key(event) -> None:  # type: ignore[no-untyped-def]
        event.current_buffer.insert_text(event.data)
        event.current_buffer.start_completion(select_first=False)


def _current_buffer_is_slash_command_draft() -> bool:
    app = get_app_or_none()
    if app is None:
        return False
    return _is_slash_command_draft(app.current_buffer.document.text_before_cursor)


def _is_slash_command_draft(text: str) -> bool:
    return text.startswith("/") and not any(character.isspace() for character in text)


def _prompt_continuation(
    state: ReplDisplayState,
    width: int,
    line_number: int,
    wrap_count: int,
) -> AnyFormattedText:
    del line_number, wrap_count
    return [
        (
            "class:prompt.continuation",
            f"{' ' * max(width - 3, 0)}{state.theme.continuation} ",
        )
    ]


def _bottom_toolbar(state: ReplDisplayState) -> AnyFormattedText:
    draft, line, column = _current_prompt_metrics()
    slash_suggestions = _format_slash_suggestions(draft)
    if slash_suggestions:
        return [("class:bottom-toolbar.key", slash_suggestions)]
    return [("class:bottom-toolbar", _format_repl_toolbar(state, draft, line, column))]


def _right_prompt(state: ReplDisplayState) -> AnyFormattedText:
    hint = "Esc+Enter sends" if state.multiline else "Enter sends"
    return [("class:prompt.rprompt", hint)]


def _format_slash_suggestions(draft_text: str) -> str:
    if not _is_slash_command_draft(draft_text):
        return ""
    prefix = draft_text.lower()
    matches = tuple(command for command in SLASH_COMMANDS if command.lower().startswith(prefix))
    if not matches:
        return "commands: no matches"
    shown = " ".join(matches[:8])
    suffix = " ..." if len(matches) > 8 else ""
    return f"commands: {shown}{suffix}"


def _style_for_theme(theme: ReplTheme) -> Style:
    return Style.from_dict(theme.styles)


def _current_prompt_metrics() -> tuple[str, int, int]:
    app = get_app_or_none()
    if app is None:
        return "", 1, 1
    document = app.current_buffer.document
    return document.text, document.cursor_position_row + 1, document.cursor_position_col + 1


async def _refresh_context_status(
    state: ReplDisplayState,
    context_service: ContextService | None,
) -> None:
    if context_service is None:
        state.context_status = None
        state.context_error = None
        return
    try:
        state.context_status = await context_service.status(state.session_id, state.model)
    except ColossusError as exc:
        state.context_status = None
        state.context_error = str(exc)
    else:
        state.context_error = None


async def _refresh_task_status(
    state: ReplDisplayState,
    task_service: TaskService | None,
) -> None:
    if task_service is None:
        state.task_summary = "tasks=n/a"
        return
    try:
        tasks = await task_service.list_tasks(session_id=state.session_id)
    except ColossusError as exc:
        state.task_summary = f"tasks=error:{_short_text(str(exc), 20)}"
        return
    open_count = sum(1 for task in tasks if task.status not in {"completed", "cancelled"})
    state.task_summary = f"tasks={open_count}/{len(tasks)}"


def _format_repl_toolbar(
    state: ReplDisplayState,
    draft_text: str,
    cursor_line: int,
    cursor_column: int,
) -> str:
    mode = state.interaction_mode if state.interaction_mode == "plan" else (
        "multi" if state.multiline else "single"
    )
    lines = _line_count(draft_text)
    return (
        f"mode={mode} model={state.active_model_role}:{_short_text(state.model, 28)} "
        f"theme={state.theme.name} "
        f"approval={state.approval_mode} stream={_on_off(state.stream_model_output)} "
        f"events={state.events_mode} transcript={state.transcript_style} "
        f"reasoning={_on_off(state.show_reasoning)} "
        f"session={_short_id(state.session_id)} pos={cursor_line}:{cursor_column} "
        f"chars={len(draft_text)} lines={lines} {_context_label(state)} "
        f"{state.task_summary} {_plan_label(state)} "
        f"last={state.last_status}:{_short_id(state.last_run_id) if state.last_run_id else '-'}"
    )


def _format_run_toolbar(state: ReplDisplayState, prompt: str) -> str:
    return (
        f"model={state.active_model_role}:{_short_text(state.model, 24)} "
        f"{_context_label(state)} session={_short_id(state.session_id)} "
        f"{state.task_summary} {_plan_label(state)} chars={len(prompt)} "
        f"lines={_line_count(prompt)}"
    )


def _format_submit_summary(state: ReplDisplayState, prompt: str) -> str:
    return (
        f"submit chars={len(prompt)} lines={_line_count(prompt)} "
        f"model={state.active_model_role}:{_short_text(state.model, 40)} "
        f"session={_short_id(state.session_id)} {_context_label(state)} "
        f"{state.task_summary} {_plan_label(state)}"
    )


def _show_submit_summary(state: ReplDisplayState) -> bool:
    return state.events_mode == "verbose"


def _context_label(state: ReplDisplayState) -> str:
    if state.context_error:
        return f"ctx=error:{_short_text(state.context_error, 28)}"
    status = state.context_status
    if status is None:
        return "ctx=n/a"
    threshold = max(status.threshold_tokens, 1)
    percent = int(status.token_estimate / threshold * 100)
    snapshot = _short_id(status.latest_snapshot_id) if status.latest_snapshot_id else "-"
    raw = ""
    if status.raw_token_estimate is not None and status.raw_token_estimate != status.token_estimate:
        raw = f" raw={status.raw_token_estimate}"
    return (
        f"ctx={status.token_estimate}/{status.threshold_tokens}({percent}%) "
        f"msgs={status.message_count}{raw} snap={snapshot}"
    )


def _plan_label(state: ReplDisplayState) -> str:
    if state.active_plan_id is None:
        return "plan=-"
    status = state.active_plan_status or "unknown"
    return f"plan={status}:{_short_id(state.active_plan_id)}"


def _render_status(console: Console, state: ReplDisplayState) -> None:
    table = Table("Field", "Value")
    rows = {
        "session": state.session_id,
        "model_role": state.active_model_role,
        "model": state.model,
        "approval_mode": state.approval_mode,
        "mode": state.interaction_mode,
        "active_plan": state.active_plan_id or "",
        "active_plan_status": state.active_plan_status or "",
        "theme": state.theme.name,
        "activity_spinner": state.theme.transcript.activity_spinner,
        "composer_mode": "multiline" if state.multiline else "single-line",
        "stream": _on_off(state.stream_model_output),
        "events": state.events_mode,
        "transcript": state.transcript_style,
        "reasoning": _on_off(state.show_reasoning),
        "last_status": state.last_status,
        "last_run": state.last_run_id or "",
        "tasks": state.task_summary,
    }
    for key, value in rows.items():
        table.add_row(key, value)
    if state.context_status is not None:
        for key, value in state.context_status.model_dump(mode="json").items():
            table.add_row(f"context.{key}", str(value))
    elif state.context_error is not None:
        table.add_row("context.error", state.context_error)
    else:
        table.add_row("context", "not configured")
    console.print(table)


def _render_help(console: Console, state: ReplDisplayState | None = None) -> None:
    table = Table("Command", "Current", "Description")
    table.add_row(
        "/model [ROLE]",
        _help_current(state, "model"),
        "Show or switch the active model role.",
    )
    table.add_row("/tools", "", "List currently registered tools.")
    table.add_row(
        "/stream on|off",
        _help_current(state, "stream"),
        "Toggle live assistant token streaming.",
    )
    table.add_row(
        "/events compact|verbose|off",
        _help_current(state, "events"),
        "Control tool/risk/activity event detail.",
    )
    table.add_row(
        "/reasoning on|off",
        _help_current(state, "reasoning"),
        "Toggle provider-supplied reasoning summaries.",
    )
    table.add_row(
        "/transcript comfortable|compact",
        _help_current(state, "transcript"),
        "Switch transcript spacing and blocks.",
    )
    table.add_row(
        "/multiline on|off|toggle",
        _help_current(state, "multiline"),
        "Toggle multiline composer mode.",
    )
    table.add_row(
        "/theme [NAME]",
        _help_current(state, "theme"),
        "Show, preview, switch, save, or reset REPL theme.",
    )
    table.add_row("/repl prefs|save|reset", "", "Show, save, or reset REPL preferences.")
    table.add_row(
        "/status",
        _help_current(state, "status"),
        "Show full REPL, model, session, and context status.",
    )
    table.add_row(
        Text("/tasks [open|all|STATUS]"),
        _help_current(state, "tasks"),
        "Show session task records.",
    )
    table.add_row(
        Text("/plan [on|off|show|approve|execute|list|discard]"),
        _help_current(state, "plan"),
        "Toggle or manage REPL Plan Mode.",
    )
    table.add_row(
        "/context [snapshots|restore ID]",
        _help_current(state, "context"),
        "Inspect or restore context snapshots.",
    )
    table.add_row("/compact", "", "Create a context snapshot for the current session.")
    table.add_row("/clear", "", "Clear the terminal.")
    table.add_row("/exit", "", "Leave the REPL.")
    table.add_row(
        "Esc+Enter",
        _help_current(state, "submit"),
        "Submit the current draft while in multiline mode.",
    )
    console.print(table)


def _help_current(state: ReplDisplayState | None, field: str) -> str:
    if state is None:
        return ""
    if field == "model":
        return f"{state.active_model_role}:{_short_text(state.model, 24)}"
    if field == "stream":
        return _on_off(state.stream_model_output)
    if field == "events":
        return state.events_mode
    if field == "reasoning":
        return _on_off(state.show_reasoning)
    if field == "transcript":
        return state.transcript_style
    if field == "multiline":
        return "multiline" if state.multiline else "single-line"
    if field == "theme":
        return state.theme.name
    if field == "status":
        return f"{state.last_status}:{_short_id(state.last_run_id) if state.last_run_id else '-'}"
    if field == "tasks":
        return state.task_summary
    if field == "plan":
        return f"{state.interaction_mode} {_plan_label(state)}"
    if field == "context":
        return _context_label(state)
    if field == "submit":
        return "Esc+Enter" if state.multiline else "Enter"
    return ""


def _render_themes(
    console: Console,
    current: ReplTheme,
    themes: dict[str, ReplTheme] | None = None,
) -> None:
    available = themes or REPL_THEMES
    table = Table("Theme", "Active", "Prompt", "Toolbar", "Events", "Transcript", "Description")
    descriptions = {
        "default": "Grey composer band with cyan title and caret.",
        "mono": "Plain terminal-friendly styling.",
        "high-contrast": "High-contrast title, badge, caret, and toolbar.",
        "carrot": "Orange-accent theme for tasteful nonsense.",
        "hacker": "Dark terminal palette with green prompt and cyan tool events.",
    }
    for name, theme in available.items():
        table.add_row(
            name,
            "yes" if theme.name == current.name else "",
            _theme_prompt_sample(theme),
            _theme_toolbar_sample(theme),
            _theme_event_sample(theme),
            _theme_transcript_sample(theme),
            descriptions.get(name, "User theme."),
        )
    console.print(table)


def _render_theme_preview(
    console: Console,
    themes: dict[str, ReplTheme],
    names: tuple[str, ...],
) -> None:
    table = Table("Theme", "Prompt", "Toolbar", "Events", "Transcript")
    for name in names:
        theme = _theme_by_name(name, themes)
        table.add_row(
            theme.name,
            _theme_prompt_sample(theme),
            _theme_toolbar_sample(theme),
            _theme_event_sample(theme),
            _theme_transcript_sample(theme),
        )
    console.print(table)


def _render_repl_preferences(console: Console, state: ReplDisplayState) -> None:
    current = _preferences_from_state(state)
    saved = state.saved_preferences
    table = Table("Preference", "Current", "Saved")
    table.add_row("theme", current.theme, saved.theme)
    table.add_row("multiline", str(current.multiline), str(saved.multiline))
    table.add_row(
        "stream_model_output",
        str(current.stream_model_output),
        str(saved.stream_model_output),
    )
    table.add_row("events_mode", current.events_mode, saved.events_mode)
    table.add_row("transcript_style", current.transcript_style, saved.transcript_style)
    table.add_row("show_reasoning", str(current.show_reasoning), str(saved.show_reasoning))
    console.print(table)


def _theme_prompt_sample(theme: ReplTheme) -> Text:
    sample = Text()
    sample.append(" ")
    sample.append(theme.title, style=_rich_style(theme.styles["prompt.title"]))
    sample.append(" SINGLE ", style=_rich_style(theme.styles["prompt.badge"]))
    sample.append(theme.caret, style=_rich_style(theme.styles["prompt.caret"]))
    sample.append(" prompt", style=_rich_style(theme.styles["prompt.model"]))
    return sample


def _theme_toolbar_sample(theme: ReplTheme) -> Text:
    sample = Text()
    sample.append("mode=single ", style=_rich_style(theme.styles["bottom-toolbar"]))
    sample.append(f"theme={theme.name} ", style=_rich_style(theme.styles["bottom-toolbar.key"]))
    sample.append("ctx=120/700", style=_rich_style(theme.styles["bottom-toolbar.warn"]))
    return sample


def _theme_event_sample(theme: ReplTheme) -> Text:
    sample = Text()
    sample.append("thinking", style=theme.trace.thinking)
    sample.append(" ")
    sample.append("tool call", style=theme.trace.tool_call)
    sample.append(" ")
    sample.append("risk assessment", style=theme.trace.risk_assessment)
    sample.append(" ")
    sample.append("done", style=theme.trace.done)
    return sample


def _theme_transcript_sample(theme: ReplTheme) -> Text:
    sample = Text()
    sample.append("you", style=theme.transcript.user)
    sample.append(" ")
    sample.append("agent", style=theme.transcript.assistant)
    sample.append(" ")
    sample.append("tool", style=theme.transcript.tool)
    sample.append(" ")
    sample.append(theme.transcript.activity_spinner, style=theme.transcript.meta)
    return sample


def _rich_style(prompt_toolkit_style: str) -> str:
    tokens = prompt_toolkit_style.split()
    flags: list[str] = []
    foreground: str | None = None
    background: str | None = None
    for token in tokens:
        if token.startswith("bg:"):
            background = token.removeprefix("bg:")
        elif token.startswith("#"):
            foreground = token
        else:
            flags.append(token)
    parts = [*flags]
    if foreground is not None:
        parts.append(foreground)
    if background is not None:
        parts.append(f"on {background}")
    return " ".join(parts)


def _line_count(value: str) -> int:
    return value.count("\n") + 1


def _on_off(value: bool) -> str:
    return "on" if value else "off"


def _short_id(value: str) -> str:
    return value[:8]


def _short_text(value: str, max_chars: int) -> str:
    if len(value) <= max_chars:
        return value
    return f"{value[: max(0, max_chars - 3)]}..."


def _render_tools(console: Console, specs: tuple[ToolSpec, ...]) -> None:
    table = Table("Name", "Filesystem", "Network", "Approval", "Risk", "Description")
    for spec in specs:
        table.add_row(
            spec.name,
            spec.permissions.filesystem,
            spec.permissions.network,
            str(spec.permissions.approval_required or spec.permissions.mutation),
            spec.permissions.risk,
            spec.description,
        )
    console.print(table)


def _render_model(
    console: Console,
    agent: AgentSpec,
    active_model_role: str,
    model_router: ModelRouter | None,
) -> None:
    if model_router is None:
        console.print(f"Model: {agent.model}")
        return
    route = model_router.resolve(active_model_role)
    table = Table("Field", "Value")
    table.add_row("role", route.role)
    table.add_row("profile", route.profile_name)
    table.add_row("provider", route.profile.provider)
    table.add_row("model", route.profile.model)
    table.add_row("base_url", route.profile.base_url or "")
    console.print(table)


async def _handle_tasks_command(
    console: Console,
    task_service: TaskService | None,
    session_id: str,
    argument: str,
) -> None:
    if task_service is None:
        console.print("Task service is not configured.")
        return
    try:
        tasks = await task_service.list_tasks(session_id=session_id)
    except ColossusError as exc:
        console.print(f"Task list failed: {exc}")
        return
    filtered = _filter_tasks(tasks, argument)
    _render_tasks(console, filtered, argument)


def _filter_tasks(tasks: tuple[Task, ...], argument: str) -> tuple[Task, ...]:
    normalized = argument.strip().lower()
    if normalized in {"", "open", "active"}:
        return tuple(task for task in tasks if task.status not in {"completed", "cancelled"})
    if normalized in {"all", "*"}:
        return tasks
    if normalized == "done":
        normalized = "completed"
    if normalized == "in-progress":
        normalized = "in_progress"
    if normalized in _task_statuses():
        return tuple(task for task in tasks if task.status == normalized)
    return tasks


def _render_tasks(console: Console, tasks: tuple[Task, ...], argument: str = "") -> None:
    if not tasks:
        scope = argument.strip() or "open"
        console.print(f"No {scope} tasks.")
        return
    table = Table("State", "Status", "ID", "Title", "Description")
    for task in tasks:
        table.add_row(
            Text(_task_marker(task.status)),
            task.status,
            task.id,
            task.title,
            _short_text(task.description, 72),
        )
    console.print(table)


def _task_marker(status: TaskStatus) -> str:
    if status == "completed":
        return "[x]"
    if status == "blocked":
        return "[!]"
    if status == "in_progress":
        return "[~]"
    if status == "cancelled":
        return "[-]"
    return "[ ]"


def _task_statuses() -> frozenset[str]:
    return frozenset({"pending", "in_progress", "completed", "blocked", "cancelled"})
