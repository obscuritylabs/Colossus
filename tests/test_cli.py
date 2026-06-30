import asyncio
import json
import sqlite3
from pathlib import Path

import typer
from typer.testing import CliRunner

import colossus.cli as cli_module
from colossus.adapters.audit_jsonl import JsonlAuditSink
from colossus.adapters.sqlite_state import SQLiteStateStore
from colossus.application.model_router import ModelRoute, ModelRouter
from colossus.application.subagents import SubagentService
from colossus.cli import app
from colossus.domain.errors import ColossusError
from colossus.domain.messages import UserMessage
from colossus.domain.models import ModelProfile, ModelRoutingConfig, ResolvedModelProfile
from colossus.domain.providers import ProviderModelInfo
from colossus.domain.requests import AgentRunRequest, AgentRunResult
from colossus.domain.subagents import SubagentJob
from colossus.infrastructure.config import (
    AgentConfig,
    ColossusConfig,
    ProviderConfig,
    ResearchConfig,
    SearchConfig,
)
from colossus.infrastructure.http_client import HttpClientConfig
from colossus.infrastructure.paths import config_path


def test_cli_run_uses_echo_provider(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    result = CliRunner().invoke(app, ["run", "hello"])

    assert result.exit_code == 0
    assert "[echo:default] hello" in result.stdout


def test_cli_run_streams_without_duplicate_final_output(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))

    result = CliRunner().invoke(app, ["run", "--stream", "hello"])

    assert result.exit_code == 0
    assert result.stdout.count("[echo:default] hello") == 1
    assert "run_id=" in result.stdout


