"""Typer CLI entry point."""

import asyncio
from dataclasses import dataclass
from pathlib import Path
from typing import Annotated, Literal, cast
from uuid import uuid4

import typer
from rich.console import Console
from rich.markdown import Markdown
from rich.table import Table

from colossus.adapters.bundles import ManifestBundleVerifier
from colossus.adapters.credentials_env import EnvCredentialBroker
from colossus.application.context import ContextService
from colossus.application.decisions import DecisionService
from colossus.application.defaults import default_agent
from colossus.application.integrations import IntegrationService
from colossus.application.memories import MemoryService
from colossus.application.model_router import ModelRouter
from colossus.application.orchestrator import AgentOrchestrator
from colossus.application.packs import PackService
from colossus.application.providers import ProviderDiagnostics
from colossus.application.research import ResearchService
from colossus.application.risk import RiskAssessmentService
from colossus.application.skills import SkillResolver
from colossus.application.subagents import SubagentService
from colossus.domain.agents import MAX_AGENT_MAX_TURNS
from colossus.domain.decisions import DecisionStatus, KeyDecision
from colossus.domain.errors import BundleVerificationError, ColossusError
from colossus.domain.integrations import IntegrationAuthType, IntegrationStatusView
from colossus.domain.memories import MemoryItem, MemoryKind, MemoryScope, MemoryStatus
from colossus.domain.messages import AssistantMessage, Message, ToolResultMessage, UserMessage
from colossus.domain.models import ProviderKind
from colossus.domain.plans import Plan
from colossus.domain.providers import (
    ProviderCapability,
    ProviderReadiness,
    ProviderReadinessCheck,
    model_context_windows_from_provider_models,
)
from colossus.domain.requests import AgentRunRequest, AgentRunResult
from colossus.domain.research import ResearchDepth, ResearchSourceKind
from colossus.domain.sessions import SessionSummary
from colossus.domain.subagents import SubagentJob, SubagentStatus
from colossus.domain.tasks import Task
from colossus.infrastructure.config import (
    ColossusConfig,
    HttpOverrides,
    ProviderOverrides,
    as_pretty_json,
    effective_model_routing,
    http_client_config_from_config,
    load_config,
    model_context_windows_from_routing,
    provider_from_config,
    write_default_config,
)
from colossus.infrastructure.container import (
    create_audit_sink,
    create_context_service,
    create_decision_service,
    create_default_orchestrator,
    create_default_skill_resolver,
    create_integration_service,
    create_mcp_gateway,
    create_memory_service,
    create_model_router,
    create_pack_service,
    create_plan_service,
    create_repl_preferences_service,
    create_research_service,
    create_search_provider,
    create_session_service,
    create_skill_authoring_service,
    create_state_store,
    create_subagent_service,
    create_task_service,
)
from colossus.infrastructure.http_client import HttpClientConfig
from colossus.infrastructure.logging import configure_logging
from colossus.infrastructure.paths import config_path, data_dir
from colossus.interfaces.approval import RichApprovalHandler
from colossus.interfaces.repl import (
    ReplWorkspaceServices,
    RichUserPromptHandler,
    load_user_repl_themes,
    repl_theme_names,
    run_repl_sync,
)
from colossus.interfaces.trace import EventDisplayMode, RichRunEventRenderer
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
agents_app = typer.Typer(help="Inspect durable subagent jobs.")
bundle_app = typer.Typer(help="Verify and install offline bundles.")
plans_app = typer.Typer(help="Manage persisted plans.")
tasks_app = typer.Typer(help="Inspect persisted session tasks.")
decisions_app = typer.Typer(help="Inspect and manage key decisions.")
memories_app = typer.Typer(help="Inspect and manage durable memories.")
context_app = typer.Typer(help="Inspect and manage context compaction.")
sessions_app = typer.Typer(help="Discover and resume persisted sessions.")
integrations_app = typer.Typer(help="Manage app and service integrations.")
packs_app = typer.Typer(help="Manage capability packs.")
packs_trust_app = typer.Typer(help="Manage trusted pack publishers and keys.")
app.add_typer(config_app, name="config")
app.add_typer(skills_app, name="skills")
app.add_typer(tools_app, name="tools")
app.add_typer(provider_app, name="provider")
app.add_typer(models_app, name="models")
app.add_typer(agents_app, name="agents")
app.add_typer(bundle_app, name="bundle")
app.add_typer(plans_app, name="plans")
app.add_typer(tasks_app, name="tasks")
app.add_typer(decisions_app, name="decisions")
app.add_typer(memories_app, name="memories")
app.add_typer(context_app, name="context")
app.add_typer(sessions_app, name="sessions")
app.add_typer(integrations_app, name="integrations")
app.add_typer(packs_app, name="packs")
packs_app.add_typer(packs_trust_app, name="trust")

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
    http_ca_bundle: Path | None = None
    http_client_cert: Path | None = None
    http_client_key: Path | None = None
    http_client_key_password_env: str | None = None
    http_proxy: str | None = None
    http_proxy_env: str | None = None
    http_trust_env: bool | None = None


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
    http_ca_bundle: Annotated[
        Path | None,
        typer.Option(
            "--http-ca-bundle",
            help="Path to a custom CA certificate bundle for Colossus-owned HTTP clients.",
            exists=True,
            file_okay=True,
            dir_okay=False,
            readable=True,
            resolve_path=True,
        ),
    ] = None,
    http_client_cert: Annotated[
        Path | None,
        typer.Option(
            "--http-client-cert",
            help="Path to a client certificate for mTLS/PKI-protected HTTP endpoints.",
            exists=True,
            file_okay=True,
            dir_okay=False,
            readable=True,
            resolve_path=True,
        ),
    ] = None,
    http_client_key: Annotated[
        Path | None,
        typer.Option(
            "--http-client-key",
            help="Path to the private key for --http-client-cert.",
            exists=True,
            file_okay=True,
            dir_okay=False,
            readable=True,
            resolve_path=True,
        ),
    ] = None,
    http_client_key_password_env: Annotated[
        str | None,
        typer.Option(
            "--http-client-key-password-env",
            help="Environment variable containing the HTTP client key password.",
        ),
    ] = None,
    http_proxy: Annotated[
        str | None,
        typer.Option("--http-proxy", help="Proxy URL for Colossus-owned HTTP clients."),
    ] = None,
    http_proxy_env: Annotated[
        str | None,
        typer.Option(
            "--http-proxy-env",
            help="Environment variable containing the HTTP proxy URL.",
        ),
    ] = None,
    http_no_trust_env: Annotated[
        bool,
        typer.Option(
            "--http-no-trust-env",
            help="Ignore standard HTTP proxy and certificate environment variables.",
        ),
    ] = False,
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
        http_ca_bundle=http_ca_bundle,
        http_client_cert=http_client_cert,
        http_client_key=http_client_key,
        http_client_key_password_env=http_client_key_password_env,
        http_proxy=http_proxy,
        http_proxy_env=http_proxy_env,
        http_trust_env=False if http_no_trust_env else None,
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


