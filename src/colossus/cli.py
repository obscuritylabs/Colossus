"""Typer CLI entry point."""

import asyncio
from dataclasses import dataclass
from pathlib import Path
from typing import Annotated, Literal
from uuid import uuid4

import typer
from rich.console import Console
from rich.markdown import Markdown
from rich.table import Table

from colossus.adapters.bundles import ManifestBundleVerifier
from colossus.application.context import ContextService
from colossus.application.defaults import default_agent
from colossus.application.model_router import ModelRouter
from colossus.application.orchestrator import AgentOrchestrator
from colossus.application.providers import ProviderDiagnostics
from colossus.application.risk import RiskAssessmentService
from colossus.domain.errors import BundleVerificationError, ColossusError
from colossus.domain.models import ProviderKind
from colossus.domain.plans import Plan
from colossus.domain.providers import (
    ProviderCapability,
    ProviderReadiness,
    ProviderReadinessCheck,
    model_context_windows_from_provider_models,
)
from colossus.domain.requests import AgentRunRequest
from colossus.domain.tasks import Task
from colossus.infrastructure.config import (
    ColossusConfig,
    ProviderOverrides,
    as_pretty_json,
    effective_model_routing,
    load_config,
    model_context_windows_from_routing,
    provider_from_config,
    write_default_config,
)
from colossus.infrastructure.container import (
    create_audit_sink,
    create_context_service,
    create_default_orchestrator,
    create_default_skill_resolver,
    create_model_router,
    create_plan_service,
    create_repl_preferences_service,
    create_state_store,
    create_task_service,
)
from colossus.infrastructure.logging import configure_logging
from colossus.infrastructure.paths import config_path, data_dir
from colossus.interfaces.approval import RichApprovalHandler
from colossus.interfaces.repl import (
    RichUserPromptHandler,
    load_user_repl_themes,
    repl_theme_names,
    run_repl_sync,
)
from colossus.interfaces.trace import EventDisplayMode, RichRunEventRenderer
from colossus.interfaces.tui import run_tui
from colossus.ports.model_provider import ModelProvider

ApprovalMode = Literal["deny", "ask", "risk-auto", "full-access"]
APPROVAL_MODE_HELP = "deny, ask, risk-auto, or full-access"
APPROVAL_MODE_ALIASES: dict[str, ApprovalMode] = {
    "full": "full-access",
    "full-access": "full-access",
    "never": "full-access",
    "yolo": "full-access",
}

app = typer.Typer(help="Colossus secure CLI agentic harness.")
config_app = typer.Typer(help="Manage configuration.")
skills_app = typer.Typer(help="Inspect and manage skills.")
tools_app = typer.Typer(help="Inspect tools.")
provider_app = typer.Typer(help="Inspect provider readiness and model catalogs.")
models_app = typer.Typer(help="Inspect configured model roles and profiles.")
bundle_app = typer.Typer(help="Verify and install offline bundles.")
plans_app = typer.Typer(help="Manage persisted plans.")
tasks_app = typer.Typer(help="Inspect persisted session tasks.")
context_app = typer.Typer(help="Inspect and manage context compaction.")
app.add_typer(config_app, name="config")
app.add_typer(skills_app, name="skills")
app.add_typer(tools_app, name="tools")
app.add_typer(provider_app, name="provider")
app.add_typer(models_app, name="models")
app.add_typer(bundle_app, name="bundle")
app.add_typer(plans_app, name="plans")
app.add_typer(tasks_app, name="tasks")
app.add_typer(context_app, name="context")

console = Console()


@dataclass(frozen=True)
class CliState:
    verbose: bool = False
    provider: str | None = None
    model: str | None = None
    context_window_tokens: int | None = None
    base_url: str | None = None
    api_key: str | None = None
    api_key_env: str | None = None
    ca_bundle: Path | None = None