def test_cli_run_drains_subagents_before_returning(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    drained = False

    async def drain(self) -> None:
        nonlocal drained
        drained = True

    monkeypatch.setattr(cli_module.SubagentService, "drain", drain)

    result = CliRunner().invoke(app, ["run", "hello"])

    assert result.exit_code == 0
    assert drained is True


def test_cli_run_accepts_repeatable_skill_option(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    captured: dict[str, AgentRunRequest] = {}

    async def fake_run_and_drain(orchestrator, subagent_service, request):
        del orchestrator, subagent_service
        captured["request"] = request
        return AgentRunResult(
            run_id="run-1",
            final_output="done",
            events_recorded=0,
            session_id=request.session_id,
        )

    monkeypatch.setattr(cli_module, "_run_agent_and_drain_subagents", fake_run_and_drain)

    result = CliRunner().invoke(app, ["run", "--skill", "coding", "hello"])

    assert result.exit_code == 0
    assert captured["request"].active_skills == ("coding",)
    assert captured["request"].skill_mode_enabled is True


def test_cli_run_accepts_max_turns_override(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    captured: dict[str, AgentRunRequest] = {}

    async def fake_run_and_drain(orchestrator, subagent_service, request):
        del orchestrator, subagent_service
        captured["request"] = request
        return AgentRunResult(
            run_id="run-1",
            final_output="done",
            events_recorded=0,
            session_id=request.session_id,
        )

    monkeypatch.setattr(cli_module, "_run_agent_and_drain_subagents", fake_run_and_drain)

    result = CliRunner().invoke(app, ["run", "--max-turns", "40", "hello"])

    assert result.exit_code == 0
    assert captured["request"].agent.max_turns == 40


def test_cli_run_uses_configured_max_turns(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "config"))
    _write_config(ColossusConfig(agent=AgentConfig(max_turns=36)))
    captured: dict[str, AgentRunRequest] = {}

    async def fake_run_and_drain(orchestrator, subagent_service, request):
        del orchestrator, subagent_service
        captured["request"] = request
        return AgentRunResult(
            run_id="run-1",
            final_output="done",
            events_recorded=0,
            session_id=request.session_id,
        )

    monkeypatch.setattr(cli_module, "_run_agent_and_drain_subagents", fake_run_and_drain)

    result = CliRunner().invoke(app, ["run", "hello"])

    assert result.exit_code == 0
    assert captured["request"].agent.max_turns == 36


def test_cli_run_accepts_workspace_option(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    captured: dict[str, Path | None] = {}
    create_default_orchestrator = cli_module.create_default_orchestrator

    def capture_orchestrator(*args, **kwargs):
        captured["workspace_root"] = kwargs.get("workspace_root")
        return create_default_orchestrator(*args, **kwargs)

    monkeypatch.setattr(cli_module, "create_default_orchestrator", capture_orchestrator)

    result = CliRunner().invoke(app, ["run", "--workspace", str(workspace), "hello"])

    assert result.exit_code == 0
    assert captured["workspace_root"] == workspace.resolve()


def test_cli_run_rejects_unknown_skill(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))

    result = CliRunner().invoke(app, ["run", "--skill", "missing", "hello"])

    assert result.exit_code == 1
    assert "Unknown skill" in result.stdout


def test_cli_run_rejects_unknown_events_mode(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))

    result = CliRunner().invoke(app, ["run", "--events", "firehose", "hello"])

    assert result.exit_code == 2
    assert "Invalid events mode" in result.stdout


def test_cli_run_ask_approval_wires_interactive_handler(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    captured: dict[str, object] = {}
    create_default_orchestrator = cli_module.create_default_orchestrator

    def capture_orchestrator(*args, **kwargs):
        captured["approval_handler"] = kwargs.get("approval_handler")
        return create_default_orchestrator(*args, **kwargs)

    monkeypatch.setattr(cli_module, "create_default_orchestrator", capture_orchestrator)

    result = CliRunner().invoke(app, ["run", "--ask-approval", "hello"])

    assert result.exit_code == 0
    assert isinstance(captured["approval_handler"], cli_module.RichApprovalHandler)
    assert "[echo:default] hello" in result.stdout


def test_cli_run_risk_auto_approval_mode_sets_risk_auto_flag(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    captured: dict[str, object] = {}
    create_default_orchestrator = cli_module.create_default_orchestrator

    def capture_orchestrator(*args, **kwargs):
        captured["approval_handler"] = kwargs.get("approval_handler")
        captured["risk_auto_approve"] = kwargs.get("risk_auto_approve")
        return create_default_orchestrator(*args, **kwargs)

    monkeypatch.setattr(cli_module, "create_default_orchestrator", capture_orchestrator)

    result = CliRunner().invoke(app, ["run", "--approval-mode", "risk-auto", "hello"])

    assert result.exit_code == 0
    assert isinstance(captured["approval_handler"], cli_module.RichApprovalHandler)
    assert captured["risk_auto_approve"] is True
    assert "[echo:default] hello" in result.stdout


def test_cli_run_full_access_approval_mode_sets_auto_approval_flag(
    tmp_path, monkeypatch
) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    captured: dict[str, object] = {}
    create_default_orchestrator = cli_module.create_default_orchestrator

    def capture_orchestrator(*args, **kwargs):
        captured["approval_handler"] = kwargs.get("approval_handler")
        captured["risk_auto_approve"] = kwargs.get("risk_auto_approve")
        captured["auto_approve_required_tools"] = kwargs.get("auto_approve_required_tools")
        return create_default_orchestrator(*args, **kwargs)

    monkeypatch.setattr(cli_module, "create_default_orchestrator", capture_orchestrator)

    result = CliRunner().invoke(app, ["run", "--approval-mode", "full-access", "hello"])

    assert result.exit_code == 0
    assert captured["approval_handler"] is None
    assert captured["risk_auto_approve"] is False
    assert captured["auto_approve_required_tools"] is True
    assert "[echo:default] hello" in result.stdout


def test_cli_run_full_access_approval_aliases_normalize(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))

    for alias in ("full", "never", "yolo"):
        result = CliRunner().invoke(app, ["run", "--approval-mode", alias, "hello"])

        assert result.exit_code == 0
        assert "[echo:default] hello" in result.stdout


def test_cli_run_rejects_unknown_approval_mode(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))

    result = CliRunner().invoke(app, ["run", "--approval-mode", "wild-west", "hello"])

    assert result.exit_code == 2
    assert "Invalid approval mode" in result.stdout
    assert "full-access" in result.stdout


def test_cli_run_accepts_global_ca_bundle_option(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    ca_bundle = tmp_path / "ca.pem"
    ca_bundle.write_text("test-ca", encoding="utf-8")

    result = CliRunner().invoke(app, ["--ca-bundle", str(ca_bundle), "run", "hello"])

    assert result.exit_code == 0
    assert "[echo:default] hello" in result.stdout


def test_cli_run_passes_global_http_settings_to_orchestrator(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "config"))
    captured: dict[str, object] = {}
    create_default_orchestrator = cli_module.create_default_orchestrator

    def capture_orchestrator(*args, **kwargs):
        captured["http_client_config"] = kwargs.get("http_client_config")
        return create_default_orchestrator(*args, **kwargs)

    monkeypatch.setattr(cli_module, "create_default_orchestrator", capture_orchestrator)

    result = CliRunner().invoke(
        app,
        [
            "--http-proxy",
            "http://proxy.example.test:8080",
            "--http-no-trust-env",
            "run",
            "hello",
        ],
    )

    http_client_config = captured["http_client_config"]
    assert result.exit_code == 0
    assert isinstance(http_client_config, HttpClientConfig)
    assert http_client_config.proxy_url == "http://proxy.example.test:8080"
    assert http_client_config.trust_env is False


def test_cli_run_accepts_provider_model_and_endpoint_options(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))

    result = CliRunner().invoke(
        app,
        [
            "--provider",
            "echo",
            "--model",
            "smoke-model",
            "--base-url",
            "https://gateway.example.test/v1",
            "--api-key",
            "test-key",
            "run",
            "hello",
        ],
    )

    assert result.exit_code == 0
    assert "[echo:smoke-model] hello" in result.stdout


