"""Optional caller-bound communication. Effects are never retried automatically."""

from __future__ import annotations

import re
from collections.abc import AsyncIterator
from types import ModuleType
from typing import Any


def _token(value: str) -> None:
    if re.fullmatch(r"[A-Za-z0-9._:-]{1,128}", value) is None:
        raise ValueError("invalid bounded agent communication identity")


def validate_agent_message(message: Any) -> Any:
    for identifier in (message.id, message.recipient_id, message.root_run_id):
        _token(identifier)
    if (
        not message.id
        or not message.recipient_id
        or not message.root_run_id
        or not 1 <= message.sequence <= 4096
        or not message.text
        or len(message.text.encode()) > 16 * 1024
        or message.WhichOneof("sender") is None
        or not message.HasField("receipt")
    ):
        raise ValueError("invalid bounded agent communication record")
    _token(getattr(message, message.WhichOneof("sender")))
    if message.HasField("reply_to"):
        _token(message.reply_to)
    receipt = message.receipt
    if receipt.state == "accepted":
        valid = not any(
            receipt.HasField(field) for field in ("run_id", "turn", "request_hash", "reason")
        )
    elif receipt.state == "included_in_turn":
        _token(receipt.run_id)
        valid = (
            bool(receipt.run_id)
            and 1 <= receipt.turn <= 65535
            and len(receipt.request_hash) == 64
            and all(char in "0123456789abcdef" for char in receipt.request_hash)
            and not receipt.HasField("reason")
        )
    elif receipt.state == "not_delivered":
        valid = receipt.reason in {
            "completed",
            "cancelled",
            "failed",
            "interrupted",
            "budget_exhausted",
            "superseded",
        } and not any(receipt.HasField(field) for field in ("run_id", "turn", "request_hash"))
    else:
        valid = False
    if not valid:
        raise ValueError("invalid bounded agent message receipt")
    return message


def _task(task: Any) -> Any:
    _token(task.task_id)
    _token(task.context_id)
    if (
        task.status
        not in {
            "queued",
            "running",
            "waiting",
            "cancelling",
            "completed",
            "failed",
            "cancelled",
            "interrupted",
            "outcome_unknown",
        }
        or len(task.history) > 16
    ):
        raise ValueError("invalid bounded agent task")
    for message in task.history:
        _token(message.message_id)
        if (
            message.task_id != task.task_id
            or message.context_id != task.context_id
            or not message.text
            or len(message.text.encode()) > 16 * 1024
        ):
            raise ValueError("invalid bounded agent task history")
    return task


class AgentCommunication:
    """Thin async operations over the optional generated communication service."""

    __slots__ = ("_stub", "_pb2")

    def __init__(self, stub: Any, pb2: ModuleType) -> None:
        self._stub, self._pb2 = stub, pb2

    async def participants(self, root_run_id: str) -> Any:
        _token(root_run_id)
        response = await self._stub.ListAgentParticipants(
            self._pb2.ListAgentParticipantsRequest(root_run_id=root_run_id)
        )
        if len(response.participants) > 128 or any(
            item.root_run_id != root_run_id
            or item.generation < 1
            or item.pending_messages > 64
            or item.pending_bytes > 256 * 1024
            for item in response.participants
        ):
            raise ValueError("invalid bounded participant page")
        for item in response.participants:
            _token(item.id)
            if item.open == item.HasField("closed_reason"):
                raise ValueError("contradictory participant lifecycle")
        return response

    async def send(self, request: Any) -> Any:
        _token(request.recipient_id)
        _token(request.idempotency_key)
        if not request.text or len(request.text.encode()) > 16 * 1024:
            raise ValueError("invalid bounded agent message input")
        response = await self._stub.SendAgentMessage(request)
        if validate_agent_message(response.message).recipient_id != request.recipient_id:
            raise ValueError("agent message recipient changed")
        return response

    async def get(self, message_id: str) -> Any:
        _token(message_id)
        response = await self._stub.GetAgentMessage(
            self._pb2.GetAgentMessageRequest(message_id=message_id)
        )
        if validate_agent_message(response.message).id != message_id:
            raise ValueError("agent message identity changed")
        return response

    async def messages(self, participant_id: str, after_sequence: int = 0, limit: int = 16) -> Any:
        _token(participant_id)
        if not 1 <= limit <= 16 or not 0 <= after_sequence <= 4096:
            raise ValueError("invalid agent inbox page bounds")
        response = await self._stub.ListAgentMessages(
            self._pb2.ListAgentMessagesRequest(
                participant_id=participant_id, after_sequence=after_sequence, limit=limit
            )
        )
        cursor = after_sequence
        if len(response.messages) > limit:
            raise ValueError("agent inbox page exceeds its bound")
        for message in response.messages:
            cursor += 1
            if (
                validate_agent_message(message).recipient_id != participant_id
                or message.sequence != cursor
            ):
                raise ValueError("agent inbox sequence gap")
        if response.next_sequence != cursor or response.has_more and not response.messages:
            raise ValueError("invalid agent inbox continuation")
        return response

    async def watch(self, root_run_id: str, after_sequence: int = 0) -> AsyncIterator[Any]:
        _token(root_run_id)
        if after_sequence < 0:
            raise ValueError("invalid agent feed cursor")
        cursor = after_sequence
        call = self._stub.WatchAgentMessages(
            self._pb2.WatchAgentMessagesRequest(
                root_run_id=root_run_id, after_sequence=after_sequence
            )
        )
        try:
            async for update in call:
                cursor += 1
                if (
                    update.sequence != cursor
                    or validate_agent_message(update.message).root_run_id != root_run_id
                ):
                    raise ValueError("agent communication feed sequence gap")
                yield update
        finally:
            call.cancel()

    async def submit_task_message(self, request: Any) -> Any:
        response = await self._stub.SubmitAgentTaskMessage(request)
        if not response.HasField("task"):
            raise ValueError("missing agent task")
        return _task(response.task)

    async def get_task(self, request: Any) -> Any:
        response = await self._stub.GetAgentTask(request)
        if not response.HasField("task"):
            raise ValueError("missing agent task")
        task = _task(response.task)
        if task.task_id != request.task_id:
            raise ValueError("agent task identity changed")
        return task

    async def list_tasks(self, request: Any) -> Any:
        response = await self._stub.ListAgentTasks(request)
        if len(response.tasks) > request.page_size or response.total_size < len(response.tasks):
            raise ValueError("invalid bounded agent task page")
        for task in response.tasks:
            _task(task)
        return response