@app.callback()
def callback(
    ctx: typer.Context,
    verbose: Annotated[bool, typer.Option("--verbose", "-v")] = False,
    provider: Annotated[
        str | None,
        typer.Option(
            "--provider",
            help="Override provider: echo, openai-responses, or local-openai-chat.",
        ),
    ] = None,
    model: Annotated[
        str | None,
        typer.Option("--model", help="Override model name for the selected provider."),
    ] = None,
    context_window_tokens: Annotated[
        int | None,
        typer.Option(
            "--context-window-tokens",
            help="Override context window tokens for the selected primary model.",
        ),
    ] = None,
    base_url: Annotated[
        str | None,
        typer.Option("--base-url", help="Override OpenAI/OpenAI-compatible API base URL."),
    ] = None,
    api_key: Annotated[
        str | None,
        typer.Option(
            "--api-key",
            help="API key for the provider. Prefer --api-key-env for shared shells.",
        ),
    ] = None,
    api_key_env: Annotated[
        str | None,
        typer.Option("--api-key-env", help="Environment variable containing the provider API key."),
    ] = None,
    ca_bundle: Annotated[
        Path | None,
        typer.Option(
            "--ca-bundle",
            help="Path to a custom CA certificate bundle for HTTPS model providers.",
            exists=True,
            file_okay=True,
            dir_okay=False,
            readable=True,
            resolve_path=True,
        ),
    ] = None,
) -> None:
    configure_logging(verbose)
    ctx.obj = CliState(
        verbose=verbose,
        provider=provider,
        model=model,
        context_window_tokens=context_window_tokens,
        base_url=base_url,
        api_key=api_key,
        api_key_env=api_key_env,
        ca_bundle=ca_bundle,
    )


def _cli_state(ctx: typer.Context) -> CliState:
    if isinstance(ctx.obj, CliState):
        return ctx.obj
    return CliState()


def _provider_overrides(ctx: typer.Context) -> ProviderOverrides:
    state = _cli_state(ctx)
    provider_kind = _normalize_provider(state.provider)
    return ProviderOverrides(
        kind=provider_kind,
        model=state.model,
        context_window_tokens=state.context_window_tokens,
        base_url=state.base_url,
        api_key=state.api_key,
        api_key_env=state.api_key_env,
        ca_bundle=state.ca_bundle,
    )


def _context_runtime(
    ctx: typer.Context,
    model: str | None,
) -> tuple[ContextService, str, ModelProvider, str]:
    config = load_config(config_path())
    overrides = _provider_overrides(ctx)
    router = create_model_router(config, overrides, require_credentials=False)
    primary_route = router.resolve("primary")
    context_route = router.resolve("context_summarizer")
    selected_model = model or primary_route.profile.model
    model_context_windows = _resolved_model_context_windows(config, overrides, router)
    service = create_context_service(
        data_dir(),
        context_config=config.context,
        model_context_windows=model_context_windows,
    )
    return service, selected_model, context_route.provider, context_route.profile.model


def _model_context_windows(
    config: ColossusConfig,
    overrides: ProviderOverrides | None = None,
    discovered: dict[str, int] | None = None,
) -> dict[str, int]:
    routing = effective_model_routing(config, overrides)
    return {
        **(discovered or {}),
        **config.provider.model_context_windows,
        **model_context_windows_from_routing(routing),
    }


def _resolved_model_context_windows(
    config: ColossusConfig,
    overrides: ProviderOverrides | None,
    router: ModelRouter,
) -> dict[str, int]:
    configured = _model_context_windows(config, overrides)
    discovered = asyncio.run(_discover_model_context_windows(router, configured))
    return _model_context_windows(config, overrides, discovered)


async def _discover_model_context_windows(
    router: ModelRouter,
    configured: dict[str, int],
) -> dict[str, int]:
    missing_models = {
        route.profile.model
        for route in router.list_routes()
        if route.profile.model not in configured
    }
    if not missing_models:
        return {}

    discovered: dict[str, int] = {}
    seen_catalogs: set[tuple[str, str | None, str | None, str | None]] = set()
    for route in router.list_routes():
        catalog_key = (
            route.profile.provider,
            route.profile.base_url,
            route.profile.api_key_env,
            route.profile.ca_bundle,
        )
        if catalog_key in seen_catalogs:
            continue
        seen_catalogs.add(catalog_key)
        try:
            models = await ProviderDiagnostics(route.provider).list_models()
        except Exception:
            continue
        discovered.update(model_context_windows_from_provider_models(models))
        if missing_models.issubset(discovered):
            break
    return discovered


