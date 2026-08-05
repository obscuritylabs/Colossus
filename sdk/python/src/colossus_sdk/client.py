"""Lazy gRPC adapters over the generated v1alpha1 package."""

from __future__ import annotations

import importlib
from collections.abc import AsyncIterator
from types import ModuleType
from typing import Any

from .credential import StaticBearerCredential
from .endpoint import EndpointDescriptor, assert_pinned_leaf_certificate
from .watch import RunFeedItem, RunWatchReconciliation, watch_run

_TERMINAL_UPDATE_CASES = frozenset({"result", "failure", "cancellation"})
_API_PACKAGE = "colossus.api.v1alpha1"


def is_terminal_run_update(update: Any) -> bool:
    """Return whether a v1alpha1 RunUpdate carries an exact terminal variant."""

    return update.WhichOneof("update") in _TERMINAL_UPDATE_CASES


def assert_compatible_server_info(
    server_info: Any,
    expected_instance_id: str,
    expected_deployment_mode: int,
) -> None:
    """Bind the authenticated live server to trusted enrollment."""

    if expected_deployment_mode not in {1, 2}:
        raise ValueError("expected deployment mode must be shared_daemon or sidecar")
    if (
        server_info is None
        or server_info.instance_id != expected_instance_id
        or server_info.deployment_mode != expected_deployment_mode
        or _API_PACKAGE not in server_info.api_packages
    ):
        raise ValueError("authenticated Colossus server identity is incompatible")


class AgentRuns:
    """Thin asynchronous methods over generated AgentRunService stubs."""

    __slots__ = ("_pb2", "_stub")

    def __init__(self, stub: Any, pb2: ModuleType) -> None:
        self._stub = stub
        self._pb2 = pb2

    async def create_run(self, request: Any) -> Any:
        """Create a run once; the SDK does not retry this effectful call."""

        return await self._stub.CreateRun(request)

    async def get_run(self, run_id: str) -> Any:
        return await self._stub.GetRun(self._pb2.GetRunRequest(run_id=run_id))

    async def list_runs(self, request: Any) -> Any:
        return await self._stub.ListRuns(request)

    async def cancel_run(self, run_id: str, idempotency_key: str) -> Any:
        request = self._pb2.CancelRunRequest(
            run_id=run_id,
            idempotency_key=idempotency_key,
        )
        return await self._stub.CancelRun(request)

    async def respond_interaction(self, request: Any) -> Any:
        return await self._stub.RespondInteraction(request)

    async def watch_run(
        self,
        run_id: str,
        *,
        after_sequence: int = 0,
    ) -> AsyncIterator[Any]:
        """Replay and tail a durable run using an exclusive reconnect cursor."""

        async def open_watch(
            watched_run_id: str,
            cursor: int,
        ) -> AsyncIterator[RunFeedItem[Any]]:
            request = self._pb2.WatchRunRequest(
                run_id=watched_run_id,
                after_sequence=cursor,
            )
            async for response in self._stub.WatchRun(request):
                update = response.update
                yield RunFeedItem(
                    run_id=update.run_id,
                    sequence=update.sequence,
                    value=response,
                )

        def is_terminal(response: Any) -> bool:
            return is_terminal_run_update(response.update)

        async def reconcile(
            watched_run_id: str,
            cursor: int,
        ) -> RunWatchReconciliation:
            response = await self.get_run(watched_run_id)
            run = response.run
            terminal_cases = {
                self._pb2.RUN_STATUS_COMPLETED: "result",
                self._pb2.RUN_STATUS_FAILED: "failure",
                self._pb2.RUN_STATUS_CANCELLED: "cancellation",
                self._pb2.RUN_STATUS_INTERRUPTED: "failure",
                self._pb2.RUN_STATUS_OUTCOME_UNKNOWN: "failure",
            }
            expected_terminal = terminal_cases.get(run.status)
            return RunWatchReconciliation(
                run_id=run.run_id,
                last_sequence=run.last_sequence,
                terminal=(
                    expected_terminal is not None
                    and run.WhichOneof("terminal") == expected_terminal
                ),
            )

        async for item in watch_run(
            run_id,
            open_watch,
            is_terminal,
            reconcile,
            after_sequence=after_sequence,
        ):
            yield item.value