def _http_overrides(ctx: typer.Context) -> HttpOverrides:
    state = _cli_state(ctx)
    return HttpOverrides(
        ca_bundle=state.http_ca_bundle,
        client_cert=state.http_client_cert,
        client_key=state.http_client_key,
        client_key_password_env=state.http_client_key_password_env,
        proxy_url=state.http_proxy,
        proxy_url_env=state.http_proxy_env,
        trust_env=state.http_trust_env,
    )


def _http_client_config(ctx: typer.Context, config: ColossusConfig) -> HttpClientConfig:
    return http_client_config_from_config(config, _http_overrides(ctx))


def _skill_resolver(config: ColossusConfig, workspace_root: Path | None = None) -> SkillResolver:
    return create_default_skill_resolver(
        data_dir() / "skills",
        allow_user_overrides=config.allow_user_skill_overrides,
        pack_root=data_dir() / "packs",
        workspace_root=workspace_root,
    )


def _workspace_root(value: Path | None = None) -> Path:
    root = value or Path.cwd()
    resolved = root.expanduser().resolve()
    if not resolved.exists():
        console.print(f"[red]Workspace does not exist:[/red] {resolved}")
        raise typer.Exit(code=2)
    if not resolved.is_dir():
        console.print(f"[red]Workspace is not a directory:[/red] {resolved}")
        raise typer.Exit(code=2)
    return resolved


def _context_runtime(
    ctx: typer.Context,
    model: str | None,
) -> tuple[ContextService, str, ModelProvider, str]:
    config = load_config(config_path())
    overrides = _provider_overrides(ctx)
    http_client_config = _http_client_config(ctx, config)
    router = create_model_router(
        config,
        overrides,
        require_credentials=False,
        http_client_config=http_client_config,
    )
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


def _research_depth(value: str) -> ResearchDepth:
    normalized = value.strip().lower()
    if normalized in {"quick", "standard", "deep"}:
        return normalized  # type: ignore[return-value]
    console.print("[red]Invalid research depth.[/red] Use quick, standard, or deep.")
    raise typer.Exit(code=2)


def _research_sources(
    values: list[str] | None,
    defaults: tuple[ResearchSourceKind, ...],
) -> tuple[ResearchSourceKind, ...]:
    selected = values or list(defaults)
    normalized: list[ResearchSourceKind] = []
    for value in selected:
        item = value.strip().lower()
        if item not in {"repo", "web", "mcp"}:
            console.print("[red]Invalid research source.[/red] Use repo, web, or mcp.")
            raise typer.Exit(code=2)
        normalized.append(item)  # type: ignore[arg-type]
    return tuple(dict.fromkeys(normalized))


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


def _integration_runtime() -> IntegrationService:
    state = create_state_store(data_dir())
    audit = create_audit_sink(data_dir())
    return create_integration_service(
        data_dir(),
        state_store=state,
        audit_sink=audit,
        credential_broker=EnvCredentialBroker(),
    )


def _pack_runtime() -> PackService:
    state = create_state_store(data_dir())
    audit = create_audit_sink(data_dir())
    return create_pack_service(data_dir(), state_store=state, audit_sink=audit)


def _integration_auth_type(value: str) -> IntegrationAuthType:
    normalized = value.strip().lower().replace("-", "_")
    if normalized not in {
        "none",
        "api_key",
        "bearer",
        "oauth2_authorization_code",
        "service_account",
    }:
        console.print(
            "[red]Invalid auth type.[/red] Use none, api-key, bearer, "
            "oauth2-authorization-code, or service-account."
        )
        raise typer.Exit(code=2)
    return cast(IntegrationAuthType, normalized)


def _credential_ref_summary(
    credential_ref: str | None,
    credential_refs: dict[str, str] | None = None,
) -> str:
    refs = credential_refs or {}
    parts: list[str] = []
    if credential_ref:
        parts.append(credential_ref)
    parts.extend(f"{key}={value}" for key, value in sorted(refs.items()))
    return ", ".join(parts) or "-"


def _print_integration_statuses(statuses: tuple[IntegrationStatusView, ...]) -> None:
    table = Table("Name", "Kind", "Status", "Auth", "Credentials", "Scopes", "Tools")
    for status in statuses:
        table.add_row(
            status.name,
            status.kind,
            status.status,
            status.auth_type,
            _credential_ref_summary(status.credential_ref, status.credential_refs),
            ", ".join(status.scopes) or "-",
            str(len(status.tools)),
        )
    console.print(table)


def _print_integration_connection(
    name: str,
    status: str,
    credential_ref: str | None,
    credential_refs: dict[str, str] | None = None,
) -> None:
    if status == "pending_auth":
        console.print(
            f"Integration {name} is pending auth. "
            "Reconnect with the required credential refs when ready."
        )
        return
    console.print(
        f"Integration {name} connected with "
        f"credentials={_credential_ref_summary(credential_ref, credential_refs)}."
    )


def _integration_connect_config(
    *,
    base_url: str | None,
    auth_type: str | None,
    auth_header: str | None,
    auth_scheme: str | None,
) -> dict[str, object]:
    config: dict[str, object] = {}
    if base_url is not None:
        config["base_url"] = base_url
    if auth_type is not None:
        config["auth_type"] = auth_type
    if auth_header is not None:
        config["auth_header"] = auth_header
    if auth_scheme is not None:
        config["auth_scheme"] = auth_scheme
    return config


