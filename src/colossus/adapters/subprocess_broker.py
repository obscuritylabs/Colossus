"""Brokered subprocess execution adapter."""

import asyncio
import os
from collections.abc import Mapping, Sequence
from pathlib import Path

from pydantic import BaseModel, ConfigDict, Field

from colossus.domain.errors import ToolExecutionError


class SubprocessCommand(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    argv: tuple[str, ...]
    cwd: Path
    env: Mapping[str, str] = Field(default_factory=dict)
    timeout_seconds: float = 30.0
    max_output_bytes: int = 32_768


class SubprocessResult(BaseModel):
    model_config = ConfigDict(frozen=True, extra="forbid")

    exit_code: int
    stdout: str
    stderr: str


class SubprocessBroker:
    async def run(self, command: SubprocessCommand) -> SubprocessResult:
        if not command.argv:
            raise ToolExecutionError("Cannot execute an empty argv.")
        env = _clean_env(command.env)
        process = await asyncio.create_subprocess_exec(
            *command.argv,
            cwd=command.cwd,
            env=env,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        try:
            stdout_bytes, stderr_bytes = await asyncio.wait_for(
                process.communicate(),
                timeout=command.timeout_seconds,
            )
        except TimeoutError as exc:
            process.kill()
            await process.wait()
            raise ToolExecutionError(f"Command timed out after {command.timeout_seconds}s") from exc
        return SubprocessResult(
            exit_code=process.returncode or 0,
            stdout=_decode_limited(stdout_bytes, command.max_output_bytes),
            stderr=_decode_limited(stderr_bytes, command.max_output_bytes),
        )


def _clean_env(extra: Mapping[str, str]) -> dict[str, str]:
    allowed_keys: Sequence[str] = ("PATH", "HOME", "LANG", "LC_ALL", "TERM")
    cleaned = {key: value for key, value in os.environ.items() if key in allowed_keys}
    cleaned.update(extra)
    return cleaned


def _decode_limited(value: bytes, limit: int) -> str:
    return value[:limit].decode("utf-8", errors="replace")
