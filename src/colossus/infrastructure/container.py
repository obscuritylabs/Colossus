"""Default service composition for the CLI surfaces."""

import os
from collections.abc import Awaitable, Callable
from pathlib import Path

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.builtin_tools import create_builtin_tools
from colossus.adapters.credentials_env import EnvCredentialBroker
from colossus.adapters.echo_provider import EchoModelProvider
from colossus.adapters.integration_tools import create_integration_tools
from colossus.adapters.packs import (
    PackagePackRepository,
    PackInstaller,
    PackIntegrationManifestLoader,
    PackSkillRepository,
    write_installed_pack_marker,
)
from colossus.adapters.research_sources import (
    DisabledMcpGateway,
    DisabledSearchProvider,
    DuckDuckGoSearchProvider,
    McpResearchToolRuntime,
    McpSdkGateway,
    McpServerRuntime,
    SearxngSearchProvider,
    WorkspaceRepoResearchProvider,
)
from colossus.adapters.skills_filesystem import FilesystemSkillRepository, WorkspaceSkillRepository
from colossus.adapters.skills_package import PackageSkillRepository
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.adapters.workspace import Workspace
from colossus.application.approvals import DenyByDefaultApprovalHandler
from colossus.application.context import ContextService
from colossus.application.decisions import DecisionService
from colossus.application.defaults import default_agent
from colossus.application.integrations import IntegrationService
from colossus.application.memories import MemoryService
from colossus.application.model_router import ModelRoute, ModelRouter
from colossus.application.orchestrator import AgentOrchestrator, RunEventObserver
from colossus.application.packs import PackService
from colossus.application.planning import PlanService
from colossus.application.policy import DefaultPolicyEngine
from colossus.application.preferences import ReplPreferencesService
from colossus.application.research import ResearchService
from colossus.application.risk import RiskAssessmentService
from colossus.application.sessions import SessionService
from colossus.application.skill_authoring import SkillAuthoringService
from colossus.application.skills import SkillComposer, SkillResolver, SkillResourceService
from colossus.application.subagents import SubagentService
from colossus.application.tasks import TaskService
from colossus.application.tools import FunctionToolExecutor, InMemoryToolRegistry
from colossus.domain.agents import DEFAULT_AGENT_MAX_TURNS
from colossus.domain.context import ContextConfig
from colossus.domain.integrations import IntegrationConnection
from colossus.domain.models import ResolvedModelProfile
from colossus.domain.requests import AgentRunRequest, AgentRunResult
from colossus.domain.subagents import SubagentJob
from colossus.infrastructure.config import (
    ColossusConfig,
    McpConfig,
    ProviderOverrides,
    SearchConfig,
    effective_model_routing,
    http_client_config_from_config,
    provider_from_profile,
)
from colossus.infrastructure.http_client import HttpClientConfig
from colossus.ports.approval import ApprovalHandler
from colossus.ports.credentials import CredentialBroker
from colossus.ports.model_provider import ModelProvider
from colossus.ports.research import McpGateway, SearchProvider
from colossus.ports.skills import SkillRepository
from colossus.ports.user_prompt import UserPromptHandler


