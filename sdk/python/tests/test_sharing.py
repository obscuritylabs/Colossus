import unittest

from colossus.api.v1alpha1 import agent_run_pb2, common_pb2

from colossus_sdk.client import AgentRuns


class SharingTests(unittest.TestCase):
    def test_missing_authority_is_read_only_and_owner_cannot_be_selected(self) -> None:
        visible = agent_run_pb2.VisibleRun.FromString(b"")
        self.assertFalse(visible.controllable)
        self.assertFalse(visible.continuable)
        request = agent_run_pb2.SetWorkspaceSharingRequest(
            recipient_application_id="app:cloud", enabled=True, allow_continuation=False
        )
        self.assertNotIn("owner_application_id", request.DESCRIPTOR.fields_by_name)
        self.assertEqual(
            agent_run_pb2.SetWorkspaceSharingRequest.FromString(request.SerializeToString()),
            request,
        )
        methods = agent_run_pb2.DESCRIPTOR.services_by_name["AgentRunService"].methods_by_name
        self.assertEqual(methods["ListVisibleRuns"].input_type.name, "ListVisibleRunsRequest")
        self.assertEqual(
            methods["SetWorkspaceSharing"].output_type.name, "SetWorkspaceSharingResponse"
        )


class SharingClientTests(unittest.IsolatedAsyncioTestCase):
    async def test_sharing_mutations_are_not_retried_and_discovery_keeps_its_cursor(self) -> None:
        calls = []

        class Stub:
            async def SetWorkspaceSharing(self, request):
                calls.append(request)
                raise RuntimeError("lost reply")

            async def ListVisibleRuns(self, request):
                calls.append(request)
                return agent_run_pb2.ListVisibleRunsResponse()

        runs = AgentRuns(Stub(), agent_run_pb2)
        with self.assertRaisesRegex(RuntimeError, "lost reply"):
            await runs.set_workspace_sharing(
                agent_run_pb2.SetWorkspaceSharingRequest(recipient_application_id="app:cloud")
            )
        self.assertEqual(len(calls), 1)
        cursor = "source-cursor"
        request = agent_run_pb2.ListVisibleRunsRequest(
            page=common_pb2.PageRequest(page_size=32, page_token=cursor)
        )
        await runs.list_visible_runs(request)
        self.assertIs(calls[-1], request)