def _integration_credential_refs(
    *,
    username_ref: str | None,
    password_ref: str | None,
) -> dict[str, str]:
    refs: dict[str, str] = {}
    if username_ref is not None:
        refs["username"] = username_ref
    if password_ref is not None:
        refs["password"] = password_ref
    return refs


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
        typer.Option("--session", help="Use or resume this exact session id."),
    ] = None,
    resume: Annotated[
        bool,
        typer.Option("--resume", help="Resume the most recently updated session."),
    ] = False,
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
    max_turns: Annotated[
        int | None,
        typer.Option(
            "--max-turns",
            min=1,
            max=MAX_AGENT_MAX_TURNS,
            help="Maximum model turns for this run.",
        ),
    ] = None,
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
    skill: Annotated[
        list[str] | None,
        typer.Option("--skill", help="Activate a skill for this one-shot run."),
    ] = None,
    workspace: Annotated[
        Path | None,
        typer.Option(
            "--workspace",
            "-C",
            help="Workspace root for tools, shell commands, repo context, and subagents.",
        ),
    ] = None,
) -> None:
    """Run one agent turn."""
    if plan and execute_plan is not None:
        console.print("[red]Use either --plan or --execute-plan, not both.[/red]")
        raise typer.Exit(code=2)
    if resume and session is not None:
        console.print("[red]Use either --resume or --session, not both.[/red]")
        raise typer.Exit(code=2)
    if resume and (plan or execute_plan is not None):
        console.print("[red]--resume is only supported for direct agent runs.[/red]")
        raise typer.Exit(code=2)
    state = create_state_store(data_dir())
    if resume:
        try:
            session_id = asyncio.run(create_session_service(data_dir()).latest_session()).id
        except ColossusError as exc:
            console.print(f"[red]{exc}[/red]")
            raise typer.Exit(code=1) from exc
    else:
        session_id = session or str(uuid4())
    if plan:
        if prompt is None:
            console.print("[red]A prompt is required when creating a plan.[/red]")
            raise typer.Exit(code=2)
        created = asyncio.run(create_plan_service(data_dir()).create_plan(prompt, session_id))
        _print_plan(created)
        return
    workspace_root = _workspace_root(workspace)
    config = load_config(config_path())
    skill_resolver = _skill_resolver(config, workspace_root)
    overrides = _provider_overrides(ctx)
    http_client_config = _http_client_config(ctx, config)
    router = create_model_router(
        config,
        overrides,
        http_client_config=http_client_config,
    )
    route = router.resolve(model_role)
    context_route = router.resolve("context_summarizer")
    model_context_windows = _resolved_model_context_windows(config, overrides, router)
    resolved_max_turns = max_turns if max_turns is not None else config.agent.max_turns
    resolved_approval_mode = _resolve_approval_mode(approval_mode, ask_approval=ask_approval)
    audit = create_audit_sink(data_dir())
    credential_broker = EnvCredentialBroker()
    pack_service = create_pack_service(data_dir(), state_store=state, audit_sink=audit)
    integration_service = create_integration_service(
        data_dir(),
        state_store=state,
        audit_sink=audit,
        credential_broker=credential_broker,
        pack_service=pack_service,
    )
    integration_connections = asyncio.run(integration_service.connected_connections())
    subagent_service = create_subagent_service(
        data_dir(),
        state_store=state,
        audit_sink=audit,
        max_concurrent=config.subagents.max_concurrent,
    )
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
        workspace_root=workspace_root,
        state_store=state,
        audit_sink=audit,
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
        subagent_service=subagent_service,
        model_router=router,
        search_provider=create_search_provider(config.research.search, http_client_config),
        mcp_gateway=create_mcp_gateway(config.research.mcp),
        http_client_config=http_client_config,
        integration_connections=integration_connections,
        credential_broker=credential_broker,
        skill_resolver=skill_resolver,
        agent_max_turns=resolved_max_turns,
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
            _run_agent_and_drain_subagents(
                orchestrator,
                subagent_service,
                AgentRunRequest(
                    prompt=prompt,
                    agent=default_agent(route.profile.model, max_turns=resolved_max_turns),
                    session_id=session_id,
                    plan_id=plan_id,
                    active_skills=tuple(skill or ()),
                )
            )
        )
    except ColossusError as exc:
        console.print(f"[red]{exc}[/red]")
        raise typer.Exit(code=1) from exc
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
def research(
    ctx: typer.Context,
    question: Annotated[str, typer.Argument(help="Research question to answer.")],
    depth: Annotated[
        str,
        typer.Option("--depth", help="Research depth: quick, standard, or deep."),
    ] = "standard",
    max_sources: Annotated[
        int,
        typer.Option("--max-sources", min=1, max=100, help="Maximum evidence sources."),
    ] = 20,
    source: Annotated[
        list[str] | None,
        typer.Option("--source", help="Source lane to use: repo, web, or mcp. Repeatable."),
    ] = None,
    workspace: Annotated[
        Path | None,
        typer.Option(
            "--workspace",
            "-C",
            help="Workspace root for repo evidence collection.",
        ),
    ] = None,
    session: Annotated[
        str | None,
        typer.Option("--session", help="Use or resume this exact session id."),
    ] = None,
    resume: Annotated[
        bool,
        typer.Option("--resume", help="Attach research to the most recently updated session."),
    ] = False,
    events: Annotated[
        str,
        typer.Option("--events", help="Event display mode: compact, verbose, or off."),
    ] = "compact",
    approval_mode: Annotated[
        str,
        typer.Option("--approval-mode", help=f"Approval mode: {APPROVAL_MODE_HELP}."),
    ] = "ask",
) -> None:
    """Run deep research and persist a cited report."""
    if resume and session is not None:
        console.print("[red]Use either --resume or --session, not both.[/red]")
        raise typer.Exit(code=2)
    config = load_config(config_path())
    workspace_root = _workspace_root(workspace)
    research_depth = _research_depth(depth)
    source_kinds = _research_sources(source, config.research.sources)
    state = create_state_store(data_dir())
    audit = create_audit_sink(data_dir())
    if resume:
        try:
            session_id = asyncio.run(create_session_service(data_dir()).latest_session()).id
        except ColossusError as exc:
            console.print(f"[red]{exc}[/red]")
            raise typer.Exit(code=1) from exc
    else:
        session_id = session or str(uuid4())
    overrides = _provider_overrides(ctx)
    http_client_config = _http_client_config(ctx, config)
    router = create_model_router(
        config,
        overrides,
        http_client_config=http_client_config,
    )
    resolved_approval_mode = _resolve_approval_mode(approval_mode)
    events_mode = _resolve_events_mode(events)
    trace_renderer = RichRunEventRenderer(
        console,
        enabled=events_mode != "off",
        events_mode=events_mode,
        stream_model_output=False,
    )
    service = create_research_service(
        data_dir(),
        config=config,
        model_router=router,
        workspace_root=workspace_root,
        state_store=state,
        audit_sink=audit,
        approval_handler=(
            RichApprovalHandler(console)
            if resolved_approval_mode in {"ask", "risk-auto"}
            else None
        ),
        auto_approve_network=resolved_approval_mode == "full-access",
        event_observer=trace_renderer.render,
        http_client_config=http_client_config,
    )
    trace_renderer.begin_run()
    try:
        run_result = asyncio.run(
            service.run(
                question=question,
                session_id=session_id,
                depth=research_depth,
                source_kinds=source_kinds,
                max_sources=max_sources,
            )
        )
    finally:
        trace_renderer.end_run()
    console.print(Markdown(run_result.report))
    console.print(
        f"[dim]research_id={run_result.id} session_id={session_id} "
        f"status={run_result.status}[/dim]"
    )


