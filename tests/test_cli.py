from click.exceptions import Exit as ClickExit
from typer.testing import CliRunner

import colossus.cli as cli_module
from colossus.cli import app
from colossus.domain.errors import ColossusError
from colossus.domain.models import ModelProfile, ModelRoutingConfig
from colossus.infrastructure.config import ColossusConfig
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


def test_cli_run_rejects_unknown_approval_mode(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))

    result = CliRunner().invoke(app, ["run", "--approval-mode", "wild-west", "hello"])

    assert result.exit_code == 2
    assert "Invalid approval mode" in result.stdout


def test_cli_run_accepts_global_ca_bundle_option(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    ca_bundle = tmp_path / "ca.pem"
    ca_bundle.write_text("test-ca", encoding="utf-8")

    result = CliRunner().invoke(app, ["--ca-bundle", str(ca_bundle), "run", "hello"])

    assert result.exit_code == 0
    assert "[echo:default] hello" in result.stdout


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
        captured["approval_mode"] = kwargs["approval_mode"]
        captured["history_path"] = kwargs["history_path"]
        captured["task_service"] = kwargs["task_service"]
        captured["theme_name"] = kwargs["theme_name"]

    monkeypatch.setattr(cli_module, "run_repl_sync", fake_run_repl_sync)

    result = CliRunner().invoke(
        app,
        ["--provider", "echo", "--model", "repl-model", "repl", "--theme", "mono"],
    )

    assert result.exit_code == 0
    assert captured["model"] == "repl-model"
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


def test_cli_repl_rejects_unknown_theme(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))

    result = CliRunner().invoke(app, ["repl", "--theme", "invisible"])

    assert result.exit_code == 2
    assert "Invalid REPL theme" in result.stdout


def test_cli_lists_bundled_skills() -> None:
    result = CliRunner().invoke(app, ["skills", "list"])

    assert result.exit_code == 0
    assert "coding" in result.stdout
    assert "offline-dev" in result.stdout


def test_cli_lists_builtin_tools() -> None:
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
    assert "web.search" in result.stdout
    assert "mcp.call" in result.stdout
    assert "trace.export" in result.stdout
    assert "context.compact" in result.stdout
    assert "context.restore" in result.stdout


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


def test_cli_main_suppresses_expected_error_tracebacks(monkeypatch) -> None:
    def raise_expected_error() -> None:
        raise ColossusError("expected failure")

    monkeypatch.setattr(cli_module, "app", raise_expected_error)

    try:
        cli_module.main()
    except ClickExit as exc:
        assert exc.exit_code == 1
        assert exc.__cause__ is None
    else:  # pragma: no cover
        raise AssertionError("main() should exit for expected Colossus errors")


def _write_config(config: ColossusConfig) -> None:
    path = config_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(config.model_dump_json(indent=2), encoding="utf-8")
