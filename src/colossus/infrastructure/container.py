"""Default service composition for the CLI surfaces."""

from pathlib import Path

from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.builtin_tools import create_builtin_tools
from colossus.adapters.echo_provider import EchoModelProvider
from colossus.adapters.skills_package import PackageSkillRepository
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.adapters.workspace import Workspace
from colossus.application.approvals import DenyByDefaultApprovalHandler
from colossus.application.context import ContextService
from colossus.application.model_router import ModelRoute, ModelRouter
from colossus.application.orchestrator import AgentOrchestrator, RunEventObserver
from colossus.application.planning import PlanService
from colossus.application.policy import DefaultPolicyEngine
from colossus.application.preferences import ReplPreferencesService
from colossus.application.risk import RiskAssessmentService
from colossus.application.skills import SkillResolver
from colossus.application.tasks import TaskService
from colossus.application.tools import FunctionToolExecutor, InMemoryToolRegistry
from colossus.domain.context import ContextConfig
from colossus.domain.models import ResolvedModelProfile
from colossus.infrastructure.config import (
    ColossusConfig,
    ProviderOverrides,
    effective_model_routing,
    provider_from_profile,
)
from colossus.ports.approval import ApprovalHandler
from colossus.ports.model_provider import ModelProvider
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
) -> AgentOrchestrator:
    data_dir.mkdir(parents=True, exist_ok=True)
    resolved_provider = provider or EchoModelProvider()
    resolved_state = state_store or create_state_store(data_dir)
    resolved_audit = audit_sink or create_audit_sink(data_dir)
    resolved_context = context_service or create_context_service(
        data_dir,
        state_store=resolved_state,
        audit_sink=resolved_audit,
        context_config=context_config,
        model_context_windows=model_context_windows,
    )
    workspace = Workspace(workspace_root or Path.cwd())
    specs, handlers = create_builtin_tools(
        workspace,
        context_service=resolved_context,
        context_provider=context_provider or resolved_provider,
        context_model=context_model,
        task_service=TaskService(resolved_state, resolved_audit),
        user_prompt_handler=user_prompt_handler,
    )
    registry = InMemoryToolRegistry(specs)
    executor = FunctionToolExecutor(handlers, registry)
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
    )


def create_model_router(
    config: ColossusConfig,
    overrides: ProviderOverrides | None = None,
    *,
    require_credentials: bool = True,
) -> ModelRouter:
    overrides = overrides or ProviderOverrides()
    routing = effective_model_routing(config, overrides)
    primary_profile_name = routing.roles["primary"]
    routes: dict[str, ModelRoute] = {}
    for role, profile_name in routing.roles.items():
        profile = routing.profiles[profile_name]
        provider = provider_from_profile(
            profile,
            api_key=overrides.api_key if profile_name == primary_profile_name else None,
            require_credentials=require_credentials,
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
    state_store: SQLiteStateStore | None = None,
    audit_sink: JsonlAuditSink | None = None,
    context_config: ContextConfig | None = None,
    model_context_windows: dict[str, int] | None = None,
) -> ContextService:
    return ContextService(
        state_store or create_state_store(data_dir),
        audit_sink or create_audit_sink(data_dir),
        config=context_config,
        model_context_windows=model_context_windows,
    )


def create_plan_service(data_dir: Path) -> PlanService:
    return PlanService(create_state_store(data_dir), create_audit_sink(data_dir))


def create_task_service(data_dir: Path) -> TaskService:
    return TaskService(create_state_store(data_dir), create_audit_sink(data_dir))


def create_repl_preferences_service(data_dir: Path) -> ReplPreferencesService:
    return ReplPreferencesService(create_state_store(data_dir))


def create_default_skill_resolver() -> SkillResolver:
    return SkillResolver((PackageSkillRepository(),))