def create_default_orchestrator(
    data_dir: Path,
    provider: ModelProvider | None = None,
    workspace_root: Path | None = None,
    state_store: SQLiteStateStore | None = None,
    audit_sink: JsonlAuditSink | None = None,
    context_service: ContextService | None = None,
    context_config: ContextConfig | None = None,
    model_context_windows: dict[str, int] | None = None,
    context_model: str = "default",
    context_provider: ModelProvider | None = None,
    event_observer: RunEventObserver | None = None,
    approval_handler: ApprovalHandler | None = None,
    user_prompt_handler: UserPromptHandler | None = None,
    risk_assessment_service: RiskAssessmentService | None = None,
    risk_auto_approve: bool = False,
    auto_approve_required_tools: bool = False,
    subagent_service: SubagentService | None = None,
    memory_service: MemoryService | None = None,
    model_router: ModelRouter | None = None,
    include_agent_delegate: bool = True,
    search_provider: SearchProvider | None = None,
    mcp_gateway: McpGateway | None = None,
    skill_resolver: SkillResolver | None = None,
    http_client_config: HttpClientConfig | None = None,
    integration_connections: tuple[IntegrationConnection, ...] = (),
    credential_broker: CredentialBroker | None = None,
    agent_max_turns: int = DEFAULT_AGENT_MAX_TURNS,
) -> AgentOrchestrator:
    data_dir.mkdir(parents=True, exist_ok=True)
    resolved_provider = provider or EchoModelProvider()
    resolved_state = state_store or create_state_store(data_dir)
    resolved_audit = audit_sink or create_audit_sink(data_dir)
    decision_service = DecisionService(resolved_state, resolved_audit)
    resolved_memory = memory_service or MemoryService(
        resolved_state,
        resolved_audit,
        resolved_state,
    )
    resolved_context = context_service or create_context_service(
        data_dir,
        workspace_root=workspace_root,
        state_store=resolved_state,
        audit_sink=resolved_audit,
        context_config=context_config,
        model_context_windows=model_context_windows,
        memory_service=resolved_memory,
    )
    workspace = Workspace(workspace_root or Path.cwd())
    resolved_skill_resolver = skill_resolver or create_default_skill_resolver(
        data_dir / "skills",
        pack_root=data_dir / "packs",
        workspace_root=workspace_root,
    )
    specs, handlers = create_builtin_tools(
        workspace,
        context_service=resolved_context,
        context_provider=context_provider or resolved_provider,
        context_model=context_model,
        task_service=TaskService(resolved_state, resolved_audit),
        decision_service=decision_service,
        memory_service=resolved_memory,
        subagent_service=subagent_service,
        include_agent_delegate=include_agent_delegate,
        user_prompt_handler=user_prompt_handler,
        search_provider=search_provider,
        mcp_gateway=mcp_gateway,
        http_client_config=http_client_config,
        skill_authoring_service=create_skill_authoring_service(
            data_dir,
            workspace_root=workspace_root,
        ),
        skill_resource_service=SkillResourceService(resolved_skill_resolver),
        audit_sink=resolved_audit,
    )
    resolved_credential_broker = credential_broker or EnvCredentialBroker()
    integration_specs, integration_handlers = create_integration_tools(
        integration_connections,
        resolved_credential_broker,
        audit_sink=resolved_audit,
        http_client_config=http_client_config,
    )
    specs = (*specs, *integration_specs)
    handlers = {**handlers, **integration_handlers}
    registry = InMemoryToolRegistry(specs)
    executor = FunctionToolExecutor(handlers, registry)
    if subagent_service is not None and model_router is not None:
        if event_observer is not None:
            subagent_service.set_event_observer(event_observer)
        subagent_service.set_runner(
            _subagent_runner(
                data_dir=data_dir,
                workspace_root=workspace_root,
                state_store=resolved_state,
                audit_sink=resolved_audit,
                context_service=resolved_context,
                context_config=context_config,
                model_context_windows=model_context_windows,
                context_model=context_model,
                context_provider=context_provider or resolved_provider,
                memory_service=resolved_memory,
                approval_handler=approval_handler,
                user_prompt_handler=user_prompt_handler,
                risk_assessment_service=risk_assessment_service,
                risk_auto_approve=risk_auto_approve,
                auto_approve_required_tools=auto_approve_required_tools,
                subagent_service=subagent_service,
                model_router=model_router,
                skill_resolver=resolved_skill_resolver,
                http_client_config=http_client_config,
                integration_connections=integration_connections,
                credential_broker=resolved_credential_broker,
                agent_max_turns=agent_max_turns,
            )
        )
    return AgentOrchestrator(
        provider=resolved_provider,
        tool_registry=registry,
        tool_executor=executor,
        policy_engine=DefaultPolicyEngine(),
        approval_handler=approval_handler or DenyByDefaultApprovalHandler(),
        state_store=resolved_state,
        audit_sink=resolved_audit,
        context_service=resolved_context,
        context_provider=context_provider or resolved_provider,
        context_model=context_model,
        event_observer=event_observer,
        risk_assessment_service=risk_assessment_service,
        risk_auto_approve=risk_auto_approve,
        auto_approve_required_tools=auto_approve_required_tools,
        subagent_service=subagent_service,
        decision_service=decision_service,
        skill_composer=SkillComposer(resolved_skill_resolver),
    )


def create_subagent_service(
    data_dir: Path,
    *,
    state_store: SQLiteStateStore | None = None,
    audit_sink: JsonlAuditSink | None = None,
    max_concurrent: int = 4,
) -> SubagentService:
    data_dir.mkdir(parents=True, exist_ok=True)
    return SubagentService(
        state_store or create_state_store(data_dir),
        audit_sink or create_audit_sink(data_dir),
        max_concurrent=max_concurrent,
    )


def create_integration_service(
    data_dir: Path,
    *,
    state_store: SQLiteStateStore | None = None,
    audit_sink: JsonlAuditSink | None = None,
    credential_broker: CredentialBroker | None = None,
    pack_service: PackService | None = None,
) -> IntegrationService:
    data_dir.mkdir(parents=True, exist_ok=True)
    resolved_state = state_store or create_state_store(data_dir)
    resolved_audit = audit_sink or create_audit_sink(data_dir)
    return IntegrationService(
        resolved_state,
        resolved_audit,
        credential_broker or EnvCredentialBroker(),
        pack_service=pack_service
        or create_pack_service(data_dir, state_store=resolved_state, audit_sink=resolved_audit),
    )