async def _run_agent_and_drain_subagents(
    orchestrator: AgentOrchestrator,
    subagent_service: SubagentService,
    request: AgentRunRequest,
) -> AgentRunResult:
    result = await orchestrator.run(request)
    await subagent_service.drain()
    return result


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
    session: Annotated[
        str | None,
        typer.Option("--session", help="Use or resume this exact session id."),
    ] = None,
    resume: Annotated[
        bool,
        typer.Option("--resume", help="Resume the most recently updated session."),
    ] = False,
    max_turns: Annotated[
        int | None,
        typer.Option(
            "--max-turns",
            min=1,
            max=MAX_AGENT_MAX_TURNS,
            help="Maximum model turns per REPL run.",
        ),
    ] = None,
    workspace: Annotated[
        Path | None,
        typer.Option(
            "--workspace",
            "-C",
            help="Workspace root for tools, research, memories, and context.",
        ),
    ] = None,
) -> None:
    """Start the interactive REPL."""
    if resume and session is not None:
        console.print("[red]Use either --resume or --session, not both.[/red]")
        raise typer.Exit(code=2)
    theme_dirs = (config_path().parent / "themes",)
    available_theme_names = (*repl_theme_names(), *load_user_repl_themes(theme_dirs))
    if theme is not None and theme not in available_theme_names:
        console.print(
            f"[red]Invalid REPL theme.[/red] Use {', '.join(available_theme_names)}."
        )
        raise typer.Exit(code=2)
    workspace_root = _workspace_root(workspace)
    config = load_config(config_path())
    skill_resolver = _skill_resolver(config, workspace_root)
    skill_authoring_service = create_skill_authoring_service(
        data_dir(),
        workspace_root=workspace_root,
    )
    overrides = _provider_overrides(ctx)
    http_client_config = _http_client_config(ctx, config)
    router = create_model_router(
        config,
        overrides,
        http_client_config=http_client_config,
    )
    primary_route = router.resolve("primary")
    context_route = router.resolve("context_summarizer")
    model_context_windows = _resolved_model_context_windows(config, overrides, router)
    resolved_max_turns = max_turns if max_turns is not None else config.agent.max_turns
    resolved_approval_mode = _resolve_approval_mode(approval_mode)
    state = create_state_store(data_dir())
    audit = create_audit_sink(data_dir())
    credential_broker = EnvCredentialBroker()
    pack_service = create_pack_service(data_dir(), state_store=state, audit_sink=audit)
    integration_service = create_integration_service(
        data_dir(),
        state_store=state,
        audit_sink=audit,
        credential_broker=credential_broker,
        pack_service=pack_service,
    )
    active_integration_connections = asyncio.run(integration_service.connected_connections())
    subagent_service = create_subagent_service(
        data_dir(),
        state_store=state,
        audit_sink=audit,
        max_concurrent=config.subagents.max_concurrent,
    )
    memory_service = MemoryService(state, audit, state)
    user_prompt_handler = RichUserPromptHandler(console)
    active_workspace_root = workspace_root
    context_service = create_context_service(
        data_dir(),
        workspace_root=active_workspace_root,
        state_store=state,
        audit_sink=audit,
        context_config=config.context,
        model_context_windows=model_context_windows,
        memory_service=memory_service,
    )

    def build_orchestrator(model_role: str) -> AgentOrchestrator:
        route = router.resolve(model_role)
        return create_default_orchestrator(
            data_dir(),
            route.provider,
            workspace_root=active_workspace_root,
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
            subagent_service=subagent_service,
            model_router=router,
            search_provider=create_search_provider(config.research.search, http_client_config),
            mcp_gateway=create_mcp_gateway(config.research.mcp),
            http_client_config=http_client_config,
            integration_connections=active_integration_connections,
            credential_broker=credential_broker,
            skill_resolver=skill_resolver,
            agent_max_turns=resolved_max_turns,
        )

    def build_research_service(workspace_root: Path) -> ResearchService:
        return create_research_service(
            data_dir(),
            config=config,
            model_router=router,
            workspace_root=workspace_root,
            state_store=state,
            audit_sink=audit,
            approval_handler=(
                RichApprovalHandler(console)
                if resolved_approval_mode in {"ask", "risk-auto"}
                else None
            ),
            auto_approve_network=resolved_approval_mode == "full-access",
            http_client_config=http_client_config,
        )

    research_service = build_research_service(active_workspace_root)

    async def refresh_integrations(model_role: str) -> AgentOrchestrator:
        nonlocal active_integration_connections
        active_integration_connections = await integration_service.connected_connections()
        return build_orchestrator(model_role)

    def build_workspace_services(workspace_root: Path, model_role: str) -> ReplWorkspaceServices:
        nonlocal active_workspace_root, context_service, research_service
        nonlocal skill_resolver, skill_authoring_service
        active_workspace_root = workspace_root
        skill_resolver = _skill_resolver(config, active_workspace_root)
        skill_authoring_service = create_skill_authoring_service(
            data_dir(),
            workspace_root=active_workspace_root,
        )
        context_service = create_context_service(
            data_dir(),
            workspace_root=active_workspace_root,
            state_store=state,
            audit_sink=audit,
            context_config=config.context,
            model_context_windows=model_context_windows,
            memory_service=memory_service,
        )
        research_service = build_research_service(active_workspace_root)
        return ReplWorkspaceServices(
            workspace_root=active_workspace_root,
            orchestrator=build_orchestrator(model_role),
            context_service=context_service,
            research_service=research_service,
            skill_resolver=skill_resolver,
            skill_authoring_service=skill_authoring_service,
        )

    history_path = data_dir() / "repl_history.txt"
    history_path.parent.mkdir(parents=True, exist_ok=True)
    run_repl_sync(
        build_orchestrator("primary"),
        skill_resolver,
        context_service,
        context_route.provider,
        default_agent(primary_route.profile.model, max_turns=resolved_max_turns),
        model_router=router,
        active_model_role="primary",
        orchestrator_factory=build_orchestrator,
        context_model=context_route.profile.model,
        approval_mode=resolved_approval_mode,
        history_path=history_path,
        preferences_service=create_repl_preferences_service(data_dir()),
        task_service=create_task_service(data_dir()),
        decision_service=DecisionService(state, audit),
        memory_service=memory_service,
        session_service=create_session_service(data_dir()),
        plan_service=create_plan_service(data_dir()),
        skill_authoring_service=skill_authoring_service,
        subagent_service=subagent_service,
        research_service=research_service,
        integration_service=integration_service,
        pack_service=pack_service,
        integration_refresh_factory=refresh_integrations,
        workspace_factory=build_workspace_services,
        theme_name=theme,
        initial_session_id=session,
        resume_latest=resume,
        repo_root=active_workspace_root,
        theme_dirs=theme_dirs,
    )


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
    resolver = _skill_resolver(load_config(config_path()), Path.cwd())
    table = Table("Name", "Version", "Offline", "Source")
    for skill in resolver.list_skills():
        table.add_row(
            skill.manifest.name,
            skill.manifest.version,
            str(skill.manifest.offline_compatible),
            skill.source,
        )
    console.print(table)
    duplicates = resolver.duplicate_names()
    if duplicates:
        duplicate_table = Table("Duplicate", "Selected Source", "All Sources")
        for duplicate in duplicates:
            duplicate_table.add_row(
                duplicate.name,
                duplicate.selected_source,
                "\n".join(duplicate.sources),
            )
        console.print(duplicate_table)


