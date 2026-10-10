from __future__ import annotations

import unittest
from typing import Any

from colossus.api.v1alpha1 import communication_pb2 as pb2

from colossus_sdk.communication import AgentCommunication, validate_agent_message


def message(**changed: Any) -> Any:
    values: dict[str, Any] = {
        "id": "message-1",
        "root_run_id": "run-1",
        "recipient_id": "participant-1",
        "sender_application_id": "app-1",
        "sequence": 1,
        "text": "Peer input",
        "accepted_at": "2026-10-09T12:00:00Z",
        "receipt": pb2.AgentMessageReceipt(state="accepted"),
    }
    values.update(changed)
    return pb2.AgentMessage(**values)


class ReceiptTests(unittest.TestCase):
    def test_contradictory_receipts_and_utf8_bounds(self) -> None:
        validate_agent_message(message())
        for changed in (
            {"receipt": pb2.AgentMessageReceipt(state="accepted", reason="failed")},
            {
                "receipt": pb2.AgentMessageReceipt(
                    state="included_in_turn", run_id="run-1", turn=1, request_hash="bad-hash"
                )
            },
            {"text": "😀" * 4097},
        ):
            with self.assertRaises(ValueError):
                validate_agent_message(message(**changed))


class OperationTests(unittest.IsolatedAsyncioTestCase):
    async def test_inbox_gap_is_rejected(self) -> None:
        class Stub:
            async def ListAgentMessages(self, _request: Any) -> Any:
                return pb2.ListAgentMessagesResponse(
                    messages=[message(sequence=2)], next_sequence=2
                )

        with self.assertRaises(ValueError):
            await AgentCommunication(Stub(), pb2).messages("participant-1")

    async def test_unknown_send_outcome_is_not_retried(self) -> None:
        class Stub:
            calls = 0

            async def SendAgentMessage(self, _request: Any) -> Any:
                self.calls += 1
                raise ConnectionError("lost response")

        stub = Stub()
        with self.assertRaises(ConnectionError):
            await AgentCommunication(stub, pb2).send(
                pb2.SendAgentMessageRequest(
                    recipient_id="participant-1", text="Peer input", idempotency_key="stable-key"
                )
            )
        self.assertEqual(stub.calls, 1)
