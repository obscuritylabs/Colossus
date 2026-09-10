import unittest

from colossus.api.v1alpha1 import agent_run_pb2


class CommandApprovalTests(unittest.TestCase):
    def test_optional_context_round_trip_preserves_arguments(self) -> None:
        approval = agent_run_pb2.ApprovalInteraction(
            reason="Approval required",
            action="process.execute",
            resource="configured executable",
            request_hash="binding",
        )
        legacy = agent_run_pb2.ApprovalInteraction.FromString(approval.SerializeToString())
        self.assertFalse(legacy.HasField("command_context"))
        approval.command_context.CopyFrom(
            agent_run_pb2.CommandApprovalContext(
                justification="Check dependency versions.",
                executable="/bin/sh",
                arguments=["-c", "echo 'two  spaces'", "", "é"],
                working_directory="/work/project",
                redacted=True,
            )
        )
        self.assertEqual(
            agent_run_pb2.ApprovalInteraction.FromString(approval.SerializeToString()),
            approval,
        )
        self.assertEqual(approval.DESCRIPTOR.fields_by_name["command_context"].number, 8)
