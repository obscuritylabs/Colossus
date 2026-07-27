"""Connect the Python SDK using a one-use credential from an anonymous stdin pipe."""

from __future__ import annotations

import asyncio
import os
import stat
import sys
from pathlib import Path

from colossus.api.v1alpha1 import system_pb2
from durable_run import run_prompt

from colossus_sdk import ColossusClient, EndpointDescriptor, StaticBearerCredential

_MAX_CREDENTIAL_BYTES = 761


def read_pipe_credential() -> StaticBearerCredential:
    """Read one bounded bearer only when stdin is a pipe or local socket."""

    mode = os.fstat(sys.stdin.fileno()).st_mode
    if not (stat.S_ISFIFO(mode) or stat.S_ISSOCK(mode)):
        raise RuntimeError("the live SDK credential must arrive through an anonymous pipe")
    raw = sys.stdin.buffer.read(_MAX_CREDENTIAL_BYTES + 1)
    if not raw or len(raw) > _MAX_CREDENTIAL_BYTES:
        raise RuntimeError("the live SDK credential is invalid")
    try:
        token = raw.decode("ascii")
    except UnicodeDecodeError as error:
        raise RuntimeError("the live SDK credential is invalid") from error
    return StaticBearerCredential(token)


async def main() -> None:
    if len(sys.argv) != 6:
        raise RuntimeError(
            "usage: live_run.py DESCRIPTOR CERTIFICATE INSTANCE_ID CERTIFICATE_SHA256 PROMPT"
        )
    descriptor_path, certificate_path, instance_id, certificate_sha256, prompt = sys.argv[1:]
    descriptor = EndpointDescriptor.from_json(Path(descriptor_path).read_text(encoding="utf-8"))
    certificate = Path(certificate_path).read_bytes()
    credential = read_pipe_credential()
    client = await ColossusClient.connect(
        descriptor,
        certificate,
        instance_id,
        certificate_sha256,
        system_pb2.DEPLOYMENT_MODE_SHARED_DAEMON,
        credential,
    )
    try:
        result = await run_prompt(client, prompt)
        print(result.output)
    finally:
        await client.close()


if __name__ == "__main__":
    asyncio.run(main())
