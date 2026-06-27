"""Interactive REPL surface."""

import asyncio
import json
import shlex
import tomllib
from collections.abc import Awaitable, Callable, Iterable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Literal, cast
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
from colossus.application.decisions import DecisionService
from colossus.application.defaults import default_agent
from colossus.application.integrations import IntegrationService
from colossus.application.memories import MemoryService
from colossus.application.model_router import ModelRouter
from colossus.application.orchestrator import AgentOrchestrator
from colossus.application.planning import PlanService
from colossus.application.preferences import ReplPreferencesService
from colossus.application.research import ResearchService
from colossus.application.sessions import SessionService
from colossus.application.skills import SkillResolver, extract_skill_mentions
from colossus.application.subagents import SubagentService
from colossus.application.tasks import TaskService
from colossus.domain.agents import AgentSpec
from colossus.domain.context import ContextStatus
from colossus.domain.decisions import KeyDecision
from colossus.domain.errors import ColossusError
from colossus.domain.integrations import (
    IntegrationAuthType,
    IntegrationManifest,
    IntegrationStatusView,
)
from colossus.domain.memories import MemoryItem, MemoryStatus
from colossus.domain.plans import Plan
from colossus.domain.preferences import ReplPreferences, TranscriptStylePreference
from colossus.domain.requests import AgentRunRequest
from colossus.domain.research import ResearchRun, ResearchSource
from colossus.domain.sessions import SessionSummary
from colossus.domain.skills import Skill
from colossus.domain.subagents import SubagentJob, SubagentStatus
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
    "skill",
    "skills",
    "trace",
    "stream",
    "events",
    "reasoning",
    "transcript",
    "multiline",
    "theme",
    "repl",
    "workspace",
    "status",
    "resume",
    "session",
    "sessions",
    "tasks",
    "decision",
    "decisions",
    "memory",
    "memories",
    "agents",
    "plan",
    "research",
    "integrations",
    "help",
    "audit",
    "compact",
    "context",
    "clear",
    "exit",
]
RunStatus = Literal["idle", "running", "done", "failed"]
ReplInteractionMode = Literal["chat", "plan", "research"]

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
    "/skill",
    "/skills",
    "/trace",
    "/stream",
    "/events",
    "/reasoning",
    "/transcript",
    "/multiline",
    "/theme",
    "/repl",
    "/workspace",
    "/status",
    "/resume",
    "/session",
    "/sessions",
    "/tasks",
    "/decision",
    "/decisions",
    "/memory",
    "/memories",
    "/agents",
    "/plan",
    "/research",
    "/integrations",
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
    "/skill": "Toggle or manage Skill Mode.",
    "/skills": "List available skills.",
    "/trace": "Compatibility toggle for compact events.",
    "/stream": "Control assistant output rendering.",
    "/events": "Control tool/risk/activity event detail.",
    "/reasoning": "Toggle provider-supplied reasoning summaries.",
    "/transcript": "Switch transcript spacing and blocks.",
    "/multiline": "Toggle multiline composer mode.",
    "/theme": "Show, preview, switch, save, or reset REPL theme.",
    "/repl": "Show, save, or reset REPL preferences.",
    "/workspace": "Show or switch the active workspace root.",
    "/status": "Show full REPL, model, session, and context status.",
    "/resume": "Choose a persisted session to resume.",
    "/session": "Show, resume, or create a session.",
    "/sessions": "List persisted sessions.",
    "/tasks": "Show session task records.",
    "/decision": "Create, archive, or supersede key decisions.",
    "/decisions": "Show active key decisions.",
    "/memory": "Create, archive, search, or supersede durable memories.",
    "/memories": "Show active memories.",
    "/agents": "Show durable subagent jobs.",
    "/plan": "Toggle or manage REPL Plan Mode.",
    "/research": "Run or inspect Deep Research Mode.",
    "/integrations": "Manage app and service integrations.",
    "/help": "Show REPL commands.",
    "/audit": "Reserved audit view.",
    "/compact": "Create a context snapshot for the current session.",
    "/context": "Inspect or restore context snapshots.",
    "/clear": "Clear the terminal.",
    "/exit": "Leave the REPL.",
}


class SlashCommandCompleter(Completer):
    def __init__(self, skills: SkillResolver | None = None) -> None:
        self._skills = skills

    def get_completions(
        self,
        document: Document,
        complete_event: CompleteEvent,
    ) -> Iterable[Completion]:
        del complete_event
        text = document.text_before_cursor
        if _is_slash_command_draft(text):
            prefix = text.lower()
            for command in SLASH_COMMANDS:
                if command.lower().startswith(prefix):
                    yield Completion(
                        command,
                        start_position=-len(text),
                        display=command,
                        display_meta=SLASH_COMMAND_DESCRIPTIONS.get(command, ""),
                    )
            return
        token = _current_completion_token(text)
        if self._skills is None or token is None or not _is_skill_mention_draft(token):
            return
        prefix = _skill_completion_prefix(token)
        for skill in self._skills.list_skills():
            name = skill.manifest.name
            if name.lower().startswith(prefix):
                completion = f"@skill:{name} "
                yield Completion(
                    completion,
                    start_position=-len(token),
                    display=f"@skill:{name}",
                    display_meta=skill.manifest.description,
                )


def parse_slash_command(value: str) -> ParsedReplCommand | None:
    stripped = value.strip()
    if not stripped.startswith("/"):
        return None
    command, _, argument = stripped[1:].partition(" ")
    if command not in {item[1:] for item in SLASH_COMMANDS}:
        return None
    return ParsedReplCommand(command=command, argument=argument.strip())  # type: ignore[arg-type]


def _current_completion_token(text: str) -> str | None:
    if not text or text[-1].isspace():
        return None
    start = max(text.rfind(" "), text.rfind("\t"), text.rfind("\n")) + 1
    return text[start:]


def _is_skill_mention_draft(token: str) -> bool:
    return token == "@" or token.startswith("@skill:") or (
        token.startswith("@") and ":" not in token and len(token) > 1
    )


def _skill_completion_prefix(token: str) -> str:
    if token in {"@", "@skill", "@skill:"}:
        return ""
    if token.startswith("@skill:"):
        return token.removeprefix("@skill:").lower()
    if token.startswith("@"):
        shorthand = token.removeprefix("@").lower()
        if "skill".startswith(shorthand):
            return ""
        return shorthand
    return token.lower()


@dataclass(frozen=True)
class ReplTheme:
    name: str
    title: str
    caret: str
    continuation: str
    styles: dict[str, str]
    trace: TraceRenderTheme = field(default_factory=TraceRenderTheme)
    transcript: TranscriptRenderTheme = field(default_factory=TranscriptRenderTheme)