class ColossusClient:
    """Authenticated asynchronous access to generated Colossus service clients."""

    __slots__ = (
        "_channel",
        "agent_runs",
        "artifacts",
        "automations",
        "distribution",
        "extensions",
        "knowledge",
        "operations",
        "sessions",
        "system",
        "work",
    )

    def __init__(self, channel: Any) -> None:
        agent_pb2 = importlib.import_module("colossus.api.v1alpha1.agent_run_pb2")
        agent_grpc = importlib.import_module("colossus.api.v1alpha1.agent_run_pb2_grpc")
        artifact_grpc = importlib.import_module("colossus.api.v1alpha1.artifact_pb2_grpc")
        product_grpc = importlib.import_module("colossus.api.v1alpha1.product_pb2_grpc")
        session_grpc = importlib.import_module("colossus.api.v1alpha1.session_pb2_grpc")
        system_grpc = importlib.import_module("colossus.api.v1alpha1.system_pb2_grpc")

        self._channel = channel
        self.agent_runs = AgentRuns(
            agent_grpc.AgentRunServiceStub(channel),
            agent_pb2,
        )
        self.artifacts = artifact_grpc.ArtifactServiceStub(channel)
        self.automations = product_grpc.AutomationServiceStub(channel)
        self.distribution = product_grpc.DistributionServiceStub(channel)
        self.extensions = product_grpc.ExtensionServiceStub(channel)
        self.knowledge = product_grpc.KnowledgeServiceStub(channel)
        self.operations = product_grpc.OperationsServiceStub(channel)
        self.sessions = session_grpc.SessionServiceStub(channel)
        self.system = system_grpc.SystemServiceStub(channel)
        self.work = product_grpc.WorkServiceStub(channel)

    @classmethod
    async def connect(
        cls,
        descriptor: EndpointDescriptor,
        leaf_certificate_pem: str | bytes,
        expected_instance_id: str,
        expected_certificate_sha256: str,
        expected_deployment_mode: int,
        credential: StaticBearerCredential,
    ) -> ColossusClient:
        """Open a TLS-only loopback channel with in-memory per-call authorization."""

        descriptor = descriptor.validated()
        assert_pinned_leaf_certificate(
            descriptor,
            leaf_certificate_pem,
            expected_instance_id,
            expected_certificate_sha256,
        )
        grpc = importlib.import_module("grpc")

        class AuthMetadataPlugin(grpc.AuthMetadataPlugin):  # type: ignore[name-defined, misc]
            def __call__(self, context: Any, callback: Any) -> None:
                del context
                callback(credential._metadata(), None)

            def __repr__(self) -> str:
                return "ColossusAuthMetadataPlugin([REDACTED])"

        root_certificate = (
            leaf_certificate_pem.encode("ascii")
            if isinstance(leaf_certificate_pem, str)
            else leaf_certificate_pem
        )
        transport_credentials = grpc.ssl_channel_credentials(root_certificates=root_certificate)
        call_credentials = grpc.metadata_call_credentials(AuthMetadataPlugin())
        channel_credentials = grpc.composite_channel_credentials(
            transport_credentials,
            call_credentials,
        )
        options: list[tuple[str, int | str]] = [
            ("grpc.enable_http_proxy", 0),
            ("grpc.enable_retries", 0),
            ("grpc.max_receive_message_length", 4 * 1024 * 1024),
            ("grpc.max_send_message_length", 4 * 1024 * 1024),
            ("grpc.primary_user_agent", "colossus-python-sdk/0.10.3"),
        ]
        channel = grpc.aio.secure_channel(
            descriptor.target,
            channel_credentials,
            options=options,
        )
        client = cls(channel)
        system_pb2 = importlib.import_module("colossus.api.v1alpha1.system_pb2")
        try:
            response = await client.system.GetServerInfo(
                system_pb2.GetServerInfoRequest(),
                timeout=5.0,
            )
            server_info = response.server_info if response.HasField("server_info") else None
            assert_compatible_server_info(
                server_info,
                expected_instance_id,
                expected_deployment_mode,
            )
        except BaseException:
            await client.close()
            raise
        return client

    async def close(self) -> None:
        await self._channel.close()