@skills_app.command("new")
def skills_new(
    name: Annotated[str, typer.Argument(help="Skill name to scaffold.")],
    path: Annotated[
        Path | None,
        typer.Option(
            "--path",
            help="Parent directory for the generated skill. Defaults to user data skills.",
        ),
    ] = None,
    description: Annotated[
        str | None,
        typer.Option("--description", help="Manifest description for the generated skill."),
    ] = None,
    resources: Annotated[
        str | None,
        typer.Option(
            "--resources",
            help="Comma-separated resource dirs: references,scripts,assets,examples,tests.",
        ),
    ] = None,
    agent_compatible: Annotated[
        bool,
        typer.Option("--agent-compatible", help="Add Agent Skills YAML frontmatter."),
    ] = False,
    pack: Annotated[
        Path | None,
        typer.Option("--pack", help="Pack root; skill is scaffolded under PACK/skills."),
    ] = None,
    user: Annotated[
        bool,
        typer.Option("--user", help="Create under the legacy Colossus user skill directory."),
    ] = False,
    force: Annotated[
        bool,
        typer.Option("--force", help="Overwrite manifest and SKILL.md if the skill exists."),
    ] = False,
) -> None:
    """Scaffold a local data-only skill."""
    service = create_skill_authoring_service(data_dir(), workspace_root=Path.cwd())
    explicit_targets = sum(1 for value in (path, pack) if value is not None) + int(user)
    if explicit_targets > 1:
        console.print("[red]Use only one of --path, --pack, or --user.[/red]")
        raise typer.Exit(code=2)
    parent = pack / "skills" if pack is not None else path
    if user:
        parent = service.user_skill_root
    try:
        result = service.scaffold(
            name,
            description=description,
            parent=parent,
            resources=_comma_list(resources),
            agent_compatible=agent_compatible,
            overwrite=force,
        )
    except ColossusError as exc:
        console.print(f"[red]{exc}[/red]")
        raise typer.Exit(code=1) from exc
    console.print(f"Wrote skill {result.name}: {result.path}")


def _comma_list(value: str | None) -> tuple[str, ...]:
    if value is None or not value.strip():
        return ()
    return tuple(item.strip() for item in value.split(",") if item.strip())


@skills_app.command("validate")
def skills_validate(
    path: Annotated[Path, typer.Argument(help="Skill directory to validate.")],
) -> None:
    """Validate a local skill directory."""
    service = create_skill_authoring_service(data_dir(), workspace_root=Path.cwd())
    result = service.validate(path)
    if result.valid:
        name = result.manifest.name if result.manifest is not None else path.name
        console.print(f"Skill is valid: {name} ({result.path})")
        return
    console.print(f"[red]Skill is invalid:[/red] {result.path}")
    for error in result.errors:
        console.print(f"- {error}")
    raise typer.Exit(code=1)


@skills_app.command("install")
def skills_install(
    path: Annotated[Path, typer.Argument(help="Local skill directory to install.")],
    force: Annotated[
        bool,
        typer.Option("--force", help="Overwrite an existing global skill."),
    ] = False,
) -> None:
    """Install a validated local skill into ~/.agents/skills."""
    service = create_skill_authoring_service(data_dir(), workspace_root=Path.cwd())
    try:
        result = service.install_skill(path, overwrite=force)
    except ColossusError as exc:
        console.print(f"[red]{exc}[/red]")
        raise typer.Exit(code=1) from exc
    console.print(f"Installed skill {result.name}: {result.target_path}")


@packs_app.command("list")
def packs_list() -> None:
    """List bundled and installed packs."""
    service = _pack_runtime()
    statuses = asyncio.run(service.list_statuses())
    table = Table("Name", "Version", "Publisher", "Source", "Trust", "Status", "Capabilities")
    for status in statuses:
        table.add_row(
            status.name,
            status.version,
            status.publisher,
            status.source_kind,
            status.trust_status,
            status.status,
            ", ".join(status.capabilities),
        )
    console.print(table)


@packs_app.command("show")
def packs_show(name: Annotated[str, typer.Argument(help="Pack name.")]) -> None:
    """Show pack details."""
    service = _pack_runtime()
    try:
        pack = asyncio.run(service.get_pack(name))
    except ColossusError as exc:
        console.print(f"[red]{exc}[/red]")
        raise typer.Exit(code=1) from exc
    manifest = pack.manifest
    table = Table("Field", "Value")
    table.add_row("name", manifest.name)
    table.add_row("version", manifest.version)
    table.add_row("publisher", manifest.publisher)
    table.add_row("source", pack.source)
    table.add_row("source_kind", pack.source_kind)
    table.add_row("trust", pack.trust_status)
    table.add_row("status", pack.status)
    table.add_row("capabilities", ", ".join(manifest.capabilities))
    table.add_row("permissions", ", ".join(manifest.permissions))
    table.add_row("skills", ", ".join(ref.path for ref in manifest.skills))
    table.add_row("integrations", ", ".join(ref.path for ref in manifest.integrations))
    table.add_row("mcp_servers", ", ".join(server.name for server in manifest.mcp_servers))
    table.add_row("tools", ", ".join(tool.name for tool in manifest.tools))
    console.print(table)


@packs_app.command("verify")
def packs_verify(
    source: Annotated[Path, typer.Argument(help="Pack directory or OCI layout.")],
) -> None:
    """Verify a pack source without installing it."""
    _verify_pack_source(source)


@packs_app.command("validate")
def packs_validate(
    source: Annotated[Path, typer.Argument(help="Pack directory or OCI layout.")],
) -> None:
    """Validate a pack source without installing it."""
    _verify_pack_source(source)


@packs_app.command("install")
def packs_install(
    source: Annotated[Path, typer.Argument(help="Pack directory or OCI layout.")],
    allow_untrusted: Annotated[
        bool,
        typer.Option("--allow-untrusted", help="Install an unsigned or untrusted pack."),
    ] = False,
) -> None:
    """Install a local pack."""
    service = _pack_runtime()
    try:
        installed = asyncio.run(service.install(source, allow_untrusted=allow_untrusted))
    except ColossusError as exc:
        console.print(f"[red]{exc}[/red]")
        raise typer.Exit(code=1) from exc
    console.print(
        f"Installed pack {installed.name} {installed.version}: {installed.installed_path}"
    )


@packs_app.command("enable")
def packs_enable(name: Annotated[str, typer.Argument(help="Pack name.")]) -> None:
    """Enable an installed pack."""
    service = _pack_runtime()
    try:
        updated = asyncio.run(service.enable(name))
    except ColossusError as exc:
        console.print(f"[red]{exc}[/red]")
        raise typer.Exit(code=1) from exc
    console.print(f"Enabled pack {updated.name}.")