def _resolve_approval_mode(value: str | None, *, ask_approval: bool = False) -> ApprovalMode:
    if value is None:
        return "ask" if ask_approval else "deny"
    normalized = value.strip().lower()
    if normalized in {"deny", "ask", "risk-auto", "full-access"}:
        return normalized  # type: ignore[return-value]
    if normalized in APPROVAL_MODE_ALIASES:
        return APPROVAL_MODE_ALIASES[normalized]
    console.print(f"[red]Invalid approval mode.[/red] Use {APPROVAL_MODE_HELP}.")
    raise typer.Exit(code=2)


def _resolve_events_mode(value: str) -> EventDisplayMode:
    normalized = value.strip().lower()
    if normalized in {"compact", "verbose", "off"}:
        return normalized  # type: ignore[return-value]
    console.print("[red]Invalid events mode.[/red] Use compact, verbose, or off.")
    raise typer.Exit(code=2)


def _normalize_provider(
    value: str | None,
) -> ProviderKind | None:
    if value is None:
        return None
    normalized = value.strip().lower().replace("-", "_")
    if normalized not in {"echo", "openai_responses", "local_openai_chat"}:
        console.print(
            "[red]Invalid provider.[/red] Use echo, openai-responses, or local-openai-chat."
        )
        raise typer.Exit(code=2)
    return normalized  # type: ignore[return-value]


@app.command()
def run(
    ctx: typer.Context,
    prompt: Annotated[str | None, typer.Argument(help="Prompt to run.")] = None,
    plan: Annotated[
        bool,
        typer.Option("--plan", help="Create a persisted plan without executing tools."),
    ] = False,
    execute_plan: Annotated[
        str | None,
        typer.Option("--execute-plan", help="Execute an approved persisted plan id."),
    ] = None,
    session: Annotated[
        str | None,
        typer.Option("--session", help="Persist messages under this session id."),
    ] = None,
    trace: Annotated[
        bool,
        typer.Option("--trace", help="Show observable agent events such as tool calls."),
    ] = False,
    stream: Annotated[
        bool,
        typer.Option("--stream", help="Stream assistant output while the model responds."),
    ] = False,
    events: Annotated[
        str,
        typer.Option("--events", help="Event display mode: compact, verbose, or off."),
    ] = "compact",
    reasoning: Annotated[
        bool,
        typer.Option("--reasoning/--no-reasoning", help="Show provider reasoning summaries."),
    ] = True,
    model_role: Annotated[
        str,
        typer.Option(
            "--model-role",
            help="Model role to use for this agent turn.",
        ),
    ] = "primary",
    ask_approval: Annotated[
        bool,
        typer.Option(
            "--ask-approval",
            help="Prompt before approval-required tool calls. Defaults to deny in one-shot mode.",
        ),
    ] = False,
    approval_mode: Annotated[
        str | None,
        typer.Option(
            "--approval-mode",
            help=f"Approval mode: {APPROVAL_MODE_HELP}.",
        ),
    ] = None,
) -> None:
    """Run one agent turn."""
    if plan and execute_plan is not None:
        console.print("[red]Use either --plan or --execute-plan, not both.[/red]")
        raise typer.Exit(code=2)
    session_id = session or str(uuid4())
    if plan:
        if prompt is None:
            console.print("[red]A prompt is required when creating a plan.[/red]")
            raise typer.Exit(code=2)
        created = asyncio.run(create_plan_service(data_dir()).create_plan(prompt, session_id))
        _print_plan(created)
        return
    config = load_config(config_path())
    overrides = _provider_overrides(ctx)
    router = create_model_router(config, overrides)
    route = router.resolve(model_role)
    context_route = router.resolve("context_summarizer")
    model_context_windows = _resolved_model_context_windows(config, overrides, router)
    resolved_approval_mode = _resolve_approval_mode(approval_mode, ask_approval=ask_approval)
    events_mode = _resolve_events_mode(events)
    if trace and events_mode == "off":
        events_mode = "compact"
    trace_renderer = RichRunEventRenderer(
        console,
        enabled=events_mode != "off" or stream,
        events_mode=events_mode,
        stream_model_output=stream,
        show_reasoning=reasoning,
    )
    orchestrator = create_default_orchestrator(
        data_dir(),
        route.provider,
        context_config=config.context,
        model_context_windows=model_context_windows,
        context_model=context_route.profile.model,
        context_provider=context_route.provider,
        event_observer=trace_renderer.render,
        approval_handler=(
            RichApprovalHandler(console)
            if resolved_approval_mode in {"ask", "risk-auto"}
            else None
        ),
        risk_assessment_service=RiskAssessmentService(router),
        risk_auto_approve=resolved_approval_mode == "risk-auto",
        auto_approve_required_tools=resolved_approval_mode == "full-access",
    )
    plan_id = execute_plan
    if execute_plan is not None:
        approved_plan = asyncio.run(create_plan_service(data_dir()).require_approved(execute_plan))
        prompt = prompt or approved_plan.prompt
        session_id = approved_plan.session_id
    if prompt is None:
        console.print("[red]A prompt or --execute-plan is required.[/red]")
        raise typer.Exit(code=2)
    trace_renderer.begin_run()
    try:
        result = asyncio.run(
            orchestrator.run(
                AgentRunRequest(
                    prompt=prompt,
                    agent=default_agent(route.profile.model),
                    session_id=session_id,
                    plan_id=plan_id,
                )
            )
        )
    finally:
        trace_renderer.end_run()
    if execute_plan is not None:
        asyncio.run(create_plan_service(data_dir()).mark_executed(execute_plan, result.run_id))
    if not trace_renderer.rendered_model_output:
        console.print(result.final_output, markup=False)
    console.print(
        f"[dim]run_id={result.run_id} session_id={session_id} events={result.events_recorded}[/dim]"
    )


