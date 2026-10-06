import json
import unittest

from colossus.api.v1alpha1 import agent_run_pb2

from colossus_sdk.client import AgentRuns
from colossus_sdk.posture import decode_runtime_policy_posture


def valid():
    return {
        "schema_version": 1,
        "provenance": "runtime_reported",
        "fingerprint": "a" * 64,
        "configuration_revision": None,
        "access_profile": "minimal",
        "sandbox_backend": "native",
        "sandbox_profile": "offline",
        "boundary_acknowledged": False,
        "approval_mode": "ask",
        "allowed_roles": ["primary"],
        "allowed_tools": ["filesystem.read"],
        "capabilities": ["runs.read"],
        "models": [{"profile": "main", "label": "safe-model"}],
        "findings": [{"code": "storage.ephemeral", "severity": "warning"}],
        "telemetry": {
            "provenance": "unavailable",
            "denied_requests": None,
            "approval_requests": None,
            "outcome_unknown_runs": None,
        },
    }


class PostureTests(unittest.TestCase):
    def test_exact_u64_and_unknown_counters(self):
        value = valid()
        value["configuration_revision"] = 2**64 - 1
        result = decode_runtime_policy_posture(json.dumps(value).encode())
        self.assertEqual(result.configuration_revision, 2**64 - 1)
        self.assertIsNone(result.telemetry.denied_requests)
        self.assertEqual(result.models[0].label, "safe-model")

    def test_closed_bounded_shapes(self):
        cases = [
            {**valid(), "secret": "withheld"},
            {**valid(), "schema_version": True},
            {**valid(), "schema_version": 2},
            {**valid(), "sandbox_backend": "custom"},
            {**valid(), "boundary_acknowledged": None},
            {**valid(), "allowed_tools": None},
            {**valid(), "allowed_roles": ["primary", "primary"]},
            {**valid(), "models": [{"profile": "main", "label": "https://private.invalid"}]},
            {**valid(), "configuration_revision": 2**64},
            {**valid(), "configuration_revision": 1.5},
            {**valid(), "telemetry": {"provenance": "unavailable", "denied_requests": 0}},
        ]
        for value in cases:
            with self.subTest(value=list(value)):
                with self.assertRaisesRegex(
                    ValueError, "^Invalid bounded runtime policy metadata$"
                ):
                    decode_runtime_policy_posture(json.dumps(value).encode())
        for payload in [
            b"",
            b" " * 65537,
            b"\xff",
            json.dumps(valid()).encode() + b" {}",
            json.dumps(valid())
            .replace('"schema_version": 1', '"schema_version": 1,"schema_version": 1')
            .encode(),
        ]:
            with self.assertRaisesRegex(ValueError, "^Invalid bounded runtime policy metadata$"):
                decode_runtime_policy_posture(payload)


class PostureClientTests(unittest.IsolatedAsyncioTestCase):
    async def test_one_typed_read_and_transport_failure(self):
        calls = []

        class Stub:
            async def GetRuntimePolicyPosture(self, request):
                calls.append(request)
                if len(calls) > 1:
                    raise RuntimeError("unavailable")
                return agent_run_pb2.GetRuntimePolicyPostureResponse(
                    policy_json=json.dumps(valid()).encode()
                )

        runs = AgentRuns(Stub(), agent_run_pb2)
        result = await runs.get_runtime_policy_posture()
        self.assertEqual(result.models[0].profile, "main")
        with self.assertRaisesRegex(RuntimeError, "unavailable"):
            await runs.get_runtime_policy_posture()
        self.assertEqual(len(calls), 2)