@packs_app.command("disable")
def packs_disable(name: Annotated[str, typer.Argument(help="Pack name.")]) -> None:
    """Disable an installed pack."""
    service = _pack_runtime()
    try:
        updated = asyncio.run(service.disable(name))
    except ColossusError as exc:
        console.print(f"[red]{exc}[/red]")
        raise typer.Exit(code=1) from exc
    console.print(f"Disabled pack {updated.name}.")


@packs_app.command("uninstall")
def packs_uninstall(name: Annotated[str, typer.Argument(help="Pack name.")]) -> None:
    """Uninstall a pack."""
    service = _pack_runtime()
    try:
        asyncio.run(service.uninstall(name))
    except ColossusError as exc:
        console.print(f"[red]{exc}[/red]")
        raise typer.Exit(code=1) from exc
    console.print(f"Uninstalled pack {name}.")


@packs_trust_app.command("list")
def packs_trust_list() -> None:
    """List trusted pack publishers and keys."""
    service = _pack_runtime()
    records = asyncio.run(service.list_trust())
    table = Table("Kind", "Value", "Added")
    for record in records:
        table.add_row(record.kind, record.value, record.added_at)
    console.print(table)


@packs_trust_app.command("add")
def packs_trust_add(value: Annotated[str, typer.Argument(help="Publisher or key:KEY_ID.")]) -> None:
    """Trust a pack publisher or signing key."""
    service = _pack_runtime()
    try:
        record = asyncio.run(service.add_trust(value))
    except ColossusError as exc:
        console.print(f"[red]{exc}[/red]")
        raise typer.Exit(code=1) from exc
    console.print(f"Trusted pack {record.kind}: {record.value}")


def _verify_pack_source(source: Path) -> None:
    service = _pack_runtime()
    try:
        result = asyncio.run(service.verify(source))
    except ColossusError as exc:
        console.print(f"[red]{exc}[/red]")
        raise typer.Exit(code=1) from exc
    table = Table("Field", "Value")
    table.add_row("name", result.name)
    table.add_row("version", result.version)
    table.add_row("source_kind", result.source_kind)
    table.add_row("trust", result.trust_status)
    table.add_row("file_count", str(result.file_count))
    table.add_row("capabilities", ", ".join(result.capabilities))
    table.add_row("permissions", ", ".join(result.permissions))
    console.print(f"Pack is valid: {result.name} {result.version}")
    console.print(table)


@tools_app.command("list")
def tools_list(
    ctx: typer.Context,
    workspace: Annotated[
        Path | None,
        typer.Option(
            "--workspace",
            "-C",
            help="Workspace root used to compose workspace-bound tool handlers.",
        ),
    ] = None,
) -> None:
    """List built-in tools."""
    config = load_config(config_path())
    workspace_root = _workspace_root(workspace)
    http_client_config = _http_client_config(ctx, config)
    state = create_state_store(data_dir())
    audit = create_audit_sink(data_dir())
    credential_broker = EnvCredentialBroker()
    integration_service = create_integration_service(
        data_dir(),
        state_store=state,
        audit_sink=audit,
        credential_broker=credential_broker,
    )
    integration_connections = asyncio.run(integration_service.connected_connections())
    orchestrator = create_default_orchestrator(
        data_dir(),
        workspace_root=workspace_root,
        state_store=state,
        audit_sink=audit,
        search_provider=create_search_provider(config.research.search, http_client_config),
        mcp_gateway=create_mcp_gateway(config.research.mcp),
        http_client_config=http_client_config,
        integration_connections=integration_connections,
        credential_broker=credential_broker,
    )
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


@integrations_app.command("list")
def integrations_list() -> None:
    """List available and configured integrations."""
    service = _integration_runtime()
    _print_integration_statuses(asyncio.run(service.list_statuses()))


@integrations_app.command("show")
def integrations_show(name: Annotated[str, typer.Argument(help="Integration name.")]) -> None:
    """Show integration manifest and connection state."""
    service = _integration_runtime()
    try:
        manifest = asyncio.run(service.get_manifest(name))
        connection = asyncio.run(service.get_connection(manifest.name))
    except ColossusError as exc:
        console.print(f"[red]{exc}[/red]")
        raise typer.Exit(code=1) from exc
    table = Table("Field", "Value")
    table.add_row("name", manifest.name)
    table.add_row("title", manifest.title)
    table.add_row("kind", manifest.kind)
    table.add_row("auth", manifest.auth.type)
    table.add_row("scopes", ", ".join(manifest.auth.scopes) or "-")
    table.add_row("status", connection.status if connection is not None else "available")
    table.add_row(
        "credential_ref",
        connection.credential_ref if connection is not None and connection.credential_ref else "-",
    )
    if connection is not None and connection.credential_refs:
        table.add_row(
            "credential_refs",
            ", ".join(
                f"{key}={value}" for key, value in sorted(connection.credential_refs.items())
            ),
        )
    if connection is not None:
        for key, value in sorted(connection.config.items()):
            table.add_row(f"config.{key}", str(value))
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


@integrations_app.command("connect")
def integrations_connect(
    name: Annotated[str, typer.Argument(help="Integration name.")],
    credential_ref: Annotated[
        str | None,
        typer.Option(
            "--credential-ref",
            help="Credential handle such as env:GITHUB_TOKEN. Raw secrets are not accepted.",
        ),
    ] = None,
    scope: Annotated[
        list[str] | None,
        typer.Option("--scope", help="Scope to record for this connection. Repeatable."),
    ] = None,
    base_url: Annotated[
        str | None,
        typer.Option(
            "--base-url",
            help="Base URL for endpoint integrations such as SearXNG or OpenSearch.",
        ),
    ] = None,
    auth_type: Annotated[
        str | None,
        typer.Option(
            "--auth-type",
            help="Auth mode for endpoint integrations, for example none, bearer, or basic.",
        ),
    ] = None,
    auth_header: Annotated[
        str | None,
        typer.Option(
            "--auth-header",
            help="HTTP auth header for optional endpoint auth.",
        ),
    ] = None,
    auth_scheme: Annotated[
        str | None,
        typer.Option(
            "--auth-scheme",
            help="HTTP auth scheme for optional endpoint auth, for example bearer or raw.",
        ),
    ] = None,
    username_ref: Annotated[
        str | None,
        typer.Option(
            "--username-ref",
            help="Credential handle for username-based auth, such as env:OPENSEARCH_USER.",
        ),
    ] = None,
    password_ref: Annotated[
        str | None,
        typer.Option(
            "--password-ref",
            help="Credential handle for password-based auth, such as env:OPENSEARCH_PASSWORD.",
        ),
    ] = None,
) -> None:
    """Connect an integration using local config and optional credential refs."""
    service = _integration_runtime()
    try:
        connection = asyncio.run(
            service.connect(
                name,
                credential_ref=credential_ref,
                credential_refs=_integration_credential_refs(
                    username_ref=username_ref,
                    password_ref=password_ref,
                ),
                scopes=tuple(scope or ()),
                config=_integration_connect_config(
                    base_url=base_url,
                    auth_type=auth_type,
                    auth_header=auth_header,
                    auth_scheme=auth_scheme,
                ),
            )
        )
    except ColossusError as exc:
        console.print(f"[red]{exc}[/red]")
        raise typer.Exit(code=1) from exc
    _print_integration_connection(
        connection.name,
        connection.status,
        connection.credential_ref,
        connection.credential_refs,
    )