@app.command()
def repl(
    ctx: typer.Context,
    approval_mode: Annotated[
        str,
        typer.Option(
            "--approval-mode",
            help=(
                "Approval mode: ask, risk-auto, or full-access. "
                "Use deny only for non-interactive testing."
            ),
        ),
    ] = "ask",
    theme: Annotated[
        str | None,
        typer.Option(
            "--theme",
            help="REPL theme: default, mono, high-contrast, carrot, hacker, or a user theme.",
        ),
    ] = None,
) -> None:
    """Start the interactive REPL."""
    theme_dirs = (config_path().parent / "themes",)
    available_theme_names = (*repl_theme_names(), *load_user_repl_themes(theme_dirs))
    if theme is not None and theme not in available_theme_names:
        console.print(
            f"[red]Invalid REPL theme.[/red] Use {', '.join(available_theme_names)}."
        )
        raise typer.Exit(code=2)
    config = load_config(config_path())
    overrides = _provider_overrides(ctx)
    router = create_model_router(config, overrides)
    primary_route = router.resolve("primary")
    context_route = router.resolve("context_summarizer")
    model_context_windows = _resolved_model_context_windows(config, overrides, router)
    resolved_approval_mode = _resolve_approval_mode(approval_mode)
    state = create_state_store(data_dir())
    audit = create_audit_sink(data_dir())
    context_service = create_context_service(
        data_dir(),
        state_store=state,
        audit_sink=audit,
        context_config=config.context,
        model_context_windows=model_context_windows,
    )
    user_prompt_handler = RichUserPromptHandler(console)

    def build_orchestrator(model_role: str) -> AgentOrchestrator:
        route = router.resolve(model_role)
        return create_default_orchestrator(
            data_dir(),
            route.provider,
            state_store=state,
            audit_sink=audit,
            context_service=context_service,
            context_config=config.context,
            model_context_windows=model_context_windows,
            context_model=context_route.profile.model,
            context_provider=context_route.provider,
            approval_handler=(
                RichApprovalHandler(console)
                if resolved_approval_mode in {"ask", "risk-auto"}
                else None
            ),
            user_prompt_handler=user_prompt_handler,
            risk_assessment_service=RiskAssessmentService(router),
            risk_auto_approve=resolved_approval_mode == "risk-auto",
            auto_approve_required_tools=resolved_approval_mode == "full-access",
        )

    history_path = data_dir() / "repl_history.txt"
    history_path.parent.mkdir(parents=True, exist_ok=True)
    run_repl_sync(
        build_orchestrator("primary"),
        create_default_skill_resolver(),
        context_service,
        context_route.provider,
        default_agent(primary_route.profile.model),
        model_router=router,
        active_model_role="primary",
        orchestrator_factory=build_orchestrator,
        context_model=context_route.profile.model,
        approval_mode=resolved_approval_mode,
        history_path=history_path,
        preferences_service=create_repl_preferences_service(data_dir()),
        task_service=create_task_service(data_dir()),
        plan_service=create_plan_service(data_dir()),
        theme_name=theme,
        theme_dirs=theme_dirs,
    )