def test_cli_models_list_shows_default_roles(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "config"))

    result = CliRunner().invoke(app, ["models", "list"])

    assert result.exit_code == 0
    assert "primary" in result.stdout
    assert "risk_evaluator" in result.stdout
    assert "context_summarizer" in result.stdout
    assert "subagent_default" in result.stdout
    assert "research_planner" in result.stdout
    assert "research_worker" in result.stdout
    assert "research_synthesizer" in result.stdout


def test_cli_models_doctor_checks_selected_role(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "config"))

    result = CliRunner().invoke(app, ["models", "doctor", "--role", "risk_evaluator"])

    assert result.exit_code == 0
    assert "Role: risk_evaluator" in result.stdout
    assert "Status: ready" in result.stdout


def test_cli_run_model_role_uses_configured_profile(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "config"))
    _write_config(
        ColossusConfig(
            models=ModelRoutingConfig(
                profiles={
                    "main": ModelProfile(provider="echo", model="main-model"),
                    "risk": ModelProfile(provider="echo", model="risk-model"),
                },
                roles={
                    "primary": "main",
                    "risk_evaluator": "risk",
                    "context_summarizer": "main",
                    "subagent_default": "main",
                },
            )
        )
    )

    result = CliRunner().invoke(app, ["run", "--model-role", "risk_evaluator", "hello"])

    assert result.exit_code == 0
    assert "[echo:risk-model] hello" in result.stdout


