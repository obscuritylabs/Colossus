package colossus

import (
	"testing"

	v1alpha1 "github.com/obscuritylabs/colossus/sdk/go/gen/colossus/api/v1alpha1"
	"google.golang.org/protobuf/proto"
)

func TestSharedRunMissingAuthorityIsReadOnly(t *testing.T) {
	var visible v1alpha1.VisibleRun
	if err := proto.Unmarshal(nil, &visible); err != nil {
		t.Fatal(err)
	}
	if visible.GetControllable() || visible.GetContinuable() {
		t.Fatal("missing flags must not grant authority")
	}
	request := &v1alpha1.SetWorkspaceSharingRequest{RecipientApplicationId: "app:cloud", Enabled: true}
	encoded, err := proto.Marshal(request)
	if err != nil {
		t.Fatal(err)
	}
	var decoded v1alpha1.SetWorkspaceSharingRequest
	if err := proto.Unmarshal(encoded, &decoded); err != nil {
		t.Fatal(err)
	}
	if !proto.Equal(request, &decoded) {
		t.Fatal("explicit sharing request changed")
	}
	if request.ProtoReflect().Descriptor().Fields().ByName("owner_application_id") != nil {
		t.Fatal("caller must not select source ownership")
	}
	service := v1alpha1.File_colossus_api_v1alpha1_agent_run_proto.Services().ByName("AgentRunService")
	if service.Methods().ByName("ListVisibleRuns").Input().Name() != "ListVisibleRunsRequest" {
		t.Fatal("discovery RPC shape changed")
	}
	if service.Methods().ByName("SetWorkspaceSharing").Output().Name() != "SetWorkspaceSharingResponse" {
		t.Fatal("sharing RPC shape changed")
	}
}
