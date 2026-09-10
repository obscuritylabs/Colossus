package colossus

import (
	v1alpha1 "github.com/obscuritylabs/colossus/sdk/go/gen/colossus/api/v1alpha1"
	"google.golang.org/protobuf/proto"
	"testing"
)

func TestCommandApprovalOptionalContextRoundTrip(t *testing.T) {
	approval := &v1alpha1.ApprovalInteraction{Reason: "Approval required", Action: "process.execute", Resource: "configured executable", RequestHash: "binding"}
	for _, context := range []*v1alpha1.CommandApprovalContext{nil, {
		Justification: "Check dependency versions.", Executable: "/bin/sh",
		Arguments: []string{"-c", "echo 'two  spaces'", "", "é"}, WorkingDirectory: "/work/project", Redacted: true,
	}} {
		approval.CommandContext = context
		encoded, err := proto.Marshal(approval)
		if err != nil {
			t.Fatal(err)
		}
		var decoded v1alpha1.ApprovalInteraction
		if err := proto.Unmarshal(encoded, &decoded); err != nil {
			t.Fatal(err)
		}
		if !proto.Equal(approval, &decoded) {
			t.Fatal("command context changed during round trip")
		}
	}
	if approval.ProtoReflect().Descriptor().Fields().ByName("command_context").Number() != 8 {
		t.Fatal("command context field number changed")
	}
}
