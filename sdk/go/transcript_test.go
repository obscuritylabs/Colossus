package colossus

import (
	"strings"
	"testing"

	v1alpha1 "github.com/obscuritylabs/colossus/sdk/go/gen/colossus/api/v1alpha1"
	"google.golang.org/protobuf/proto"
)

func TestTranscriptPreservesFullTextAndExactSnapshotBoundaries(t *testing.T) {
	text := strings.Repeat("```rust\nlet quoted = \"🦀\\é\";\n```\n", 4000)
	const boundary = uint64(9007199254740993)
	response := &v1alpha1.ReadSessionTranscriptResponse{
		Snapshot: &v1alpha1.TranscriptSnapshot{SchemaVersion: 1, SessionId: "session", StreamVersion: boundary + 1,
			ExportedAt: "2026-11-18T20:00:00Z", Activity: v1alpha1.TranscriptActivity_TRANSCRIPT_ACTIVITY_ACTIVE},
		Messages: []*v1alpha1.TranscriptMessage{{Sequence: boundary, RunId: "source-run", CreatedAt: "2026-11-18T19:00:00Z",
			Role: v1alpha1.TranscriptMessageRole_TRANSCRIPT_MESSAGE_ROLE_ASSISTANT,
			Parts: []*v1alpha1.TranscriptPart{
				{Content: &v1alpha1.TranscriptPart_Text{Text: text}},
				{Content: &v1alpha1.TranscriptPart_Attachment{Attachment: &v1alpha1.TranscriptAttachment{FileName: "picture.png", MediaType: "image/png", SizeBytes: 42}}},
				{Content: &v1alpha1.TranscriptPart_Text{Text: "after attachment"}},
			}}},
		Page: &v1alpha1.PageResponse{NextPageToken: "snapshot-cursor"},
	}
	encoded, err := proto.Marshal(response)
	if err != nil {
		t.Fatal(err)
	}
	var decoded v1alpha1.ReadSessionTranscriptResponse
	if err := proto.Unmarshal(encoded, &decoded); err != nil {
		t.Fatal(err)
	}
	if !proto.Equal(response, &decoded) || decoded.Messages[0].Sequence != boundary || decoded.Messages[0].Parts[0].GetText() != text {
		t.Fatal("original transcript text, order or snapshot boundary changed")
	}
	service := v1alpha1.File_colossus_api_v1alpha1_agent_run_proto.Services().ByName("AgentRunService")
	if service.Methods().ByName("ReadSessionTranscript").Input().Name() != "ReadSessionTranscriptRequest" {
		t.Fatal("canonical transcript RPC shape changed")
	}
	request := &v1alpha1.ReadSessionTranscriptRequest{SourceRunId: "source-run", Page: &v1alpha1.PageRequest{PageSize: 1, PageToken: "snapshot-cursor"}}
	fields := request.ProtoReflect().Descriptor().Fields()
	if fields.ByName("session_id") != nil || fields.ByName("owner_application_id") != nil {
		t.Fatal("caller must not select canonical session ownership")
	}
}
