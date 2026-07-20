from __future__ import annotations

import unittest
from collections.abc import AsyncIterator
from pathlib import Path
from types import SimpleNamespace

import grpc
from colossus.api.v1alpha1 import agent_run_pb2, system_pb2, system_pb2_grpc

from colossus_sdk.client import AgentRuns, ColossusClient, assert_compatible_server_info
from colossus_sdk.credential import StaticBearerCredential
from colossus_sdk.endpoint import EndpointDescriptor, certificate_sha256
from colossus_sdk.watch import RunFeedProtocolError

EXPECTED_INSTANCE_ID = "00000000-0000-4000-8000-000000000001"


def server_info(**changed: object) -> SimpleNamespace:
    values: dict[str, object] = {
        "instance_id": EXPECTED_INSTANCE_ID,
        "api_packages": ["colossus.api.v1alpha1"],
        "deployment_mode": 1,
    }
    values.update(changed)
    return SimpleNamespace(**values)


class ClientIdentityTests(unittest.TestCase):
    def test_live_server_identity_api_and_deployment_are_bound(self) -> None:
        assert_compatible_server_info(server_info(), EXPECTED_INSTANCE_ID, 1)
        for incompatible in (
            server_info(instance_id="00000000-0000-4000-8000-000000000002"),
            server_info(api_packages=["colossus.api.v2"]),
            server_info(deployment_mode=2),
        ):
            with (
                self.subTest(incompatible=incompatible),
                self.assertRaisesRegex(ValueError, "incompatible"),
            ):
                assert_compatible_server_info(incompatible, EXPECTED_INSTANCE_ID, 1)

    def test_grpc_connector_rejects_embedded_or_unspecified_expectation(self) -> None:
        for deployment_mode in (0, 3):
            with (
                self.subTest(deployment_mode=deployment_mode),
                self.assertRaisesRegex(ValueError, "deployment mode"),
            ):
                assert_compatible_server_info(
                    server_info(),
                    EXPECTED_INSTANCE_ID,
                    deployment_mode,
                )


class ConnectorTests(unittest.IsolatedAsyncioTestCase):
    async def test_connector_verifies_tls_bearer_and_live_identity(self) -> None:
        testdata = Path(__file__).parents[2] / "testdata"
        certificate = (testdata / "connector-cert.pem").read_bytes()
        private_key = (testdata / "connector-key.pem").read_bytes()
        authenticated_calls = 0

        class SystemService(system_pb2_grpc.SystemServiceServicer):
            async def GetServerInfo(self, _request: object, context: object) -> object:
                nonlocal authenticated_calls
                metadata = dict(context.invocation_metadata())  # type: ignore[attr-defined]
                self_outer.assertEqual(
                    metadata.get("authorization"),
                    "Bearer connector-test-token",
                )
                authenticated_calls += 1
                return system_pb2.GetServerInfoResponse(
                    server_info=system_pb2.ServerInfo(
                        instance_id=EXPECTED_INSTANCE_ID,
                        api_packages=["colossus.api.v1alpha1"],
                        deployment_mode=1,
                    )
                )

        self_outer = self
        server = grpc.aio.server()
        system_pb2_grpc.add_SystemServiceServicer_to_server(SystemService(), server)
        port = server.add_secure_port(
            "127.0.0.1:0",
            grpc.ssl_server_credentials(((private_key, certificate),)),
        )
        await server.start()
        try:
            pin = certificate_sha256(certificate)
            descriptor = EndpointDescriptor.from_json(
                {
                    "schema_version": 1,
                    "api_version": "colossus.api.v1alpha1",
                    "instance_id": EXPECTED_INSTANCE_ID,
                    "endpoint": f"https://127.0.0.1:{port}",
                    "pid": 1,
                    "certificate_sha256": pin,
                }
            )
            client = await ColossusClient.connect(
                descriptor,
                certificate,
                EXPECTED_INSTANCE_ID,
                pin,
                1,
                StaticBearerCredential("connector-test-token"),
            )
            self.assertEqual(authenticated_calls, 1)
            await client.close()

            wrong_leaf = (testdata / "leaf.pem").read_bytes()
            wrong_pin = certificate_sha256(wrong_leaf)
            wrong_descriptor = EndpointDescriptor.from_json(
                {
                    "schema_version": 1,
                    "api_version": "colossus.api.v1alpha1",
                    "instance_id": EXPECTED_INSTANCE_ID,
                    "endpoint": f"https://127.0.0.1:{port}",
                    "pid": 1,
                    "certificate_sha256": wrong_pin,
                }
            )
            with self.assertRaises(grpc.aio.AioRpcError):
                await ColossusClient.connect(
                    wrong_descriptor,
                    wrong_leaf,
                    EXPECTED_INSTANCE_ID,
                    wrong_pin,
                    1,
                    StaticBearerCredential("wrong-leaf-token"),
                )
            self.assertEqual(authenticated_calls, 1)
        finally:
            await server.stop(None)


class RunWatchReconciliationTests(unittest.IsolatedAsyncioTestCase):
    async def collect_watch(self, run: agent_run_pb2.Run) -> list[agent_run_pb2.WatchRunResponse]:
        class Stub:
            def WatchRun(self, _request: object) -> AsyncIterator[agent_run_pb2.WatchRunResponse]:
                async def empty() -> AsyncIterator[agent_run_pb2.WatchRunResponse]:
                    if False:
                        yield agent_run_pb2.WatchRunResponse()

                return empty()

            async def GetRun(self, _request: object) -> agent_run_pb2.GetRunResponse:
                return agent_run_pb2.GetRunResponse(run=run)

        return [response async for response in AgentRuns(Stub(), agent_run_pb2).watch_run("run-1")]

    async def test_clean_eof_requires_status_matched_terminal_payload(self) -> None:
        invalid_runs = (
            agent_run_pb2.Run(
                run_id="run-1",
                status=agent_run_pb2.RUN_STATUS_COMPLETED,
            ),
            agent_run_pb2.Run(
                run_id="run-1",
                status=agent_run_pb2.RUN_STATUS_COMPLETED,
                failure=agent_run_pb2.RunFailure(),
            ),
            agent_run_pb2.Run(
                run_id="run-1",
                status=agent_run_pb2.RUN_STATUS_RUNNING,
                result=agent_run_pb2.RunResult(),
            ),
        )
        for run in invalid_runs:
            with (
                self.subTest(status=run.status, terminal=run.WhichOneof("terminal")),
                self.assertRaisesRegex(RunFeedProtocolError, "exact cursor"),
            ):
                await self.collect_watch(run)

    async def test_clean_eof_accepts_exact_terminal_payload(self) -> None:
        observed = await self.collect_watch(
            agent_run_pb2.Run(
                run_id="run-1",
                status=agent_run_pb2.RUN_STATUS_COMPLETED,
                result=agent_run_pb2.RunResult(),
            )
        )
        self.assertEqual(observed, [])


if __name__ == "__main__":
    unittest.main()