@dataclass(frozen=True)
class ReplWorkspaceServices:
    workspace_root: Path
    orchestrator: AgentOrchestrator
    context_service: ContextService | None = None
    research_service: ResearchService | None = None


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
        "research",
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
        "research",
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
    raw_stream_model_output: bool = False
    interaction_mode: ReplInteractionMode = "chat"
    skill_mode_enabled: bool = True
    sticky_skills: tuple[str, ...] = field(default_factory=tuple)
    last_active_skills: tuple[str, ...] = field(default_factory=tuple)
    active_plan_id: str | None = None
    active_plan_status: str | None = None
    active_research_id: str | None = None
    active_research_status: str | None = None
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
    workspace_root: Path = field(default_factory=lambda: Path.cwd().resolve())


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
    decision_service: DecisionService | None = None,
    memory_service: MemoryService | None = None,
    session_service: SessionService | None = None,
    plan_service: PlanService | None = None,
    subagent_service: SubagentService | None = None,
    research_service: ResearchService | None = None,
    integration_service: IntegrationService | None = None,
    model_router: ModelRouter | None = None,
    active_model_role: str = "primary",
    orchestrator_factory: Callable[[str], AgentOrchestrator] | None = None,
    integration_refresh_factory: Callable[[str], Awaitable[AgentOrchestrator]] | None = None,
    workspace_factory: Callable[[Path, str], ReplWorkspaceServices] | None = None,
    context_model: str | None = None,
    approval_mode: str = "ask",
    history_path: Path | None = None,
    theme_name: str | None = None,
    preferences_service: ReplPreferencesService | None = None,
    initial_session_id: str | None = None,
    resume_latest: bool = False,
    repo_root: Path | None = None,
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
        completer=SlashCommandCompleter(skills),
        complete_while_typing=True,
        reserve_space_for_menu=8,
        history=FileHistory(str(history_path)) if history_path is not None else None,
        erase_when_done=True,
    )
    agent = agent or default_agent()
    resumed_session: SessionSummary | None = None
    if resume_latest:
        if session_service is None:
            raise ColossusError("Session service is not configured.")
        resumed_session = await session_service.latest_session()
        active_session_id = resumed_session.id
    else:
        active_session_id = initial_session_id or (
            session_service.new_session_id() if session_service is not None else str(uuid4())
        )
        if initial_session_id is not None and session_service is not None:
            resumed_session = await session_service.get_session(initial_session_id)
    display_state = ReplDisplayState(
        session_id=active_session_id,
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
        workspace_root=(repo_root or Path.cwd()).resolve(),
    )
    trace_renderer = TranscriptRenderer(
        console,
        events_mode=display_state.events_mode,
        stream_model_output=display_state.stream_model_output,
        render_streamed_markdown=not display_state.raw_stream_model_output,
        show_reasoning=display_state.show_reasoning,
        transcript_style=display_state.transcript_style,
        theme=display_state.theme.transcript,
    )
    orchestrator.set_event_observer(trace_renderer.render)
    if research_service is not None:
        research_service.set_event_observer(trace_renderer.render)
    if subagent_service is not None:
        await subagent_service.start()
    _render_repl_startup(console, display_state)
    if resumed_session is not None:
        _render_resumed_session(console, resumed_session)
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
            if command.command == "skill":
                _handle_skill_command(console, skills, display_state, agent, command.argument)
                continue
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
                stream_enabled, raw_stream = _stream_output_mode(command.argument, display_state)
                display_state.stream_model_output = stream_enabled
                display_state.raw_stream_model_output = raw_stream
                trace_renderer.stream_model_output = stream_enabled
                trace_renderer.render_streamed_markdown = not raw_stream
                console.print(f"Assistant output is {_stream_mode_label(display_state)}.")
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
            if command.command == "workspace":
                services = _handle_workspace_command(
                    console,
                    display_state,
                    command.argument,
                    workspace_factory,
                    display_state.active_model_role,
                )
                if services is not None:
                    orchestrator = services.orchestrator
                    context_service = services.context_service
                    research_service = services.research_service
                    orchestrator.set_event_observer(trace_renderer.render)
                    if research_service is not None:
                        research_service.set_event_observer(trace_renderer.render)
                    await _refresh_context_status(display_state, context_service)
                    await _refresh_task_status(display_state, task_service)
                continue
            if command.command == "status":
                await _refresh_context_status(display_state, context_service)
                await _refresh_task_status(display_state, task_service)
                _render_status(console, display_state)
                continue
            if command.command == "resume":
                await _handle_resume_command(
                    console,
                    session_service,
                    display_state,
                    command.argument,
                    context_service,
                    task_service,
                )
                continue
            if command.command == "sessions":
                await _handle_sessions_command(console, session_service, command.argument)
                continue
            if command.command == "session":
                await _handle_session_command(
                    console,
                    session_service,
                    display_state,
                    command.argument,
                    context_service,
                    task_service,
                )
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
            if command.command == "decision":
                await _handle_decision_command(
                    console,
                    decision_service,
                    display_state.session_id,
                    command.argument,
                )
                continue
            if command.command == "decisions":
                await _handle_decisions_command(
                    console,
                    decision_service,
                    display_state.session_id,
                    command.argument,
                )
                continue
            if command.command == "memory":
                await _handle_memory_command(
                    console,
                    memory_service,
                    display_state.session_id,
                    display_state.workspace_root,
                    command.argument,
                )
                continue
            if command.command == "memories":
                await _handle_memories_command(
                    console,
                    memory_service,
                    display_state.session_id,
                    display_state.workspace_root,
                    command.argument,
                )
                continue
            if command.command == "agents":
                await _handle_agents_command(
                    console,
                    subagent_service,
                    display_state.session_id,
                    command.argument,
                )
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
            if command.command == "research":
                await _handle_research_command(
                    console,
                    research_service,
                    display_state,
                    command.argument,
                    trace_renderer,
                )
                continue
            if command.command == "integrations":
                changed = await _handle_integrations_command(
                    console,
                    integration_service,
                    command.argument,
                )
                if changed and integration_refresh_factory is not None:
                    orchestrator = await integration_refresh_factory(
                        display_state.active_model_role
                    )
                    orchestrator.set_event_observer(trace_renderer.render)
                    console.print("Integration tool catalog refreshed.")
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
                        skill_mode_enabled=display_state.skill_mode_enabled,
                        active_skills=_sticky_skills_for_request(display_state),
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
                display_state.last_active_skills = _active_skill_names_for_prompt(
                    display_state,
                    skills,
                    line,
                )
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
            if display_state.interaction_mode == "research":
                await _run_research_query(
                    console,
                    research_service,
                    display_state,
                    line,
                    trace_renderer,
                    started=True,
                    render_prompt=False,
                )
                await _refresh_context_status(display_state, context_service)
                await _refresh_task_status(display_state, task_service)
                continue
            result = await orchestrator.run(
                AgentRunRequest(
                    prompt=line,
                    agent=agent,
                    session_id=display_state.session_id,
                    skill_mode_enabled=display_state.skill_mode_enabled,
                    active_skills=_sticky_skills_for_request(display_state),
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
        display_state.last_active_skills = _active_skill_names_for_prompt(
            display_state,
            skills,
            line,
        )
        await _refresh_context_status(display_state, context_service)
        await _refresh_task_status(display_state, task_service)
        if not trace_renderer.rendered_model_output:
            trace_renderer.render_final_answer(result.final_output)
        if not trace_renderer.rendered_model_output:
            trace_renderer.render_empty_response()


async def _handle_resume_command(
    console: Console,
    session_service: SessionService | None,
    state: ReplDisplayState,
    argument: str,
    context_service: ContextService | None,
    task_service: TaskService | None,
    prompt_handler: RichUserPromptHandler | None = None,
) -> None:
    if session_service is None:
        console.print("Session service is not configured.")
        return
    try:
        limit = int(argument.strip()) if argument.strip() else 10
    except ValueError:
        console.print("Use /resume [LIMIT].")
        return
    sessions = await session_service.list_sessions(limit=limit)
    if not sessions:
        console.print("No sessions.")
        return
    _render_sessions(console, sessions)
    handler = prompt_handler or RichUserPromptHandler(console)
    answer = await handler.ask(
        question="Choose a session to resume.",
        choices=tuple(_resume_choice(session) for session in sessions),
        allow_freeform=False,
    )
    if answer.choice_id is None:
        console.print("No session selected.")
        return
    try:
        session = await session_service.require_session(answer.choice_id)
    except ColossusError as exc:
        console.print(f"[red]{exc}[/red]")
        return
    await _activate_repl_session(state, session.id, context_service, task_service)
    _render_resumed_session(console, session)


async def _handle_sessions_command(
    console: Console,
    session_service: SessionService | None,
    argument: str,
) -> None:
    if session_service is None:
        console.print("Session service is not configured.")
        return
    try:
        limit = int(argument.strip()) if argument.strip() else 10
    except ValueError:
        console.print("Use /sessions [LIMIT].")
        return
    _render_sessions(console, await session_service.list_sessions(limit=limit))


async def _handle_session_command(
    console: Console,
    session_service: SessionService | None,
    state: ReplDisplayState,
    argument: str,
    context_service: ContextService | None,
    task_service: TaskService | None,
) -> None:
    if session_service is None:
        console.print("Session service is not configured.")
        return
    parts = argument.split(maxsplit=2)
    action = parts[0] if parts else "show"
    try:
        if action in {"", "show"}:
            target_session_id = parts[1] if len(parts) > 1 else state.session_id
            session = await session_service.get_session(target_session_id)
            if session is None:
                console.print(f"Session {target_session_id} has no persisted messages yet.")
                return
            _render_session_summary(console, session)
            return
        if action == "resume":
            if len(parts) < 2:
                console.print("Use /session resume SESSION_ID.")
                return
            session = await session_service.require_session(parts[1])
            await _activate_repl_session(state, session.id, context_service, task_service)
            _render_resumed_session(console, session)
            return
        if action == "latest":
            session = await session_service.latest_session()
            await _activate_repl_session(state, session.id, context_service, task_service)
            _render_resumed_session(console, session)
            return
        if action == "new":
            new_session_id = session_service.new_session_id()
            await _activate_repl_session(state, new_session_id, context_service, task_service)
            console.print(f"Started new session {new_session_id}.")
            return
    except ColossusError as exc:
        console.print(f"[red]{exc}[/red]")
        return
    console.print("Use /session show [ID], /session resume ID, /session latest, or /session new.")


async def _activate_repl_session(
    state: ReplDisplayState,
    session_id: str,
    context_service: ContextService | None,
    task_service: TaskService | None,
) -> None:
    state.session_id = session_id
    state.active_plan_id = None
    state.active_plan_status = None
    state.last_run_id = None
    state.last_status = "idle"
    await _refresh_context_status(state, context_service)
    await _refresh_task_status(state, task_service)


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
    decision_service: DecisionService | None = None,
    memory_service: MemoryService | None = None,
    session_service: SessionService | None = None,
    plan_service: PlanService | None = None,
    subagent_service: SubagentService | None = None,
    research_service: ResearchService | None = None,
    integration_service: IntegrationService | None = None,
    model_router: ModelRouter | None = None,
    active_model_role: str = "primary",
    orchestrator_factory: Callable[[str], AgentOrchestrator] | None = None,
    integration_refresh_factory: Callable[[str], Awaitable[AgentOrchestrator]] | None = None,
    workspace_factory: Callable[[Path, str], ReplWorkspaceServices] | None = None,
    context_model: str | None = None,
    approval_mode: str = "ask",
    history_path: Path | None = None,
    theme_name: str | None = None,
    preferences_service: ReplPreferencesService | None = None,
    initial_session_id: str | None = None,
    resume_latest: bool = False,
    repo_root: Path | None = None,
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
            decision_service=decision_service,
            memory_service=memory_service,
            session_service=session_service,
            plan_service=plan_service,
            subagent_service=subagent_service,
            research_service=research_service,
            integration_service=integration_service,
            model_router=model_router,
            active_model_role=active_model_role,
            orchestrator_factory=orchestrator_factory,
            integration_refresh_factory=integration_refresh_factory,
            workspace_factory=workspace_factory,
            context_model=context_model,
            approval_mode=approval_mode,
            history_path=history_path,
            theme_name=theme_name,
            preferences_service=preferences_service,
            initial_session_id=initial_session_id,
            resume_latest=resume_latest,
            repo_root=repo_root,
            theme_dirs=theme_dirs,
        )
    )


