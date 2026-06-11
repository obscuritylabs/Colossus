import sys

import pytest

from colossus.adapters.subprocess_broker import SubprocessBroker, SubprocessCommand
from colossus.domain.errors import ToolExecutionError


@pytest.mark.asyncio
async def test_subprocess_broker_runs_without_shell_and_limits_output(tmp_path) -> None:
    result = await SubprocessBroker().run(
        SubprocessCommand(
            argv=(sys.executable, "-c", "import sys; sys.stdout.write('abcdef')"),
            cwd=tmp_path,
            max_output_bytes=3,
        )
    )

    assert result.exit_code == 0
    assert result.stdout == "abc"
    assert result.stderr == ""


@pytest.mark.asyncio
async def test_subprocess_broker_strips_parent_env_and_adds_explicit_env(
    tmp_path, monkeypatch
) -> None:
    monkeypatch.setenv("COLOSSUS_PARENT_SECRET", "hidden")
    code = (
        "import os; "
        "print(os.getenv('COLOSSUS_PARENT_SECRET', 'missing')); "
        "print(os.getenv('EXPLICIT_FLAG', 'missing'))"
    )

    result = await SubprocessBroker().run(
        SubprocessCommand(
            argv=(sys.executable, "-c", code),
            cwd=tmp_path,
            env={"EXPLICIT_FLAG": "present"},
        )
    )

    assert result.stdout.splitlines() == ["missing", "present"]


@pytest.mark.asyncio
async def test_subprocess_broker_rejects_empty_argv(tmp_path) -> None:
    with pytest.raises(ToolExecutionError, match="empty argv"):
        await SubprocessBroker().run(SubprocessCommand(argv=(), cwd=tmp_path))


@pytest.mark.asyncio
async def test_subprocess_broker_times_out_and_kills_process(tmp_path) -> None:
    with pytest.raises(ToolExecutionError, match="timed out"):
        await SubprocessBroker().run(
            SubprocessCommand(
                argv=(sys.executable, "-c", "import time; time.sleep(2)"),
                cwd=tmp_path,
                timeout_seconds=0.01,
            )
        )


def test_subprocess_command_defaults_to_empty_env(tmp_path) -> None:
    command = SubprocessCommand(argv=(sys.executable, "--version"), cwd=tmp_path)

    assert command.env == {}
    assert command.max_output_bytes == 32_768