@integrations_app.command("disconnect")
def integrations_disconnect(
    name: Annotated[str, typer.Argument(help="Integration name.")],
) -> None:
    """Disconnect an integration."""
    service = _integration_runtime()
    try:
        asyncio.run(service.disconnect(name))
    except ColossusError as exc:
        console.print(f"[red]{exc}[/red]")
        raise typer.Exit(code=1) from exc
    console.print(f"Disconnected integration {name}.")


@integrations_app.command("import-openapi")
def integrations_import_openapi(
    name: Annotated[str, typer.Argument(help="Integration name.")],
    spec_path: Annotated[
        Path,
        typer.Argument(
            help="Path to a JSON OpenAPI document.",
            exists=True,
            file_okay=True,
            dir_okay=False,
            readable=True,
            resolve_path=True,
        ),
    ],
    base_url: Annotated[
        str | None,
        typer.Option("--base-url", help="Override the OpenAPI server URL."),
    ] = None,
    credential_ref: Annotated[
        str | None,
        typer.Option(
            "--credential-ref",
            help="Credential handle such as env:API_TOKEN. Raw secrets are not accepted.",
        ),
    ] = None,
    auth_type: Annotated[
        str,
        typer.Option(
            "--auth-type",
            help="Auth type: none, api-key, bearer, oauth2-authorization-code, or service-account.",
        ),
    ] = "bearer",
) -> None:
    """Import a JSON OpenAPI document as a brokered integration."""
    service = _integration_runtime()
    try:
        connection = asyncio.run(
            service.import_openapi(
                name,
                spec_path=spec_path,
                base_url=base_url,
                credential_ref=credential_ref,
                auth_type=_integration_auth_type(auth_type),
            )
        )
    except ColossusError as exc:
        console.print(f"[red]{exc}[/red]")
        raise typer.Exit(code=1) from exc
    _print_integration_connection(connection.name, connection.status, connection.credential_ref)


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
        http_client_config=_http_client_config(ctx, config),
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
    provider = provider_from_config(
        config,
        _provider_overrides(ctx),
        http_client_config=_http_client_config(ctx, config),
    )
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
    router = create_model_router(
        config,
        _provider_overrides(ctx),
        require_credentials=False,
        http_client_config=_http_client_config(ctx, config),
    )
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
    router = create_model_router(
        config,
        _provider_overrides(ctx),
        require_credentials=False,
        http_client_config=_http_client_config(ctx, config),
    )
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


@sessions_app.command("list")
def sessions_list(limit: Annotated[int, typer.Option("--limit")] = 20) -> None:
    """List persisted sessions by recent activity."""
    sessions = asyncio.run(create_session_service(data_dir()).list_sessions(limit))
    _print_sessions(sessions)


@sessions_app.command("show")
def sessions_show(
    session_id: Annotated[str, typer.Argument(help="Session id.")],
    limit: Annotated[int, typer.Option("--limit")] = 10,
) -> None:
    """Show a persisted session and recent messages."""
    service = create_session_service(data_dir())
    try:
        session = asyncio.run(service.require_session(session_id))
    except ColossusError as exc:
        console.print(f"[red]{exc}[/red]")
        raise typer.Exit(code=1) from exc
    messages = asyncio.run(service.recent_messages(session_id, limit=limit))
    _print_session_detail(session)
    _print_session_messages(messages)


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


@decisions_app.command("list")
def decisions_list(
    session: Annotated[str | None, typer.Option("--session")] = None,
    status: Annotated[str | None, typer.Option("--status")] = "active",
) -> None:
    """List persisted key decisions."""
    decisions = asyncio.run(
        create_decision_service(data_dir()).list_decisions(
            session_id=session,
            status=_decision_status(status),
        )
    )
    _print_decisions(decisions)


@decisions_app.command("archive")
def decisions_archive(decision_id: Annotated[str, typer.Argument(help="Decision id.")]) -> None:
    """Archive a key decision."""
    decision = asyncio.run(create_decision_service(data_dir()).archive_decision(decision_id))
    console.print(f"Archived decision {decision.id}")


@decisions_app.command("supersede")
def decisions_supersede(
    decision_id: Annotated[str, typer.Argument(help="Decision id.")],
    text: Annotated[str, typer.Argument(help="Replacement decision text.")],
) -> None:
    """Supersede a key decision with a new active decision."""
    decision = asyncio.run(
        create_decision_service(data_dir()).supersede_decision(
            decision_id,
            title=text[:80],
            decision=text,
            source="user",
            priority="normal",
        )
    )
    console.print(f"Superseded with decision {decision.id}")


@memories_app.command("list")
def memories_list(
    scope: Annotated[str | None, typer.Option("--scope")] = None,
    kind: Annotated[str | None, typer.Option("--kind")] = None,
    status: Annotated[str | None, typer.Option("--status")] = "active",
    session: Annotated[str | None, typer.Option("--session")] = None,
    repo: Annotated[str | None, typer.Option("--repo")] = None,
) -> None:
    """List durable memories."""
    memories = asyncio.run(
        create_memory_service(data_dir()).list_memories(
            scope=_memory_scope(scope),
            kind=_memory_kind(kind),
            status=_memory_status(status),
            repo_root=repo,
            session_id=session,
        )
    )
    _print_memories(memories)


@memories_app.command("search")
def memories_search(
    query: Annotated[str, typer.Argument(help="Search query.")],
    kind: Annotated[str | None, typer.Option("--kind")] = None,
    status: Annotated[str | None, typer.Option("--status")] = "active",
    session: Annotated[str | None, typer.Option("--session")] = None,
    repo: Annotated[str | None, typer.Option("--repo")] = None,
    limit: Annotated[int, typer.Option("--limit")] = 8,
) -> None:
    """Search durable memories."""
    memories = asyncio.run(
        create_memory_service(data_dir()).search_memories(
            query,
            repo_root=repo or str(Path.cwd()),
            session_id=session,
            kind=_memory_kind(kind),
            status=_memory_status(status),
            limit=limit,
        )
    )
    _print_memories(memories)