@app.command()
def tui() -> None:
    """Start the Textual TUI."""
    run_tui()


@config_app.command("init")
def config_init(force: Annotated[bool, typer.Option("--force")] = False) -> None:
    """Write a default config file."""
    path = config_path()
    if path.exists() and not force:
        console.print(f"Config already exists: {path}")
        raise typer.Exit(code=1)
    write_default_config(path)
    console.print(f"Wrote {path}")


@config_app.command("show")
def config_show() -> None:
    """Show resolved config."""
    console.print(as_pretty_json(load_config(config_path())))


@skills_app.command("list")
def skills_list() -> None:
    """List bundled and enabled skills."""
    resolver = create_default_skill_resolver()
    table = Table("Name", "Version", "Offline", "Source")
    for skill in resolver.list_skills():
        table.add_row(
            skill.manifest.name,
            skill.manifest.version,
            str(skill.manifest.offline_compatible),
            skill.source,
        )
    console.print(table)


@tools_app.command("list")
def tools_list() -> None:
    """List built-in tools."""
    orchestrator = create_default_orchestrator(data_dir())
    specs = orchestrator.tool_specs()
    table = Table()
    table.add_column("Name", no_wrap=True)
    table.add_column("Filesystem")
    table.add_column("Network")
    table.add_column("Approval")
    table.add_column("Timeout")
    table.add_column("Risk")
    table.add_column("Description")
    for spec in specs:
        table.add_row(
            spec.name,
            spec.permissions.filesystem,
            spec.permissions.network,
            str(spec.permissions.approval_required or spec.permissions.mutation),
            str(spec.timeout_seconds),
            spec.permissions.risk,
            spec.description,
        )
    console.print(table)


@context_app.command("show")
def context_show(
    ctx: typer.Context,
    session: Annotated[str, typer.Option("--session", help="Session id to inspect.")],
    model: Annotated[str | None, typer.Option("--model", help="Override model for budget.")] = None,
) -> None:
    """Show context budget and latest snapshot for a session."""
    service, selected_model, _provider, _summary_model = _context_runtime(ctx, model)
    status = asyncio.run(service.status(session, selected_model))
    table = Table("Field", "Value")
    for key, value in status.model_dump(mode="json").items():
        table.add_row(key, str(value))
    console.print(table)


@context_app.command("compact")
def context_compact(
    ctx: typer.Context,
    session: Annotated[str, typer.Option("--session", help="Session id to compact.")],
    model: Annotated[str | None, typer.Option("--model", help="Override model for budget.")] = None,
) -> None:
    """Create a context snapshot for a session."""
    service, selected_model, provider, summary_model = _context_runtime(ctx, model)
    snapshot = asyncio.run(
        service.compact_session(
            session_id=session,
            model=selected_model,
            provider=provider,
            summary_model=summary_model,
        )
    )
    console.print(f"Compacted session {session} into snapshot {snapshot.id}")
    console.print(f"Strategy: {snapshot.strategy}")
    console.print(
        f"Source messages: {snapshot.source_message_range[0]}-{snapshot.source_message_range[1]}"
    )


@context_app.command("snapshots")
def context_snapshots(
    session: Annotated[str, typer.Option("--session", help="Session id to inspect.")],
) -> None:
    """List context snapshots for a session."""
    config = load_config(config_path())
    service = create_context_service(
        data_dir(),
        context_config=config.context,
        model_context_windows=_model_context_windows(config),
    )
    snapshots = asyncio.run(service.list_snapshots(session))
    table = Table()
    table.add_column("Snapshot", no_wrap=True)
    table.add_column("Strategy")
    table.add_column("Source")
    table.add_column("Created")
    for snapshot in snapshots:
        table.add_row(
            snapshot.id,
            snapshot.strategy,
            f"{snapshot.source_message_range[0]}-{snapshot.source_message_range[1]}",
            snapshot.created_at,
        )
    console.print(table)


