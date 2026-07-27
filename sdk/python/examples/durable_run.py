"""Application core for one durable Colossus run.

Connection and credential loading stay in trusted application composition. Pass an
already connected ``ColossusClient``; never move its bearer into argv, an environment
variable, a descriptor, or a renderer.
"""

from __future__ import annotations

import uuid
from collections.abc import Awaitable, Callable, Iterable
from dataclasses import dataclass
from typing import Protocol, runtime_checkable

from colossus.api.v1alpha1 import agent_run_pb2, session_pb2

from colossus_sdk import ColossusClient, decode_colossus_rpc_error

InteractionHandler = Callable[
    [agent_run_pb2.Interaction],
    Awaitable[agent_run_pb2.RespondInteractionRequest | None],
]


@runtime_checkable
class RpcErrorLike(Protocol):
    """Minimal error surface required by the bounded rich-status decoder."""

    def trailing_metadata(self) -> Iterable[tuple[str, str | bytes]] | None: ...

    def code(self) -> object: ...


@dataclass(frozen=True)
class DurableRunResult:
    """Released terminal result plus bounded tool activity seen along the way."""

    run_id: str
    output: str
    tool_names: tuple[str, ...]


class DurableRunFailed(RuntimeError):
    """A terminal run failure with retry and outcome metadata preserved."""

    def __init__(self, failure: agent_run_pb2.RunFailure) -> None:
        super().__init__(failure.message)
        self.reason = failure.reason
        self.recoverable = failure.recoverable
        self.outcome_certainty = failure.outcome_certainty
        self.http_status = failure.http_status if failure.HasField("http_status") else None
        self.retry_after_ms = failure.retry_after_ms if failure.HasField("retry_after_ms") else None


async def run_prompt(
    client: ColossusClient,
    prompt: str,
    *,
    mode: agent_run_pb2.RunMode = agent_run_pb2.RUN_MODE_EXECUTE,
    max_turns: int = 12,
    handle_interaction: InteractionHandler | None = None,
) -> DurableRunResult:
    """Create once, watch durably, and return only released terminal output."""

    request = agent_run_pb2.CreateRunRequest(
        input=[session_pb2.ContentPart(text=session_pb2.TextContent(text=prompt))],
        role="primary",
        mode=mode,
        max_turns=max_turns,
        idempotency_key=f"sdk-example-create-{uuid.uuid4()}",
    )
    try:
        created = await client.agent_runs.create_run(request)
    except Exception as error:
        detail = decode_colossus_rpc_error(error) if isinstance(error, RpcErrorLike) else None
        if detail is not None:
            raise RuntimeError(
                f"CreateRun failed: {detail.reason}; retryable={detail.retryable}; "
                f"outcome={detail.outcome_certainty}"
            ) from error
        raise

    run_id = created.run.run_id
    tool_names: set[str] = set()
    async for response in client.agent_runs.watch_run(run_id):
        update = response.update
        update_case = update.WhichOneof("update")
        if update_case == "tool_activity":
            tool_names.add(update.tool_activity.tool_name)
        elif update_case == "interaction":
            if not update.interaction.respondable_by_caller:
                continue
            if handle_interaction is None:
                raise RuntimeError(
                    "run is waiting for an interaction; provide handle_interaction and "
                    "resume from the last durable cursor"
                )
            interaction_response = await handle_interaction(update.interaction)
            if interaction_response is not None:
                await client.agent_runs.respond_interaction(interaction_response)
        elif update_case == "result":
            return DurableRunResult(run_id, update.result.output, tuple(sorted(tool_names)))
        elif update_case == "failure":
            raise DurableRunFailed(update.failure.failure)
        elif update_case == "cancellation":
            raise RuntimeError(f"run cancelled: {update.cancellation.message}")

    raise RuntimeError("run watch ended without an exact terminal update")


def deny_approval(
    interaction: agent_run_pb2.Interaction,
) -> Awaitable[agent_run_pb2.RespondInteractionRequest | None]:
    """Example safe default: deny approval requests and leave prompts unanswered."""

    async def decide() -> agent_run_pb2.RespondInteractionRequest | None:
        if interaction.WhichOneof("content") != "approval":
            return None
        return agent_run_pb2.RespondInteractionRequest(
            run_id=interaction.run_id,
            interaction_id=interaction.interaction_id,
            etag=interaction.etag,
            idempotency_key=f"sdk-example-interaction-{uuid.uuid4()}",
            approval_answer=agent_run_pb2.ApprovalAnswer(
                approved=False,
                request_hash=interaction.approval.request_hash,
            ),
        )

    return decide()