@memories_app.command("archive")
def memories_archive(memory_id: Annotated[str, typer.Argument(help="Memory id.")]) -> None:
    """Archive a durable memory."""
    memory = asyncio.run(create_memory_service(data_dir()).archive_memory(memory_id))
    console.print(f"Archived memory {memory.id}")


@memories_app.command("supersede")
def memories_supersede(
    memory_id: Annotated[str, typer.Argument(help="Memory id.")],
    text: Annotated[str, typer.Argument(help="Replacement memory text.")],
) -> None:
    """Supersede a durable memory with a new active memory."""
    memory = asyncio.run(
        create_memory_service(data_dir()).supersede_memory(
            memory_id,
            text=text,
            source="user",
        )
    )
    console.print(f"Superseded with memory {memory.id}")


@agents_app.command("list")
def agents_list(
    session: Annotated[str | None, typer.Option("--session")] = None,
    status: Annotated[str | None, typer.Option("--status")] = None,
) -> None:
    """List durable subagent jobs."""
    config = load_config(config_path())
    service = create_subagent_service(
        data_dir(),
        max_concurrent=config.subagents.max_concurrent,
    )
    jobs = asyncio.run(service.list_jobs(session_id=session, status=_subagent_status(status)))
    _print_subagents(jobs)


@agents_app.command("show")
def agents_show(job_id: Annotated[str, typer.Argument(help="Subagent job id.")]) -> None:
    """Show a durable subagent job."""
    config = load_config(config_path())
    service = create_subagent_service(
        data_dir(),
        max_concurrent=config.subagents.max_concurrent,
    )
    job = asyncio.run(service.get_job(job_id))
    _print_subagents((job,))
    if job.final_output:
        console.print(Markdown(job.final_output))
    if job.error:
        console.print(f"[red]{job.error}[/red]")


@agents_app.command("cancel")
def agents_cancel(job_id: Annotated[str, typer.Argument(help="Subagent job id.")]) -> None:
    """Cancel a queued or running subagent job."""
    config = load_config(config_path())
    service = create_subagent_service(
        data_dir(),
        max_concurrent=config.subagents.max_concurrent,
    )
    job = asyncio.run(service.cancel_job(job_id))
    console.print(f"Cancelled subagent {job.id}: {job.status}")


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


def _print_sessions(sessions: tuple[SessionSummary, ...]) -> None:
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


def _print_session_detail(session: SessionSummary) -> None:
    table = Table("Field", "Value")
    table.add_row("id", session.id)
    table.add_row("title", session.title or "")
    table.add_row("created_at", session.created_at)
    table.add_row("updated_at", session.updated_at)
    table.add_row("message_count", str(session.message_count))
    table.add_row("last_run_id", session.last_run_id or "")
    table.add_row("last_user_preview", session.last_user_preview or "")
    console.print(table)


def _print_session_messages(messages: tuple[Message, ...]) -> None:
    if not messages:
        console.print("No messages.")
        return
    table = Table("Role", "Preview")
    for message in messages:
        table.add_row(_message_role(message), _message_preview(message))
    console.print(table)


def _message_role(message: Message) -> str:
    if isinstance(message, ToolResultMessage):
        return f"tool:{message.name}"
    return message.role


def _message_preview(message: Message, limit: int = 120) -> str:
    if isinstance(message, UserMessage | AssistantMessage | ToolResultMessage):
        value = message.content
    else:
        value = ""
    normalized = " ".join(value.split())
    if len(normalized) <= limit:
        return normalized
    return normalized[: limit - 3].rstrip() + "..."


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


def _print_decisions(decisions: tuple[KeyDecision, ...]) -> None:
    if not decisions:
        console.print("No key decisions.")
        return
    table = Table("Status", "Priority", "ID", "Session", "Source", "Decision")
    for decision in decisions:
        table.add_row(
            decision.status,
            decision.priority,
            decision.id,
            decision.session_id,
            decision.source,
            decision.decision[:100],
        )
    console.print(table)


def _print_memories(memories: tuple[MemoryItem, ...]) -> None:
    if not memories:
        console.print("No memories.")
        return
    table = Table("Status", "Scope", "Kind", "ID", "Source", "Memory")
    for memory in memories:
        table.add_row(
            memory.status,
            memory.scope,
            memory.kind,
            memory.id,
            memory.source,
            memory.text[:100],
        )
    console.print(table)


def _print_subagents(jobs: tuple[SubagentJob, ...]) -> None:
    if not jobs:
        console.print("No subagent jobs.")
        return
    table = Table("Status", "ID", "Role", "Task", "Child Run", "Updated")
    for job in jobs:
        table.add_row(
            job.status,
            job.id,
            job.role,
            job.task[:80],
            job.child_run_id or "",
            job.updated_at,
        )
    console.print(table)


def _subagent_status(value: str | None) -> SubagentStatus | None:
    if value is None:
        return None
    normalized = value.strip().lower()
    if normalized in {"queued", "running", "completed", "failed", "cancelled", "interrupted"}:
        return cast(SubagentStatus, normalized)
    console.print("[red]Invalid subagent status.[/red]")
    raise typer.Exit(code=2)


def _decision_status(value: str | None) -> DecisionStatus | None:
    if value is None or value.strip().lower() in {"", "all", "*"}:
        return None
    normalized = value.strip().lower()
    if normalized in {"active", "archived", "superseded"}:
        return cast(DecisionStatus, normalized)
    console.print("[red]Invalid decision status.[/red]")
    raise typer.Exit(code=2)


def _memory_status(value: str | None) -> MemoryStatus | None:
    if value is None or value.strip().lower() in {"", "all", "*"}:
        return None
    normalized = value.strip().lower()
    if normalized in {"active", "archived", "superseded"}:
        return cast(MemoryStatus, normalized)
    console.print("[red]Invalid memory status.[/red]")
    raise typer.Exit(code=2)


def _memory_scope(value: str | None) -> MemoryScope | None:
    if value is None or value.strip().lower() in {"", "all", "*"}:
        return None
    normalized = value.strip().lower()
    if normalized in {"global", "repo", "session"}:
        return cast(MemoryScope, normalized)
    console.print("[red]Invalid memory scope.[/red]")
    raise typer.Exit(code=2)


def _memory_kind(value: str | None) -> MemoryKind | None:
    if value is None or value.strip().lower() in {"", "all", "*"}:
        return None
    normalized = value.strip().lower()
    if normalized in {"preference", "project_fact", "episode", "capability", "warning"}:
        return cast(MemoryKind, normalized)
    console.print("[red]Invalid memory kind.[/red]")
    raise typer.Exit(code=2)


def main() -> None:
    try:
        app()
    except ColossusError as exc:
        console.print(f"[red]Error:[/red] {exc}")
        raise typer.Exit(code=1) from None