def create_pack_service(
    data_dir: Path,
    *,
    state_store: SQLiteStateStore | None = None,
    audit_sink: JsonlAuditSink | None = None,
) -> PackService:
    data_dir.mkdir(parents=True, exist_ok=True)
    return PackService(
        state_store or create_state_store(data_dir),
        audit_sink or create_audit_sink(data_dir),
        PackInstaller(data_dir / "packs"),
        PackagePackRepository(),
        PackIntegrationManifestLoader(),
        marker_writer=write_installed_pack_marker,
    )


def create_search_provider(
    config: SearchConfig,
    http_client_config: HttpClientConfig | None = None,
) -> SearchProvider:
    if config.kind == "duckduckgo":
        return DuckDuckGoSearchProvider(
            endpoint=config.endpoint,
            user_agent=config.user_agent,
            http_client_config=http_client_config,
        )
    if config.kind == "searxng":
        return SearxngSearchProvider(
            endpoint=config.endpoint,
            user_agent=config.user_agent,
            api_key=os.environ.get(config.api_key_env, "") if config.api_key_env else None,
            auth_header=config.auth_header,
            auth_scheme=config.auth_scheme,
            http_client_config=http_client_config,
        )
    return DisabledSearchProvider()


def create_mcp_gateway(config: McpConfig) -> McpGateway:
    if not config.servers:
        return DisabledMcpGateway()
    servers = []
    for name, server in config.servers.items():
        servers.append(
            McpServerRuntime(
                name=name,
                command=server.command,
                args=server.args,
                env=server.env,
                allowed_tools=server.allowed_tools,
                research_tools=tuple(
                    McpResearchToolRuntime(
                        server=name,
                        tool=tool.tool,
                        arguments=tool.arguments,
                        title=tool.title,
                    )
                    for tool in server.research_tools
                ),
            )
        )
    return McpSdkGateway(tuple(servers))


def create_research_service(
    data_dir: Path,
    *,
    config: ColossusConfig,
    model_router: ModelRouter | None = None,
    workspace_root: Path | None = None,
    state_store: SQLiteStateStore | None = None,
    audit_sink: JsonlAuditSink | None = None,
    approval_handler: ApprovalHandler | None = None,
    auto_approve_network: bool = False,
    event_observer: RunEventObserver | None = None,
    http_client_config: HttpClientConfig | None = None,
) -> ResearchService:
    resolved_state = state_store or create_state_store(data_dir)
    resolved_audit = audit_sink or create_audit_sink(data_dir)
    workspace = Workspace(workspace_root or Path.cwd())
    return ResearchService(
        resolved_state,
        resolved_audit,
        repo_provider=WorkspaceRepoResearchProvider(workspace),
        model_router=model_router,
        search_provider=create_search_provider(config.research.search, http_client_config),
        mcp_gateway=create_mcp_gateway(config.research.mcp),
        approval_handler=approval_handler,
        auto_approve_network=auto_approve_network,
        event_observer=event_observer,
    )


def _subagent_runner(
    *,
    data_dir: Path,
    workspace_root: Path | None,
    state_store: SQLiteStateStore,
    audit_sink: JsonlAuditSink,
    context_service: ContextService,
    context_config: ContextConfig | None,
    model_context_windows: dict[str, int] | None,
    context_model: str,
    context_provider: ModelProvider,
    memory_service: MemoryService,
    approval_handler: ApprovalHandler | None,
    user_prompt_handler: UserPromptHandler | None,
    risk_assessment_service: RiskAssessmentService | None,
    risk_auto_approve: bool,
    auto_approve_required_tools: bool,
    subagent_service: SubagentService,
    model_router: ModelRouter,
    skill_resolver: SkillResolver,
    http_client_config: HttpClientConfig | None,
    integration_connections: tuple[IntegrationConnection, ...],
    credential_broker: CredentialBroker,
    agent_max_turns: int,
) -> Callable[[SubagentJob], Awaitable[AgentRunResult]]:
    async def run(job: SubagentJob) -> AgentRunResult:
        route = model_router.resolve(job.role or "subagent_default")
        orchestrator = create_default_orchestrator(
            data_dir,
            route.provider,
            workspace_root=workspace_root,
            state_store=state_store,
            audit_sink=audit_sink,
            context_service=context_service,
            context_config=context_config,
            model_context_windows=model_context_windows,
            context_model=context_model,
            context_provider=context_provider,
            memory_service=memory_service,
            approval_handler=approval_handler,
            user_prompt_handler=user_prompt_handler,
            risk_assessment_service=risk_assessment_service,
            risk_auto_approve=risk_auto_approve,
            auto_approve_required_tools=auto_approve_required_tools,
            subagent_service=subagent_service,
            model_router=model_router,
            include_agent_delegate=False,
            skill_resolver=skill_resolver,
            http_client_config=http_client_config,
            integration_connections=integration_connections,
            credential_broker=credential_broker,
        )
        return await orchestrator.run(
            AgentRunRequest(
                prompt=job.task,
                agent=default_agent(route.profile.model, max_turns=agent_max_turns),
                session_id=job.child_session_id,
            )
        )

    return run