@context_app.command("restore")
def context_restore(snapshot_id: Annotated[str, typer.Argument(help="Snapshot id.")]) -> None:
    """Select a context snapshot as active."""
    config = load_config(config_path())
    service = create_context_service(
        data_dir(),
        context_config=config.context,
        model_context_windows=_model_context_windows(config),
    )
    snapshot = asyncio.run(service.restore_snapshot(snapshot_id))
    console.print(f"Restored snapshot {snapshot.id} for session {snapshot.session_id}")


@provider_app.command("doctor")
def provider_doctor(
    ctx: typer.Context,
    probe_tools: Annotated[
        bool,
        typer.Option(
            "--probe-tools",
            help="Ask the selected model to emit a structured tool call.",
        ),
    ] = False,
) -> None:
    """Check whether the selected provider is ready."""
    config = load_config(config_path())
    overrides = _provider_overrides(ctx)
    provider = provider_from_config(
        config,
        overrides,
        require_credentials=False,
    )
    diagnostics = ProviderDiagnostics(provider)
    readiness = asyncio.run(diagnostics.check_readiness())
    _render_provider_readiness(readiness)
    _render_provider_capabilities(diagnostics.capabilities())
    if readiness.ready and probe_tools:
        selected_model = overrides.model or config.provider.model
        _render_provider_checks(
            "Model Probes",
            (asyncio.run(diagnostics.probe_tool_calls(selected_model)),),
        )
    if not readiness.ready:
        raise typer.Exit(code=1)


@provider_app.command("models")
def provider_models(ctx: typer.Context) -> None:
    """List models advertised by the selected provider."""
    config = load_config(config_path())
    provider = provider_from_config(config, _provider_overrides(ctx))
    models = asyncio.run(ProviderDiagnostics(provider).list_models())
    table = Table("Model", "Owner", "Created", "Context", "Max Output")
    for model in models:
        table.add_row(
            model.id,
            model.owner or "",
            str(model.created) if model.created is not None else "",
            str(model.context_window_tokens) if model.context_window_tokens is not None else "",
            str(model.max_output_tokens) if model.max_output_tokens is not None else "",
        )
    console.print(table)


@models_app.command("list")
def models_list(
    ctx: typer.Context,
    check: Annotated[
        bool,
        typer.Option("--check", help="Check readiness for each configured role."),
    ] = False,
) -> None:
    """List configured model roles and profiles."""
    config = load_config(config_path())
    router = create_model_router(config, _provider_overrides(ctx), require_credentials=False)
    readiness = asyncio.run(_model_role_readiness(router)) if check else {}
    table = Table()
    table.add_column("Role", no_wrap=True)
    table.add_column("Profile", no_wrap=True)
    table.add_column("Provider")
    table.add_column("Model")
    table.add_column("Base URL")
    table.add_column("Context")
    if check:
        table.add_column("Ready")
    for route in router.list_routes():
        row = [
            route.role,
            route.profile_name,
            route.profile.provider,
            route.profile.model,
            route.profile.base_url or "",
            str(route.profile.context_window_tokens or ""),
        ]
        if check:
            row.append(readiness.get(route.role, "unknown"))
        table.add_row(*row)
    console.print(table)


@models_app.command("doctor")
def models_doctor(
    ctx: typer.Context,
    role: Annotated[
        str,
        typer.Option("--role", help="Model role to check."),
    ] = "primary",
) -> None:
    """Check readiness for a configured model role."""
    config = load_config(config_path())
    router = create_model_router(config, _provider_overrides(ctx), require_credentials=False)
    route = router.resolve(role)
    diagnostics = ProviderDiagnostics(route.provider)
    readiness = asyncio.run(diagnostics.check_readiness())
    console.print(f"Role: {route.role}")
    console.print(f"Profile: {route.profile_name}")
    console.print(f"Model: {route.profile.model}")
    _render_provider_readiness(readiness)
    _render_provider_capabilities(diagnostics.capabilities())
    if not readiness.ready:
        raise typer.Exit(code=1)


