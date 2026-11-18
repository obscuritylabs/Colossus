import assert from "node:assert/strict";
import { test } from "node:test";
import { AgentRunServiceService } from "../src/gen/colossus/api/v1alpha1/agent_run.js";
import {
  ReadSessionTranscriptRequest,
  ReadSessionTranscriptResponse,
  TranscriptActivity,
  TranscriptMessageRole,
} from "../src/gen/colossus/api/v1alpha1/transcript.js";

test("transcript wire preserves full text, multipart order and exact u64 boundaries", () => {
  const text = "```rust\nlet quoted = \"🦀\\é\";\n```\n".repeat(4000);
  const boundary = 9007199254740993n;
  const response = ReadSessionTranscriptResponse.fromPartial({
    snapshot: {
      schemaVersion: 1,
      sessionId: "session",
      streamVersion: boundary + 1n,
      exportedAt: "2026-11-18T20:00:00Z",
      activity: TranscriptActivity.TRANSCRIPT_ACTIVITY_ACTIVE,
    },
    messages: [{
      sequence: boundary,
      runId: "source-run",
      role: TranscriptMessageRole.TRANSCRIPT_MESSAGE_ROLE_ASSISTANT,
      createdAt: "2026-11-18T19:00:00Z",
      parts: [
        { content: { $case: "text", value: text } },
        { content: { $case: "attachment", value: { fileName: "picture.png", mediaType: "image/png", sizeBytes: 42n } } },
        { content: { $case: "text", value: "after attachment" } },
      ],
    }],
    page: { nextPageToken: "snapshot-cursor" },
  });
  const decoded = ReadSessionTranscriptResponse.decode(ReadSessionTranscriptResponse.encode(response).finish());
  assert.deepEqual(decoded, response);
  assert.equal(decoded.messages[0]?.sequence, boundary);
  assert.equal(decoded.messages[0]?.parts[0]?.content?.value, text);
  assert.equal(decoded.page?.nextPageToken, "snapshot-cursor");
  assert.equal(AgentRunServiceService.readSessionTranscript.path, "/colossus.api.v1alpha1.AgentRunService/ReadSessionTranscript");
  const request = ReadSessionTranscriptRequest.fromPartial({ sourceRunId: "source-run", page: { pageSize: 1, pageToken: "snapshot-cursor" } });
  assert.deepEqual(ReadSessionTranscriptRequest.decode(ReadSessionTranscriptRequest.encode(request).finish()), request);
  assert.equal("sessionId" in request, false);
  assert.equal("ownerApplicationId" in request, false);
});