def create_model_router(
    config: ColossusConfig,
    overrides: ProviderOverrides | None = None,
    *,
    require_credentials: bool = True,
    http_client_config: HttpClientConfig | None = None,
) -> ModelRouter:
    overrides = overrides or ProviderOverrides()
    resolved_http = http_client_config or http_client_config_from_config(config)
    routing = effective_model_routing(config, overrides)
    primary_profile_name = routing.roles["primary"]
    routes: dict[str, ModelRoute] = {}
    for role, profile_name in routing.roles.items():
        profile = routing.profiles[profile_name]
        provider = provider_from_profile(
            profile,
            api_key=overrides.api_key if profile_name == primary_profile_name else None,
            require_credentials=require_credentials,
            http_client_config=resolved_http,
        )
        resolved = ResolvedModelProfile(
            role=role,
            profile_name=profile_name,
            provider=profile.provider,
            model=profile.model,
            base_url=profile.base_url,
            api_key_env=profile.api_key_env,
            ca_bundle=profile.ca_bundle,
            context_window_tokens=profile.context_window_tokens,
        )
        routes[role] = ModelRoute(
            role=role,
            profile_name=profile_name,
            provider=provider,
            profile=resolved,
        )
    return ModelRouter(routes)


def create_state_store(data_dir: Path) -> SQLiteStateStore:
    return SQLiteStateStore(data_dir / "state.sqlite3")


def create_audit_sink(data_dir: Path) -> JsonlAuditSink:
    return JsonlAuditSink(data_dir / "audit.jsonl")


def create_context_service(
    data_dir: Path,
    *,
    workspace_root: Path | None = None,
    state_store: SQLiteStateStore | None = None,
    audit_sink: JsonlAuditSink | None = None,
    context_config: ContextConfig | None = None,
    model_context_windows: dict[str, int] | None = None,
    memory_service: MemoryService | None = None,
) -> ContextService:
    resolved_state = state_store or create_state_store(data_dir)
    resolved_audit = audit_sink or create_audit_sink(data_dir)
    return ContextService(
        resolved_state,
        resolved_audit,
        config=context_config,
        model_context_windows=model_context_windows,
        memory_service=memory_service
        or MemoryService(
            resolved_state,
            resolved_audit,
            resolved_state,
        ),
        repo_root=str(workspace_root or Path.cwd()),
    )


def create_plan_service(data_dir: Path) -> PlanService:
    return PlanService(create_state_store(data_dir), create_audit_sink(data_dir))


def create_task_service(data_dir: Path) -> TaskService:
    return TaskService(create_state_store(data_dir), create_audit_sink(data_dir))


def create_decision_service(data_dir: Path) -> DecisionService:
    return DecisionService(create_state_store(data_dir), create_audit_sink(data_dir))


def create_memory_service(data_dir: Path) -> MemoryService:
    state = create_state_store(data_dir)
    return MemoryService(state, create_audit_sink(data_dir), state)


def create_session_service(data_dir: Path) -> SessionService:
    return SessionService(create_state_store(data_dir))


def create_repl_preferences_service(data_dir: Path) -> ReplPreferencesService:
    return ReplPreferencesService(create_state_store(data_dir))


def create_skill_authoring_service(
    data_dir: Path,
    *,
    workspace_root: Path | None = None,
) -> SkillAuthoringService:
    workspace_skill_root = (
        workspace_root.resolve(strict=False) / ".agents" / "skills"
        if workspace_root is not None
        else None
    )
    return SkillAuthoringService(
        data_dir / "skills",
        workspace_skill_root=workspace_skill_root,
    )


def create_default_skill_resolver(
    user_skill_root: Path | None = None,
    *,
    allow_user_overrides: bool = False,
    pack_root: Path | None = None,
    workspace_root: Path | None = None,
    global_skill_root: Path | None = None,
) -> SkillResolver:
    repositories: list[SkillRepository] = [
        PackageSkillRepository(),
        PackSkillRepository(
            pack_root,
            bundled_repository=PackagePackRepository(),
        ),
    ]
    if user_skill_root is not None:
        repositories.append(FilesystemSkillRepository(user_skill_root))
    resolved_global_skill_root = global_skill_root or Path.home() / ".agents" / "skills"
    repositories.append(FilesystemSkillRepository(resolved_global_skill_root))
    if workspace_root is not None:
        repositories.append(WorkspaceSkillRepository(workspace_root))
    return SkillResolver(tuple(repositories), allow_user_overrides=allow_user_overrides)