async def _model_role_readiness(router: ModelRouter) -> dict[str, str]:
    statuses: dict[str, str] = {}
    for route in router.list_routes():
        readiness = await ProviderDiagnostics(route.provider).check_readiness()
        statuses[route.role] = "yes" if readiness.ready else "no"
    return statuses


def _render_provider_readiness(readiness: ProviderReadiness) -> None:
    console.print(f"Provider: {readiness.provider}")
    console.print(f"Status: {'ready' if readiness.ready else 'not ready'}")
    _render_provider_checks("Provider Checks", readiness.checks)


def _render_provider_checks(
    title: str,
    checks: tuple[ProviderReadinessCheck, ...],
) -> None:
    table = Table("Check", "Status", "Detail")
    table.title = title
    for check in checks:
        table.add_row(check.name, check.status, check.detail)
    console.print(table)


def _render_provider_capabilities(capabilities: tuple[ProviderCapability, ...]) -> None:
    table = Table("Capability", "Supported", "Detail")
    for capability in capabilities:
        table.add_row(
            capability.name,
            "yes" if capability.supported else "no",
            capability.detail or "",
        )
    console.print(table)


@bundle_app.command("verify")
def bundle_verify(path: Annotated[Path, typer.Argument(exists=True, file_okay=False)]) -> None:
    """Verify an offline bundle manifest and checksums."""
    try:
        ManifestBundleVerifier().verify(path)
    except BundleVerificationError as exc:
        console.print(f"[red]Bundle verification failed:[/red] {exc}")
        raise typer.Exit(code=1) from exc
    console.print("Bundle verified.")


@plans_app.command("list")
def plans_list(session: Annotated[str | None, typer.Option("--session")] = None) -> None:
    """List persisted plans."""
    plans = asyncio.run(create_plan_service(data_dir()).list_plans(session))
    for item in plans:
        console.print(
            (
                f"{item.id}\t"
                f"session={item.session_id}\t"
                f"status={item.status}\t"
                f"approval={item.requires_approval}\t"
                f"prompt={item.prompt[:80]}"
            ),
            markup=False,
        )


@plans_app.command("show")
def plans_show(plan_id: Annotated[str, typer.Argument(help="Plan id.")]) -> None:
    """Show a persisted plan."""
    plan = asyncio.run(create_plan_service(data_dir()).get_plan(plan_id))
    _print_plan(plan)


@plans_app.command("approve")
def plans_approve(plan_id: Annotated[str, typer.Argument(help="Plan id.")]) -> None:
    """Approve a persisted plan for execution."""
    plan = asyncio.run(create_plan_service(data_dir()).approve_plan(plan_id))
    console.print(f"Approved plan {plan.id}")


@tasks_app.command("list")
def tasks_list(
    session: Annotated[str | None, typer.Option("--session")] = None,
    status: Annotated[str | None, typer.Option("--status")] = None,
) -> None:
    """List persisted session tasks."""
    tasks = asyncio.run(create_task_service(data_dir()).list_tasks(session_id=session))
    if status:
        tasks = tuple(task for task in tasks if task.status == status)
    _print_tasks(tasks)


def _print_plan(plan: Plan) -> None:
    console.print(f"[bold]Plan:[/bold] {plan.id}")
    console.print(f"Session: {plan.session_id}")
    console.print(f"Status: {plan.status}")
    console.print(f"Requires approval: {plan.requires_approval}")
    console.print(f"Prompt: {plan.prompt}")
    if plan.content.strip():
        console.print(Markdown(plan.content))
        return
    table = Table("Step", "Title", "Mutation", "Detail")
    for step in plan.steps:
        table.add_row(str(step.index), step.title, str(step.requires_mutation), step.detail)
    console.print(table)


def _print_tasks(tasks: tuple[Task, ...]) -> None:
    table = Table("Status", "ID", "Session", "Title", "Description")
    for task in tasks:
        table.add_row(
            task.status,
            task.id,
            task.session_id,
            task.title,
            task.description[:80],
        )
    console.print(table)


def main() -> None:
    try:
        app()
    except ColossusError as exc:
        console.print(f"[red]Error:[/red] {exc}")
        raise typer.Exit(code=1) from None