def test_cli_repl_passes_selected_model_to_agent(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    captured: dict[str, object] = {}

    def fake_run_repl_sync(*args, **kwargs) -> None:
        agent = args[4]
        captured["model"] = agent.model
        captured["max_turns"] = agent.max_turns
        captured["approval_mode"] = kwargs["approval_mode"]
        captured["history_path"] = kwargs["history_path"]
        captured["task_service"] = kwargs["task_service"]
        captured["theme_name"] = kwargs["theme_name"]

    monkeypatch.setattr(cli_module, "run_repl_sync", fake_run_repl_sync)

    result = CliRunner().invoke(
        app,
        [
            "--provider",
            "echo",
            "--model",
            "repl-model",
            "repl",
            "--theme",
            "mono",
            "--max-turns",
            "38",
        ],
    )

    assert result.exit_code == 0
    assert captured["model"] == "repl-model"
    assert captured["max_turns"] == 38
    assert captured["approval_mode"] == "ask"
    assert captured["history_path"].name == "repl_history.txt"
    assert captured["history_path"].parent.exists()
    assert captured["task_service"] is not None
    assert captured["theme_name"] == "mono"


def test_cli_repl_accepts_user_theme_files(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "config"))
    theme_dir = config_path().parent / "themes"
    theme_dir.mkdir(parents=True)
    (theme_dir / "ocean.json").write_text(
        '{"name":"ocean","styles":{"prompt.caret":"#00ffff bold"}}',
        encoding="utf-8",
    )
    captured: dict[str, object] = {}

    def fake_run_repl_sync(*args, **kwargs) -> None:
        captured["theme_name"] = kwargs["theme_name"]
        captured["theme_dirs"] = kwargs["theme_dirs"]
        captured["preferences_service"] = kwargs["preferences_service"]

    monkeypatch.setattr(cli_module, "run_repl_sync", fake_run_repl_sync)

    result = CliRunner().invoke(app, ["repl", "--theme", "ocean"])

    assert result.exit_code == 0
    assert captured["theme_name"] == "ocean"
    assert captured["theme_dirs"] == (theme_dir,)
    assert captured["preferences_service"] is not None


