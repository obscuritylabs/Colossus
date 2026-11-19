import unittest

from colossus.api.v1alpha1 import agent_run_pb2, common_pb2, transcript_pb2

from colossus_sdk.client import AgentRuns


class TranscriptTests(unittest.TestCase):
    def test_full_text_and_exact_boundaries_round_trip(self) -> None:
        text = '```rust\nlet quoted = "🦀\\é";\n```\n' * 4000
        boundary = 9007199254740993
        cursor = "snapshot-cursor"
        response = transcript_pb2.ReadSessionTranscriptResponse(
            snapshot=transcript_pb2.TranscriptSnapshot(
                schema_version=1,
                session_id="session",
                stream_version=boundary + 1,
                exported_at="2026-11-18T20:00:00Z",
                activity=transcript_pb2.TRANSCRIPT_ACTIVITY_ACTIVE,
            ),
            messages=[
                transcript_pb2.TranscriptMessage(
                    sequence=boundary,
                    run_id="source-run",
                    created_at="2026-11-18T19:00:00Z",
                    role=transcript_pb2.TRANSCRIPT_MESSAGE_ROLE_ASSISTANT,
                    parts=[
                        transcript_pb2.TranscriptPart(text=text),
                        transcript_pb2.TranscriptPart(
                            attachment=transcript_pb2.TranscriptAttachment(
                                file_name="picture.png", media_type="image/png", size_bytes=42
                            )
                        ),
                        transcript_pb2.TranscriptPart(text="after attachment"),
                    ],
                )
            ],
            page=common_pb2.PageResponse(next_page_token=cursor),
        )
        decoded = transcript_pb2.ReadSessionTranscriptResponse.FromString(
            response.SerializeToString()
        )
        self.assertEqual(decoded, response)
        self.assertEqual(decoded.messages[0].parts[0].text, text)
        self.assertEqual(decoded.messages[0].sequence, boundary)
        methods = agent_run_pb2.DESCRIPTOR.services_by_name["AgentRunService"].methods_by_name
        self.assertEqual(
            methods["ReadSessionTranscript"].input_type.name, "ReadSessionTranscriptRequest"
        )
        fields = transcript_pb2.ReadSessionTranscriptRequest.DESCRIPTOR.fields_by_name
        self.assertNotIn("owner_application_id", fields)
        self.assertNotIn("session_id", fields)


class TranscriptClientTests(unittest.IsolatedAsyncioTestCase):
    async def test_public_adapter_preserves_cursor_and_does_not_retry_failures(self) -> None:
        calls = []

        class Stub:
            async def ReadSessionTranscript(self, request):
                calls.append(request)
                raise RuntimeError("disconnected snapshot read")

        cursor = "snapshot-cursor"
        request = transcript_pb2.ReadSessionTranscriptRequest(
            source_run_id="source-run",
            page=common_pb2.PageRequest(page_size=1, page_token=cursor),
        )
        runs = AgentRuns(Stub(), agent_run_pb2)
        with self.assertRaisesRegex(RuntimeError, "disconnected snapshot read"):
            await runs.read_session_transcript(request)
        self.assertEqual(calls, [request])
        self.assertIs(calls[0], request)