def _render_repl_startup(console: Console, state: ReplDisplayState) -> None:
    console.clear()
    console.print("[bold]Colossus REPL[/bold]  Type /exit to leave.")
    console.print(
        f"[dim]session_id={state.session_id} "
        f"workspace={state.workspace_root} "
        f"mode={state.interaction_mode} "
        f"composer={'multi' if state.multiline else 'single'} "
        f"theme={state.theme.name} stream={_stream_mode_label(state)} "
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


def _stream_output_mode(argument: str, state: ReplDisplayState) -> tuple[bool, bool]:
    normalized = argument.strip().lower()
    if normalized in {"on", "markdown", "buffer", "buffered", "final"}:
        return True, False
    if normalized in {"raw", "live"}:
        return True, True
    if normalized in {"off", "false", "0", "no"}:
        return False, False
    if state.stream_model_output:
        return False, False
    return True, False


def _stream_mode_label(state: ReplDisplayState) -> str:
    if not state.stream_model_output:
        return "off"
    if state.raw_stream_model_output:
        return "raw"
    return "markdown"


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
    state.raw_stream_model_output = False
    state.events_mode = preferences.events_mode
    state.show_reasoning = preferences.show_reasoning
    state.transcript_style = preferences.transcript_style


def _sync_renderer(renderer: TranscriptRenderer, state: ReplDisplayState) -> None:
    renderer.events_mode = state.events_mode
    renderer.stream_model_output = state.stream_model_output
    renderer.render_streamed_markdown = not state.raw_stream_model_output
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


async def _handle_integrations_command(
    console: Console,
    integration_service: IntegrationService | None,
    argument: str,
) -> bool:
    if integration_service is None:
        console.print("Integration service is not configured.")
        return False
    try:
        parts = shlex.split(argument)
    except ValueError as exc:
        console.print(f"Invalid /integrations command: {exc}")
        return False
    action = parts[0] if parts else "list"
    try:
        if action in {"", "list"}:
            _render_integration_statuses(console, await integration_service.list_statuses())
            return False
        if action == "show":
            if len(parts) != 2:
                console.print("Use /integrations show NAME.")
                return False
            manifest = await integration_service.get_manifest(parts[1])
            connection = await integration_service.get_connection(manifest.name)
            status = connection.status if connection is not None else None
            _render_integration_manifest(console, manifest, status)
            return False
        if action == "connect":
            name, credential_ref, scopes = _parse_repl_integration_connect(parts)
            connection = await integration_service.connect(
                name,
                credential_ref=credential_ref,
                scopes=scopes,
            )
            _render_repl_integration_connection(console, connection.name, connection.status)
            return connection.status == "connected"
        if action == "disconnect":
            if len(parts) != 2:
                console.print("Use /integrations disconnect NAME.")
                return False
            await integration_service.disconnect(parts[1])
            console.print(f"Disconnected integration {parts[1]}.")
            return True
        if action == "import-openapi":
            name, spec_path, base_url, credential_ref, auth_type = _parse_repl_openapi_import(parts)
            connection = await integration_service.import_openapi(
                name,
                spec_path=spec_path,
                base_url=base_url,
                credential_ref=credential_ref,
                auth_type=auth_type,
            )
            _render_repl_integration_connection(console, connection.name, connection.status)
            return connection.status == "connected"
    except ColossusError as exc:
        console.print(f"[red]{exc}[/red]")
        return False
    console.print(
        "Use /integrations list, show NAME, connect NAME [--credential-ref REF], "
        "disconnect NAME, or import-openapi NAME SPEC."
    )
    return False


def _parse_repl_integration_connect(
    parts: list[str],
) -> tuple[str, str | None, tuple[str, ...]]:
    if len(parts) < 2:
        raise ColossusError("Use /integrations connect NAME [--credential-ref REF].")
    name = parts[1]
    credential_ref: str | None = None
    scopes: list[str] = []
    index = 2
    while index < len(parts):
        token = parts[index]
        if token == "--credential-ref":
            index += 1
            if index >= len(parts):
                raise ColossusError("--credential-ref requires a value.")
            credential_ref = parts[index]
        elif token == "--scope":
            index += 1
            if index >= len(parts):
                raise ColossusError("--scope requires a value.")
            scopes.append(parts[index])
        elif credential_ref is None:
            credential_ref = token
        else:
            raise ColossusError(f"Unknown integration connect argument: {token}")
        index += 1
    return name, credential_ref, tuple(scopes)


def _parse_repl_openapi_import(
    parts: list[str],
) -> tuple[str, Path, str | None, str | None, IntegrationAuthType]:
    if len(parts) < 3:
        raise ColossusError("Use /integrations import-openapi NAME SPEC_PATH.")
    name = parts[1]
    spec_path = Path(parts[2]).expanduser()
    base_url: str | None = None
    credential_ref: str | None = None
    auth_type: IntegrationAuthType = "bearer"
    index = 3
    while index < len(parts):
        token = parts[index]
        index += 1
        if token not in {"--base-url", "--credential-ref", "--auth-type"}:
            raise ColossusError(f"Unknown OpenAPI import argument: {token}")
        if index >= len(parts):
            raise ColossusError(f"{token} requires a value.")
        value = parts[index]
        index += 1
        if token == "--base-url":
            base_url = value
        elif token == "--credential-ref":
            credential_ref = value
        elif token == "--auth-type":
            auth_type = _repl_integration_auth_type(value)
    return name, spec_path, base_url, credential_ref, auth_type


def _repl_integration_auth_type(value: str) -> IntegrationAuthType:
    normalized = value.strip().lower().replace("-", "_")
    if normalized not in {
        "none",
        "api_key",
        "bearer",
        "oauth2_authorization_code",
        "service_account",
    }:
        raise ColossusError(
            "Auth type must be none, api-key, bearer, "
            "oauth2-authorization-code, or service-account."
        )
    return cast(IntegrationAuthType, normalized)


def _handle_workspace_command(
    console: Console,
    state: ReplDisplayState,
    argument: str,
    workspace_factory: Callable[[Path, str], ReplWorkspaceServices] | None,
    active_model_role: str,
) -> ReplWorkspaceServices | None:
    normalized = argument.strip()
    if not normalized or normalized.lower() == "show":
        _render_workspace(console, state.workspace_root)
        return None
    if workspace_factory is None:
        console.print("Workspace switching is not configured.")
        return None
    try:
        workspace_root = _resolve_workspace_argument(normalized, state.workspace_root)
        services = workspace_factory(workspace_root, active_model_role)
    except ColossusError as exc:
        console.print(f"[red]Workspace switch failed:[/red] {exc}")
        return None
    state.workspace_root = services.workspace_root.resolve()
    state.context_status = None
    state.context_error = None
    console.print(f"Workspace set to {state.workspace_root}")
    return services


def _resolve_workspace_argument(value: str, current_root: Path) -> Path:
    candidate = Path(value).expanduser()
    if not candidate.is_absolute():
        candidate = current_root / candidate
    resolved = candidate.resolve()
    if not resolved.exists():
        raise ColossusError(f"Workspace does not exist: {resolved}")
    if not resolved.is_dir():
        raise ColossusError(f"Workspace is not a directory: {resolved}")
    return resolved


def _render_workspace(console: Console, workspace_root: Path) -> None:
    table = Table("Field", "Value")
    table.add_row("workspace", str(workspace_root))
    console.print(table)


def _handle_skill_command(
    console: Console,
    skills: SkillResolver,
    state: ReplDisplayState,
    agent: AgentSpec,
    argument: str,
) -> None:
    action, _, rest = argument.strip().partition(" ")
    normalized = action.lower()
    if normalized in {"", "show"}:
        target = rest.strip()
        if target:
            skill = _available_repl_skill(skills, agent, target)
            if skill is None:
                console.print(f"Skill is not available: {target}")
                return
            _render_skill_detail(console, skill)
            return
        _render_skill_status(console, skills, state, agent)
        return
    if normalized == "on":
        state.skill_mode_enabled = True
        console.print("Skill Mode is on.")
        return
    if normalized == "off":
        state.skill_mode_enabled = False
        console.print("Skill Mode is off.")
        return
    if normalized == "use":
        name = rest.strip()
        if not name:
            console.print("Use /skill use NAME.")
            return
        skill = _available_repl_skill(skills, agent, name)
        if skill is None:
            console.print(f"Skill is not available: {name}")
            return
        state.sticky_skills = _dedupe_skill_names((*state.sticky_skills, skill.manifest.name))
        console.print(f"Sticky skill added: {skill.manifest.name}")
        return
    if normalized == "drop":
        name = rest.strip()
        if not name:
            console.print("Use /skill drop NAME.")
            return
        before = state.sticky_skills
        state.sticky_skills = tuple(skill for skill in state.sticky_skills if skill != name)
        if before == state.sticky_skills:
            console.print(f"Sticky skill was not active: {name}")
        else:
            console.print(f"Sticky skill dropped: {name}")
        return
    if normalized == "clear":
        state.sticky_skills = ()
        console.print("Sticky skills cleared.")
        return
    console.print("Use /skill [on|off|show|use NAME|drop NAME|clear].")


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


async def _handle_research_command(
    console: Console,
    research_service: ResearchService | None,
    state: ReplDisplayState,
    argument: str,
    trace_renderer: TranscriptRenderer,
) -> None:
    if research_service is None:
        console.print("Research service is not configured.")
        return
    action, _, rest = argument.strip().partition(" ")
    normalized = action.lower()
    if normalized in {"", "toggle"}:
        state.interaction_mode = (
            "chat" if state.interaction_mode == "research" else "research"
        )
        console.print(f"Research Mode is {state.interaction_mode}.")
        return
    if normalized == "on":
        state.interaction_mode = "research"
        console.print("Research Mode is on.")
        return
    if normalized == "off":
        state.interaction_mode = "chat"
        console.print("Research Mode is off.")
        return
    if normalized == "list":
        _render_research_runs(
            console,
            await research_service.list_runs(session_id=state.session_id),
            active_research_id=state.active_research_id,
        )
        return
    if normalized == "show":
        run = await _active_research_run(console, research_service, state, rest.strip())
        if run is not None:
            console.print(Markdown(run.report or "No report saved."))
        return
    if normalized == "sources":
        run = await _active_research_run(console, research_service, state, rest.strip())
        if run is not None:
            _render_research_sources(console, await research_service.list_sources(run.id))
        return
    await _run_research_query(
        console,
        research_service,
        state,
        argument,
        trace_renderer,
    )


async def _run_research_query(
    console: Console,
    research_service: ResearchService | None,
    state: ReplDisplayState,
    question: str,
    trace_renderer: TranscriptRenderer,
    *,
    started: bool = False,
    render_prompt: bool = True,
) -> None:
    if research_service is None:
        if started:
            trace_renderer.end_run()
        console.print("Research service is not configured.")
        return
    if render_prompt:
        trace_renderer.render_user_prompt(question)
    if not started:
        trace_renderer.begin_run(activity_context=_format_run_toolbar(state, question))
    state.last_status = "running"
    try:
        run = await research_service.run(
            question=question,
            session_id=state.session_id,
        )
    except ColossusError as exc:
        trace_renderer.end_run()
        state.last_status = "failed"
        console.print(f"[red]Research failed:[/red] {exc}")
        return
    trace_renderer.end_run()
    state.last_run_id = run.id
    state.last_status = "done"
    state.active_research_id = run.id
    state.active_research_status = run.status
    trace_renderer.render_final_answer(run.report)


async def _active_research_run(
    console: Console,
    research_service: ResearchService,
    state: ReplDisplayState,
    run_id: str = "",
) -> ResearchRun | None:
    run: ResearchRun | None
    try:
        if run_id:
            run = await research_service.get_run(run_id)
        elif state.active_research_id is not None:
            run = await research_service.get_run(state.active_research_id)
        else:
            run = await research_service.latest_run(state.session_id)
    except ColossusError as exc:
        console.print(f"Research run is unavailable: {exc}")
        return None
    if run is None:
        console.print("No research run.")
        return None
    state.active_research_id = run.id
    state.active_research_status = run.status
    return run


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
                skill_mode_enabled=state.skill_mode_enabled,
                active_skills=_sticky_skills_for_request(state),
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
    state.last_active_skills = _active_skill_names_for_prompt(state, None, prompt)
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

    @bindings.add("@")
    def _skill_at(event) -> None:  # type: ignore[no-untyped-def]
        event.current_buffer.insert_text("@")
        if _current_buffer_is_skill_mention_draft():
            event.current_buffer.start_completion(select_first=False)

    for key in "abcdefghijklmnopqrstuvwxyz":
        _bind_slash_completion_key(bindings, key)
        _bind_skill_completion_key(bindings, key)

    for key in "0123456789:-.":
        _bind_skill_completion_key(bindings, key)

    @bindings.add("escape", "enter")
    def _accept(event) -> None:  # type: ignore[no-untyped-def]
        event.current_buffer.validate_and_handle()

    return bindings


def _bind_slash_completion_key(bindings: KeyBindings, key: str) -> None:
    @bindings.add(key, filter=Condition(_current_buffer_is_slash_command_draft))
    def _slash_command_key(event) -> None:  # type: ignore[no-untyped-def]
        event.current_buffer.insert_text(event.data)
        event.current_buffer.start_completion(select_first=False)


def _bind_skill_completion_key(bindings: KeyBindings, key: str) -> None:
    @bindings.add(key, filter=Condition(_current_buffer_is_skill_mention_draft))
    def _skill_mention_key(event) -> None:  # type: ignore[no-untyped-def]
        event.current_buffer.insert_text(event.data)
        event.current_buffer.start_completion(select_first=False)


def _current_buffer_is_slash_command_draft() -> bool:
    app = get_app_or_none()
    if app is None:
        return False
    return _is_slash_command_draft(app.current_buffer.document.text_before_cursor)


def _current_buffer_is_skill_mention_draft() -> bool:
    app = get_app_or_none()
    if app is None:
        return False
    token = _current_completion_token(app.current_buffer.document.text_before_cursor)
    return token is not None and _is_skill_mention_draft(token)


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
    mode = (
        state.interaction_mode
        if state.interaction_mode in {"plan", "research"}
        else ("multi" if state.multiline else "single")
    )
    lines = _line_count(draft_text)
    return (
        f"mode={mode} model={state.active_model_role}:{_short_text(state.model, 28)} "
        f"theme={state.theme.name} "
        f"approval={state.approval_mode} stream={_stream_mode_label(state)} "
        f"events={state.events_mode} transcript={state.transcript_style} "
        f"reasoning={_on_off(state.show_reasoning)} "
        f"session={_short_id(state.session_id)} pos={cursor_line}:{cursor_column} "
        f"chars={len(draft_text)} lines={lines} {_context_label(state)} "
        f"{state.task_summary} {_plan_label(state)} {_research_label(state)} {_skill_label(state)} "
        f"last={state.last_status}:{_short_id(state.last_run_id) if state.last_run_id else '-'}"
    )


def _format_run_toolbar(state: ReplDisplayState, prompt: str) -> str:
    return (
        f"model={state.active_model_role}:{_short_text(state.model, 24)} "
        f"{_context_label(state)} session={_short_id(state.session_id)} "
        f"{state.task_summary} {_plan_label(state)} {_research_label(state)} {_skill_label(state)} "
        f"chars={len(prompt)} "
        f"lines={_line_count(prompt)}"
    )


def _format_submit_summary(state: ReplDisplayState, prompt: str) -> str:
    return (
        f"submit chars={len(prompt)} lines={_line_count(prompt)} "
        f"model={state.active_model_role}:{_short_text(state.model, 40)} "
        f"session={_short_id(state.session_id)} {_context_label(state)} "
        f"{state.task_summary} {_plan_label(state)} {_research_label(state)} {_skill_label(state)}"
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


def _research_label(state: ReplDisplayState) -> str:
    if state.active_research_id is None:
        return "research=-"
    status = state.active_research_status or "unknown"
    return f"research={status}:{_short_id(state.active_research_id)}"


def _skill_label(state: ReplDisplayState) -> str:
    mode = "on" if state.skill_mode_enabled else "off"
    sticky = _format_skill_names(state.sticky_skills)
    return f"skills={mode}:{sticky}"


def _format_skill_names(names: tuple[str, ...]) -> str:
    return ",".join(names) if names else "-"


def _sticky_skills_for_request(state: ReplDisplayState) -> tuple[str, ...]:
    return state.sticky_skills if state.skill_mode_enabled else ()


def _active_skill_names_for_prompt(
    state: ReplDisplayState,
    skills: SkillResolver | None,
    prompt: str,
) -> tuple[str, ...]:
    if not state.skill_mode_enabled:
        return ()
    mentioned: tuple[str, ...] = ()
    if skills is not None:
        available_names = tuple(skill.manifest.name for skill in skills.list_skills())
        mentioned = extract_skill_mentions(prompt, available_names=available_names)
    return _dedupe_skill_names((*state.sticky_skills, *mentioned))


def _available_repl_skill(
    skills: SkillResolver,
    agent: AgentSpec,
    name: str,
) -> Skill | None:
    for skill in _available_repl_skills(skills, agent):
        if skill.manifest.name == name:
            return skill
    return None


def _available_repl_skills(skills: SkillResolver, agent: AgentSpec) -> tuple[Skill, ...]:
    by_name = {skill.manifest.name: skill for skill in skills.list_skills()}
    if not agent.skills:
        return tuple(by_name.values())
    return tuple(by_name[name] for name in _dedupe_skill_names(agent.skills) if name in by_name)


def _dedupe_skill_names(names: tuple[str, ...]) -> tuple[str, ...]:
    seen: set[str] = set()
    deduped: list[str] = []
    for name in names:
        if name and name not in seen:
            seen.add(name)
            deduped.append(name)
    return tuple(deduped)


def _resume_choice(session: SessionSummary) -> UserPromptChoice:
    title = session.title or session.last_user_preview or "untitled session"
    preview = session.last_user_preview or "no user messages yet"
    return UserPromptChoice(
        id=session.id,
        label=f"{_short_id(session.id)} {title}",
        description=(
            f"messages={session.message_count} updated={session.updated_at} "
            f"last_user={preview}"
        ),
    )


def _render_resumed_session(console: Console, session: SessionSummary) -> None:
    preview = f" last_user={session.last_user_preview}" if session.last_user_preview else ""
    console.print(
        f"Resumed session {session.id} "
        f"messages={session.message_count} updated={session.updated_at}{preview}",
        markup=False,
    )


def _render_sessions(console: Console, sessions: tuple[SessionSummary, ...]) -> None:
    if not sessions:
        console.print("No sessions.")
        return
    table = Table("Updated", "Messages", "ID", "Title", "Last User")
    for session in sessions:
        table.add_row(
            session.updated_at,
            str(session.message_count),
            session.id,
            session.title or "",
            session.last_user_preview or "",
        )
    console.print(table)


def _render_research_runs(
    console: Console,
    runs: tuple[ResearchRun, ...],
    *,
    active_research_id: str | None = None,
) -> None:
    if not runs:
        console.print("No research runs.")
        return
    table = Table("Active", "Status", "ID", "Depth", "Question", "Updated")
    for run in runs:
        table.add_row(
            "*" if run.id == active_research_id else "",
            run.status,
            run.id,
            run.depth,
            _short_text(run.question, 60),
            run.updated_at,
        )
    console.print(table)


def _render_research_sources(
    console: Console,
    sources: tuple[ResearchSource, ...],
) -> None:
    if not sources:
        console.print("No research sources.")
        return
    table = Table("Label", "Type", "Title", "URI")
    for source in sources:
        table.add_row(
            f"[{source.label}]",
            source.kind,
            _short_text(source.title, 52),
            _short_text(source.uri, 70),
        )
    console.print(table)


def _render_session_summary(console: Console, session: SessionSummary) -> None:
    table = Table("Field", "Value")
    table.add_row("id", session.id)
    table.add_row("title", session.title or "")
    table.add_row("created_at", session.created_at)
    table.add_row("updated_at", session.updated_at)
    table.add_row("message_count", str(session.message_count))
    table.add_row("last_run_id", session.last_run_id or "")
    table.add_row("last_user_preview", session.last_user_preview or "")
    console.print(table)


def _render_status(console: Console, state: ReplDisplayState) -> None:
    table = Table("Field", "Value")
    rows = {
        "session": state.session_id,
        "workspace": str(state.workspace_root),
        "model_role": state.active_model_role,
        "model": state.model,
        "approval_mode": state.approval_mode,
        "mode": state.interaction_mode,
        "skill_mode": _on_off(state.skill_mode_enabled),
        "sticky_skills": _format_skill_names(state.sticky_skills),
        "last_active_skills": _format_skill_names(state.last_active_skills),
        "active_plan": state.active_plan_id or "",
        "active_plan_status": state.active_plan_status or "",
        "active_research": state.active_research_id or "",
        "active_research_status": state.active_research_status or "",
        "theme": state.theme.name,
        "activity_spinner": state.theme.transcript.activity_spinner,
        "composer_mode": "multiline" if state.multiline else "single-line",
        "stream": _stream_mode_label(state),
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


def _render_skill_status(
    console: Console,
    skills: SkillResolver,
    state: ReplDisplayState,
    agent: AgentSpec,
) -> None:
    available = _available_repl_skills(skills, agent)
    table = Table("Field", "Value")
    table.add_row("mode", _on_off(state.skill_mode_enabled))
    table.add_row("sticky", _format_skill_names(state.sticky_skills))
    table.add_row("last_active", _format_skill_names(state.last_active_skills))
    table.add_row("available_count", str(len(available)))
    table.add_row(
        "available",
        _format_skill_names(tuple(skill.manifest.name for skill in available)),
    )
    console.print(table)


def _render_skill_detail(console: Console, skill: Skill) -> None:
    table = Table("Field", "Value")
    manifest = skill.manifest
    table.add_row("name", manifest.name)
    table.add_row("version", manifest.version)
    table.add_row("description", manifest.description)
    table.add_row("required_tools", ", ".join(manifest.required_tools) or "-")
    table.add_row("permissions", ", ".join(manifest.permissions) or "-")
    table.add_row("offline", str(manifest.offline_compatible))
    table.add_row("source", skill.source)
    table.add_row("preview", _short_text(skill.instructions.strip(), 260))
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
        "/stream on|raw|off",
        _help_current(state, "stream"),
        "Control assistant output rendering.",
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
        "/workspace [PATH]",
        _help_current(state, "workspace"),
        "Show or switch the active workspace root.",
    )
    table.add_row(
        "/status",
        _help_current(state, "status"),
        "Show full REPL, model, session, and context status.",
    )
    table.add_row(
        "/resume [LIMIT]",
        _help_current(state, "session"),
        "Choose a persisted session to resume.",
    )
    table.add_row("/sessions [LIMIT]", "", "List persisted sessions.")
    table.add_row(
        "/session show|resume|latest|new",
        _help_current(state, "session"),
        "Inspect or switch the active session.",
    )
    table.add_row(
        Text("/tasks [open|all|STATUS]"),
        _help_current(state, "tasks"),
        "Show session task records.",
    )
    table.add_row(
        Text("/decisions [all|STATUS]"),
        _help_current(state, "decisions"),
        "Show key decisions.",
    )
    table.add_row(
        Text("/decision [archive|supersede|TEXT]"),
        _help_current(state, "decision"),
        "Create or update key decisions.",
    )
    table.add_row(
        Text("/memories [all|STATUS]"),
        _help_current(state, "memories"),
        "Show durable memories.",
    )
    table.add_row(
        Text("/memory [archive|search|supersede|TEXT]"),
        _help_current(state, "memory"),
        "Create or update durable memories.",
    )
    table.add_row(
        Text("/plan [on|off|show|approve|execute|list|discard]"),
        _help_current(state, "plan"),
        "Toggle or manage REPL Plan Mode.",
    )
    table.add_row(
        Text("/research [on|off|show|sources|QUESTION]"),
        _help_current(state, "research"),
        "Run or inspect Deep Research Mode.",
    )
    table.add_row(
        Text("/integrations [list|show|connect|disconnect|import-openapi]"),
        "",
        "Manage app and service integrations.",
    )
    table.add_row(
        Text("/skill [on|off|show|use|drop|clear]"),
        _help_current(state, "skill"),
        "Toggle or manage Skill Mode.",
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
        return _stream_mode_label(state)
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
    if field == "workspace":
        return _short_text(str(state.workspace_root), 28)
    if field == "status":
        return f"{state.last_status}:{_short_id(state.last_run_id) if state.last_run_id else '-'}"
    if field == "session":
        return _short_id(state.session_id)
    if field == "tasks":
        return state.task_summary
    if field == "plan":
        return f"{state.interaction_mode} {_plan_label(state)}"
    if field == "research":
        return f"{state.interaction_mode} {_research_label(state)}"
    if field == "skill":
        return _skill_label(state)
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


def _render_integration_statuses(
    console: Console,
    statuses: tuple[IntegrationStatusView, ...],
) -> None:
    table = Table("Name", "Kind", "Status", "Auth", "Credential", "Scopes", "Tools")
    for status in statuses:
        table.add_row(
            status.name,
            status.kind,
            status.status,
            status.auth_type,
            status.credential_ref or "-",
            ", ".join(status.scopes) or "-",
            str(len(status.tools)),
        )
    console.print(table)


def _render_integration_manifest(
    console: Console,
    manifest: IntegrationManifest,
    status: str | None,
) -> None:
    table = Table("Field", "Value")
    table.add_row("name", manifest.name)
    table.add_row("title", manifest.title)
    table.add_row("kind", manifest.kind)
    table.add_row("status", status or "available")
    table.add_row("auth", manifest.auth.type)
    table.add_row("scopes", ", ".join(manifest.auth.scopes) or "-")
    table.add_row("description", manifest.description)
    console.print(table)
    tools_table = Table("Tool", "Network", "Approval", "Risk", "Description")
    for tool in manifest.tools:
        tools_table.add_row(
            tool.name,
            tool.permissions.network,
            str(tool.permissions.approval_required or tool.permissions.mutation),
            tool.permissions.risk,
            tool.description,
        )
    console.print(tools_table)


def _render_repl_integration_connection(console: Console, name: str, status: str) -> None:
    if status == "pending_auth":
        console.print(
            f"Integration {name} is pending auth. "
            "Reconnect with --credential-ref env:VARIABLE_NAME when ready."
        )
        return
    console.print(f"Integration {name} connected.")


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


async def _handle_decision_command(
    console: Console,
    decision_service: DecisionService | None,
    session_id: str,
    argument: str,
) -> None:
    if decision_service is None:
        console.print("Decision service is not configured.")
        return
    text = argument.strip()
    if not text:
        await _handle_decisions_command(console, decision_service, session_id, "")
        return
    parts = text.split(maxsplit=2)
    try:
        if len(parts) == 2 and parts[0] == "archive":
            decision = await decision_service.archive_decision(parts[1], session_id=session_id)
            console.print(f"Archived decision {decision.id}.")
            return
        if len(parts) == 3 and parts[0] == "supersede":
            decision = await decision_service.supersede_decision(
                parts[1],
                session_id=session_id,
                title=parts[2][:80],
                decision=parts[2],
                source="user",
                priority="normal",
            )
            console.print(f"Superseded with decision {decision.id}.")
            return
        decision = await decision_service.create_decision(
            session_id=session_id,
            title=text[:80],
            decision=text,
            source="user",
            priority="normal",
        )
    except ColossusError as exc:
        console.print(f"Decision command failed: {exc}")
        return
    console.print(f"Created decision {decision.id}.")


async def _handle_decisions_command(
    console: Console,
    decision_service: DecisionService | None,
    session_id: str,
    argument: str,
) -> None:
    if decision_service is None:
        console.print("Decision service is not configured.")
        return
    status = _decision_status_filter(argument)
    try:
        decisions = await decision_service.list_decisions(session_id=session_id, status=status)
    except ColossusError as exc:
        console.print(f"Decision list failed: {exc}")
        return
    _render_decisions(console, decisions, argument)


async def _handle_memory_command(
    console: Console,
    memory_service: MemoryService | None,
    session_id: str,
    repo_root: Path,
    argument: str,
) -> None:
    if memory_service is None:
        console.print("Memory service is not configured.")
        return
    text = argument.strip()
    if not text:
        await _handle_memories_command(console, memory_service, session_id, repo_root, "")
        return
    parts = text.split(maxsplit=2)
    try:
        if len(parts) == 2 and parts[0] == "archive":
            memory = await memory_service.archive_memory(parts[1])
            console.print(f"Archived memory {memory.id}.")
            return
        if len(parts) == 2 and parts[0] == "search":
            memories = await memory_service.search_memories(
                parts[1],
                repo_root=str(repo_root),
                session_id=session_id,
            )
            _render_memories(console, memories, parts[1])
            return
        if len(parts) == 3 and parts[0] == "supersede":
            memory = await memory_service.supersede_memory(
                parts[1],
                text=parts[2],
                source="user",
            )
            console.print(f"Superseded with memory {memory.id}.")
            return
        memory = await memory_service.create_memory(
            scope="repo",
            kind="preference",
            text=text,
            source="user",
            repo_root=str(repo_root),
            session_id=session_id,
        )
    except ColossusError as exc:
        console.print(f"Memory command failed: {exc}")
        return
    console.print(f"Saved memory {memory.id} [{memory.scope}/{memory.kind}].")


async def _handle_memories_command(
    console: Console,
    memory_service: MemoryService | None,
    session_id: str,
    repo_root: Path,
    argument: str,
) -> None:
    if memory_service is None:
        console.print("Memory service is not configured.")
        return
    status = _memory_status_filter(argument)
    try:
        memories = await memory_service.search_memories(
            "",
            repo_root=str(repo_root),
            session_id=session_id,
            status=status,
            limit=50,
        )
    except ColossusError as exc:
        console.print(f"Memory list failed: {exc}")
        return
    _render_memories(console, memories, argument)


async def _handle_agents_command(
    console: Console,
    subagent_service: SubagentService | None,
    session_id: str,
    argument: str,
) -> None:
    if subagent_service is None:
        console.print("Subagent service is not configured.")
        return
    parts = argument.split()
    try:
        if len(parts) == 2 and parts[0] == "show":
            job = await subagent_service.get_job(parts[1])
            _render_subagents(console, (job,))
            if job.final_output:
                console.print(Markdown(job.final_output))
            if job.error:
                console.print(f"[red]{job.error}[/red]")
            return
        if len(parts) == 2 and parts[0] == "cancel":
            job = await subagent_service.cancel_job(parts[1])
            console.print(f"Cancelled subagent {job.id}: {job.status}")
            return
        status = _agent_status_filter(argument)
        jobs = await subagent_service.list_jobs(session_id=session_id, status=status)
    except ColossusError as exc:
        console.print(f"Subagent command failed: {exc}")
        return
    _render_subagents(console, jobs, argument)


def _agent_status_filter(argument: str) -> SubagentStatus | None:
    normalized = argument.strip().lower()
    if normalized in {"", "all", "*"}:
        return None
    if normalized in {"queued", "running", "completed", "failed", "cancelled", "interrupted"}:
        return cast(SubagentStatus, normalized)
    return None


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


def _decision_status_filter(argument: str) -> Literal["active", "archived", "superseded"] | None:
    normalized = argument.strip().lower()
    if normalized in {"", "active", "open"}:
        return "active"
    if normalized in {"all", "*"}:
        return None
    if normalized in {"archived", "superseded"}:
        return cast(Literal["active", "archived", "superseded"], normalized)
    return "active"


def _memory_status_filter(argument: str) -> MemoryStatus | None:
    normalized = argument.strip().lower()
    if normalized in {"", "active", "open"}:
        return "active"
    if normalized in {"all", "*"}:
        return None
    if normalized in {"archived", "superseded"}:
        return cast(MemoryStatus, normalized)
    return "active"


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


def _render_decisions(
    console: Console,
    decisions: tuple[KeyDecision, ...],
    argument: str = "",
) -> None:
    if not decisions:
        scope = argument.strip() or "active"
        console.print(f"No {scope} key decisions.")
        return
    table = Table("Status", "Priority", "ID", "Source", "Decision")
    for decision in decisions:
        table.add_row(
            decision.status,
            decision.priority,
            decision.id,
            decision.source,
            _short_text(decision.decision, 88),
        )
    console.print(table)


def _render_memories(
    console: Console,
    memories: tuple[MemoryItem, ...],
    argument: str = "",
) -> None:
    if not memories:
        scope = argument.strip() or "active"
        console.print(f"No {scope} memories.")
        return
    table = Table("Status", "Scope", "Kind", "ID", "Source", "Memory")
    for memory in memories:
        table.add_row(
            memory.status,
            memory.scope,
            memory.kind,
            memory.id,
            memory.source,
            _short_text(memory.text, 88),
        )
    console.print(table)


def _render_subagents(
    console: Console,
    jobs: tuple[SubagentJob, ...],
    argument: str = "",
) -> None:
    if not jobs:
        scope = argument.strip() or "current-session"
        console.print(f"No {scope} subagent jobs.")
        return
    table = Table("State", "Status", "ID", "Role", "Task", "Child Run")
    for job in jobs:
        table.add_row(
            Text(_subagent_marker(job.status)),
            job.status,
            job.id,
            job.role,
            _short_text(job.task, 72),
            job.child_run_id or "",
        )
    console.print(table)


def _subagent_marker(status: str) -> str:
    if status == "completed":
        return "[x]"
    if status in {"failed", "interrupted"}:
        return "[!]"
    if status == "running":
        return "[~]"
    if status == "cancelled":
        return "[-]"
    return "[ ]"


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
