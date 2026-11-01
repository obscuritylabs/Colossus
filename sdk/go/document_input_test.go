package colossus

import (
	"testing"

	v1alpha1 "github.com/obscuritylabs/colossus/sdk/go/gen/colossus/api/v1alpha1"
	"google.golang.org/protobuf/proto"
)

func TestModelDocumentFormatsRoundTripAndLegacyAbsence(t *testing.T) {
	profile := &v1alpha1.ConfiguredModelProfile{
		Profile: "office", Model: "configured-model", Provider: "open_ai_responses", ProviderProfile: "work",
		DocumentMediaTypes: []string{
			"application/pdf",
			"application/vnd.openxmlformats-officedocument.wordprocessingml.document",
			"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
			"application/vnd.openxmlformats-officedocument.presentationml.presentation",
		},
	}
	info := &v1alpha1.ServerInfo{
		Capabilities:  []*v1alpha1.Capability{{Name: "attachments.document_input", Enabled: true}},
		ModelProfiles: []*v1alpha1.ConfiguredModelProfile{profile},
	}
	encoded, err := proto.Marshal(info)
	if err != nil {
		t.Fatal(err)
	}
	var decoded v1alpha1.ServerInfo
	if err := proto.Unmarshal(encoded, &decoded); err != nil {
		t.Fatal(err)
	}
	if !proto.Equal(info, &decoded) {
		t.Fatal("document format catalog changed during round trip")
	}
	var legacy v1alpha1.ConfiguredModelProfile
	if err := proto.Unmarshal([]byte{0x0a, 0x06, 'l', 'e', 'g', 'a', 'c', 'y'}, &legacy); err != nil {
		t.Fatal(err)
	}
	if len(legacy.GetDocumentMediaTypes()) != 0 {
		t.Fatal("legacy server profile must not claim document support")
	}
	if profile.ProtoReflect().Descriptor().Fields().ByName("document_media_types").Number() != 7 {
		t.Fatal("document media types field number changed")
	}
}