def test_cli_repl_passes_risk_auto_approval_mode(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    captured: dict[str, object] = {}

    def fake_run_repl_sync(*args, **kwargs) -> None:
        captured["approval_mode"] = kwargs["approval_mode"]

    monkeypatch.setattr(cli_module, "run_repl_sync", fake_run_repl_sync)

    result = CliRunner().invoke(app, ["repl", "--approval-mode", "risk-auto"])

    assert result.exit_code == 0
    assert captured["approval_mode"] == "risk-auto"


def test_cli_repl_passes_full_access_approval_mode(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    captured: dict[str, object] = {}
    create_default_orchestrator = cli_module.create_default_orchestrator

    def capture_orchestrator(*args, **kwargs):
        captured["approval_handler"] = kwargs.get("approval_handler")
        captured["risk_auto_approve"] = kwargs.get("risk_auto_approve")
        captured["auto_approve_required_tools"] = kwargs.get("auto_approve_required_tools")
        return create_default_orchestrator(*args, **kwargs)

    def fake_run_repl_sync(*args, **kwargs) -> None:
        captured["approval_mode"] = kwargs["approval_mode"]

    monkeypatch.setattr(cli_module, "create_default_orchestrator", capture_orchestrator)
    monkeypatch.setattr(cli_module, "run_repl_sync", fake_run_repl_sync)

    result = CliRunner().invoke(app, ["repl", "--approval-mode", "never"])

    assert result.exit_code == 0
    assert captured["approval_mode"] == "full-access"
    assert captured["approval_handler"] is None
    assert captured["risk_auto_approve"] is False
    assert captured["auto_approve_required_tools"] is True


def test_cli_repl_rejects_unknown_theme(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))

    result = CliRunner().invoke(app, ["repl", "--theme", "invisible"])

    assert result.exit_code == 2
    assert "Invalid REPL theme" in result.stdout


def test_cli_repl_accepts_resume_flag_without_prompt_loop(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    asyncio.run(
        SQLiteStateStore(tmp_path / "colossus" / "state.sqlite3").append_message(
            "session-1",
            "run-1",
            UserMessage(content="hello"),
        )
    )
    captured: dict[str, object] = {}

    def fake_run_repl_sync(*args, **kwargs):
        del args
        captured["resume_latest"] = kwargs.get("resume_latest")
        captured["initial_session_id"] = kwargs.get("initial_session_id")

    monkeypatch.setattr(cli_module, "run_repl_sync", fake_run_repl_sync)

    result = CliRunner().invoke(app, ["repl", "--resume"])

    assert result.exit_code == 0
    assert captured == {"resume_latest": True, "initial_session_id": None}


def test_cli_agents_list_renders_persisted_jobs(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    state = SQLiteStateStore(tmp_path / "colossus" / "state.sqlite3")
    service = SubagentService(state, JsonlAuditSink(tmp_path / "colossus" / "audit.jsonl"))
    job = SubagentJob(
        id="agent-1",
        session_id="session-1",
        parent_run_id="run-1",
        parent_call_id="call-1",
        task="Check tests",
        child_session_id="session-1:subagent:agent-1",
    )
    asyncio.run(state.save_subagent_job(job))

    result = CliRunner().invoke(app, ["agents", "list", "--session", "session-1"])

    assert result.exit_code == 0
    assert "agent-1" in result.stdout
    assert "Check tests" in result.stdout
    assert service.max_concurrent == 4


def test_cli_lists_bundled_skills() -> None:
    result = CliRunner().invoke(app, ["skills", "list"])

    assert result.exit_code == 0
    assert "coding" in result.stdout
    assert "offline-dev" in result.stdout
    assert "skill-creator" in result.stdout


def test_cli_skills_new_scaffolds_and_refuses_overwrite(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    monkeypatch.chdir(tmp_path)

    created = CliRunner().invoke(
        app,
        ["skills", "new", "demo-skill", "--description", "Demo workflow."],
    )
    skill_dir = tmp_path / ".agents" / "skills" / "demo-skill"

    assert created.exit_code == 0
    assert "demo-skill" in created.stdout
    assert skill_dir.is_dir()
    assert json.loads((skill_dir / "manifest.json").read_text(encoding="utf-8"))[
        "description"
    ] == "Demo workflow."

    duplicate = CliRunner().invoke(app, ["skills", "new", "demo-skill"])
    forced = CliRunner().invoke(app, ["skills", "new", "demo-skill", "--force"])

    assert duplicate.exit_code == 1
    assert "already exists" in duplicate.stdout
    assert forced.exit_code == 0


def test_cli_skills_new_accepts_custom_parent_path(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    monkeypatch.chdir(tmp_path)
    parent = tmp_path / "custom-skills"

    result = CliRunner().invoke(
        app,
        ["skills", "new", "custom-skill", "--path", str(parent)],
    )

    assert result.exit_code == 0
    assert (parent / "custom-skill" / "manifest.json").is_file()
    assert (parent / "custom-skill" / "SKILL.md").is_file()


def test_cli_skills_new_accepts_resources_agent_frontmatter_and_pack_path(
    tmp_path,
    monkeypatch,
) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    monkeypatch.chdir(tmp_path)
    pack_root = tmp_path / "demo-pack"

    result = CliRunner().invoke(
        app,
        [
            "skills",
            "new",
            "pack-skill",
            "--pack",
            str(pack_root),
            "--resources",
            "references,scripts,assets",
            "--agent-compatible",
        ],
    )
    skill_dir = pack_root / "skills" / "pack-skill"
    skill_text = (skill_dir / "SKILL.md").read_text(encoding="utf-8")

    assert result.exit_code == 0
    assert skill_text.startswith("---\nname: pack-skill\n")
    assert (skill_dir / "manifest.json").is_file()
    assert (skill_dir / "references").is_dir()
    assert (skill_dir / "scripts").is_dir()
    assert (skill_dir / "assets").is_dir()


def test_cli_skills_validate_reports_success_and_failure(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    monkeypatch.chdir(tmp_path)
    created = CliRunner().invoke(app, ["skills", "new", "valid-skill"])
    skill_dir = tmp_path / ".agents" / "skills" / "valid-skill"
    invalid_dir = tmp_path / "invalid"
    invalid_dir.mkdir()

    valid = CliRunner().invoke(app, ["skills", "validate", str(skill_dir)])
    invalid = CliRunner().invoke(app, ["skills", "validate", str(invalid_dir)])

    assert created.exit_code == 0
    assert valid.exit_code == 0
    assert "Skill is valid" in valid.stdout
    assert invalid.exit_code == 1
    assert "SKILL.md is missing" in invalid.stdout


def test_cli_skills_new_user_and_install_global_skill(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    monkeypatch.setenv("HOME", str(tmp_path / "home"))
    monkeypatch.chdir(tmp_path)

    local = CliRunner().invoke(app, ["skills", "new", "installable-skill"])
    local_dir = tmp_path / ".agents" / "skills" / "installable-skill"
    installed = CliRunner().invoke(app, ["skills", "install", str(local_dir)])
    user = CliRunner().invoke(app, ["skills", "new", "legacy-skill", "--user"])

    assert local.exit_code == 0
    assert installed.exit_code == 0
    assert (tmp_path / "home" / ".agents" / "skills" / "installable-skill").is_dir()
    assert user.exit_code == 0
    assert (cli_module.data_dir() / "skills" / "legacy-skill").is_dir()


def test_cli_lists_builtin_tools(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "config"))

    result = CliRunner().invoke(app, ["tools", "list"])

    assert result.exit_code == 0
    assert "filesystem.read" in result.stdout
    assert "filesystem.write" in result.stdout
    assert "shell.run" in result.stdout
    assert "task.create" in result.stdout
    assert "plan.approve_request" in result.stdout
    assert "test.run" in result.stdout
    assert "patch.apply" in result.stdout
    assert "repo.map" in result.stdout
    assert "agent.delegate" in result.stdout
    assert "skill.scaffold" in result.stdout
    assert "skill.inspect" in result.stdout
    assert "skill.read" in result.stdout
    assert "skill.write" in result.stdout
    assert "skill.validate" in result.stdout
    assert "skill.install" in result.stdout
    assert "skill.resource.list" in result.stdout
    assert "skill.resource.read" in result.stdout
    assert "web.search" not in result.stdout
    assert "mcp.call" not in result.stdout
    assert "trace.export" in result.stdout
    assert "context.compact" in result.stdout
    assert "context.restore" in result.stdout


def test_cli_tools_list_shows_web_search_when_searxng_is_configured(
    tmp_path, monkeypatch
) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "config"))
    _write_config(
        ColossusConfig(
            research=ResearchConfig(
                search=SearchConfig(
                    kind="searxng",
                    endpoint="https://search.example.test",
                )
            )
        )
    )

    result = CliRunner().invoke(app, ["tools", "list"])

    assert result.exit_code == 0
    assert "web.search" in result.stdout


def test_cli_research_persists_cited_report(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))

    result = CliRunner().invoke(
        app,
        [
            "research",
            "How does AgentSpec tool filtering work?",
            "--source",
            "repo",
            "--max-sources",
            "3",
            "--events",
            "off",
            "--session",
            "session-research",
        ],
    )

    assert result.exit_code == 0
    assert "Research Report" in result.stdout
    assert "[R1]" in result.stdout
    assert "research_id=research-" in result.stdout
    assert "session_id=session-research" in result.stdout


def test_cli_research_uses_workspace_option_for_repo_sources(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "workspace-note.txt").write_text(
        "workspace sentinel evidence for research\n",
        encoding="utf-8",
    )

    result = CliRunner().invoke(
        app,
        [
            "research",
            "workspace sentinel evidence",
            "--source",
            "repo",
            "--workspace",
            str(workspace),
            "--max-sources",
            "3",
            "--events",
            "off",
        ],
    )

    assert result.exit_code == 0
    assert "workspace-note.txt" in result.stdout


def test_cli_research_rejects_invalid_source(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))

    result = CliRunner().invoke(app, ["research", "hello", "--source", "rumor"])

    assert result.exit_code == 2
    assert "Invalid research source" in result.stdout


def test_cli_creates_lists_approves_and_executes_plan(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    runner = CliRunner()

    created = runner.invoke(app, ["run", "--plan", "--session", "session-1", "ship it"])
    assert created.exit_code == 0
    assert "Plan:" in created.stdout
    plan_line = next(line for line in created.stdout.splitlines() if line.startswith("Plan:"))
    plan_id = plan_line.split()[1]

    listed = runner.invoke(app, ["plans", "list", "--session", "session-1"])
    approved = runner.invoke(app, ["plans", "approve", plan_id])
    executed = runner.invoke(app, ["run", "--execute-plan", plan_id])

    assert listed.exit_code == 0
    assert plan_id in listed.stdout
    assert approved.exit_code == 0
    assert executed.exit_code == 0
    assert "session_id=session-1" in executed.stdout


def test_cli_plans_show_renders_markdown_content(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    service = cli_module.create_plan_service(cli_module.data_dir())
    plan = cli_module.asyncio.run(
        service.create_plan("ship it", "session-1", content="# Ship It\n\n- Verify")
    )

    result = CliRunner().invoke(app, ["plans", "show", plan.id])

    assert result.exit_code == 0
    assert "Ship It" in result.stdout
    assert "Verify" in result.stdout
    assert "Clarify Objective" not in result.stdout


def test_cli_context_commands(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    runner = CliRunner()

    run = runner.invoke(app, ["run", "--session", "session-ctx", "hello context"])
    show = runner.invoke(app, ["context", "show", "--session", "session-ctx"])
    compact = runner.invoke(app, ["context", "compact", "--session", "session-ctx"])
    snapshot_id = compact.stdout.split("snapshot ", 1)[1].splitlines()[0]
    snapshots = runner.invoke(app, ["context", "snapshots", "--session", "session-ctx"])
    restore = runner.invoke(app, ["context", "restore", snapshot_id])

    assert run.exit_code == 0
    assert show.exit_code == 0
    assert "threshold_tokens" in show.stdout
    assert compact.exit_code == 0
    assert "Compacted session session-ctx" in compact.stdout
    assert snapshots.exit_code == 0
    assert snapshot_id in snapshots.stdout
    assert restore.exit_code == 0
    assert f"Restored snapshot {snapshot_id}" in restore.stdout


def test_cli_sessions_list_show_and_resume(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    runner = CliRunner()

    old_run = runner.invoke(app, ["run", "--session", "session-old", "older prompt"])
    new_run = runner.invoke(app, ["run", "--session", "session-new", "newer prompt"])
    state_path = tmp_path / "colossus" / "state.sqlite3"
    with sqlite3.connect(state_path) as conn:
        conn.execute(
            "update sessions set updated_at = ? where id = ?",
            ("2026-01-01", "session-old"),
        )
        conn.execute(
            "update sessions set updated_at = ? where id = ?",
            ("2026-01-02", "session-new"),
        )

    listed = runner.invoke(app, ["sessions", "list"])
    shown = runner.invoke(app, ["sessions", "show", "session-new"])
    resumed = runner.invoke(app, ["run", "--resume", "continued prompt"])
    messages = asyncio.run(SQLiteStateStore(state_path).list_messages("session-new"))

    assert old_run.exit_code == 0
    assert new_run.exit_code == 0
    assert listed.exit_code == 0
    assert "session-new" in listed.stdout
    assert "newer prompt" in listed.stdout
    assert shown.exit_code == 0
    assert "last_run_id" in shown.stdout
    assert "newer prompt" in shown.stdout
    assert resumed.exit_code == 0
    assert "session_id=session-new" in resumed.stdout
    assert [message.role for message in messages] == ["user", "assistant", "user", "assistant"]
    assert messages[-2].content == "continued prompt"


def test_cli_run_rejects_resume_with_explicit_session(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))

    result = CliRunner().invoke(app, ["run", "--resume", "--session", "session-1", "hello"])

    assert result.exit_code == 2
    assert "Use either --resume or --session" in result.stdout


def test_cli_run_session_reuses_exact_session_history(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    runner = CliRunner()

    first = runner.invoke(app, ["run", "--session", "session-exact", "first"])
    second = runner.invoke(app, ["run", "--session", "session-exact", "second"])
    messages = asyncio.run(
        SQLiteStateStore(tmp_path / "colossus" / "state.sqlite3").list_messages("session-exact")
    )

    assert first.exit_code == 0
    assert second.exit_code == 0
    assert [message.role for message in messages] == ["user", "assistant", "user", "assistant"]
    assert messages[0].content == "first"
    assert messages[2].content == "second"


def test_cli_context_window_override_updates_context_budget(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "config"))

    result = CliRunner().invoke(
        app,
        [
            "--provider",
            "echo",
            "--model",
            "large-model",
            "--context-window-tokens",
            "131072",
            "context",
            "show",
            "--session",
            "session-large",
        ],
    )

    assert result.exit_code == 0
    assert "context_window_tokens" in result.stdout
    assert "131072" in result.stdout


def test_cli_context_show_uses_provider_discovered_window(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path / "data"))
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "config"))

    class CatalogProvider:
        name = "catalog-provider"

        async def list_models(self) -> tuple[ProviderModelInfo, ...]:
            return (ProviderModelInfo(id="catalog-model", context_window_tokens=200_000),)

    profile = ResolvedModelProfile(
        role="primary",
        profile_name="primary",
        provider="echo",
        model="catalog-model",
    )
    route = ModelRoute(
        role="primary",
        profile_name="primary",
        provider=CatalogProvider(),
        profile=profile,
    )
    router = ModelRouter({"primary": route, "context_summarizer": route})
    monkeypatch.setattr(cli_module, "create_model_router", lambda *args, **kwargs: router)

    result = CliRunner().invoke(app, ["context", "show", "--session", "session-catalog"])

    assert result.exit_code == 0
    assert "context_window_tokens" in result.stdout
    assert "200000" in result.stdout


def test_cli_model_context_windows_prefers_explicit_config() -> None:
    config = ColossusConfig(
        provider=ProviderConfig(
            model_context_windows={
                "discovered-model": 65_536,
                "legacy-model": 98_304,
            },
        ),
        models=ModelRoutingConfig(
            profiles={
                "main": ModelProfile(
                    provider="echo",
                    model="discovered-model",
                    context_window_tokens=131_072,
                )
            },
            roles={"primary": "main"},
        ),
    )

    windows = cli_module._model_context_windows(
        config,
        discovered={
            "discovered-model": 32_000,
            "discovered-only-model": 200_000,
        },
    )

    assert windows["discovered-model"] == 131_072
    assert windows["legacy-model"] == 98_304
    assert windows["discovered-only-model"] == 200_000


def test_cli_tasks_list_shows_persisted_tasks(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    service = cli_module.create_task_service(cli_module.data_dir())

    task = cli_module.asyncio.run(
        service.create_task(session_id="session-task", title="Show task UX")
    )

    result = CliRunner().invoke(app, ["tasks", "list", "--session", "session-task"])

    assert result.exit_code == 0
    assert task.id in result.stdout
    assert "session-task" in result.stdout
    assert "Show task UX" in result.stdout


def test_cli_memories_list_and_search_show_persisted_memories(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    service = cli_module.create_memory_service(cli_module.data_dir())
    memory = cli_module.asyncio.run(
        service.create_memory(
            scope="repo",
            kind="preference",
            text="Run pytest and ruff before completion.",
            source="user",
            repo_root=str(Path.cwd()),
        )
    )

    listed = CliRunner().invoke(app, ["memories", "list", "--scope", "repo"])
    searched = CliRunner().invoke(app, ["memories", "search", "pytest ruff"])

    assert listed.exit_code == 0
    assert searched.exit_code == 0
    assert memory.id in listed.stdout
    assert memory.id in searched.stdout
    assert "preference" in searched.stdout


def test_cli_main_suppresses_expected_error_tracebacks(monkeypatch) -> None:
    def raise_expected_error() -> None:
        raise ColossusError("expected failure")

    monkeypatch.setattr(cli_module, "app", raise_expected_error)

    try:
        cli_module.main()
    except typer.Exit as exc:
        assert exc.exit_code == 1
        assert exc.__cause__ is None
    else:  # pragma: no cover
        raise AssertionError("main() should exit for expected Colossus errors")


def _write_config(config: ColossusConfig) -> None:
    path = config_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(config.model_dump_json(indent=2), encoding="utf-8")
