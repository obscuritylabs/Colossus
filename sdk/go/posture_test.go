package colossus

import (
	"context"
	"encoding/json"
	"errors"
	v1alpha1 "github.com/obscuritylabs/colossus/sdk/go/gen/colossus/api/v1alpha1"
	"google.golang.org/grpc"
	"strings"
	"testing"
)

func validPolicy() RuntimePolicyPosture {
	return RuntimePolicyPosture{SchemaVersion: 1, Provenance: PolicyRuntimeReported, Fingerprint: strings.Repeat("a", 64), AccessProfile: "minimal", SandboxBackend: PolicySandboxNative, SandboxProfile: "offline", ApprovalMode: PolicyApprovalAsk, AllowedRoles: []string{"primary"}, AllowedTools: []string{"filesystem.read"}, Capabilities: []string{"runs.read"}, Models: []PolicyModelLabel{{Profile: "main", Label: "safe-model"}}, Findings: []PolicyFinding{{Code: PolicyFindingEphemeral, Severity: PolicyFindingWarning}}, Telemetry: PolicyTelemetry{Provenance: PolicyTelemetryUnavailable}}
}
func TestPolicyU64AndUnknownCounters(t *testing.T) {
	posture := validPolicy()
	revision := ^uint64(0)
	posture.ConfigurationRevision = &revision
	payload, err := json.Marshal(posture)
	if err != nil {
		t.Fatal(err)
	}
	decoded, err := DecodeRuntimePolicyPosture(payload)
	if err != nil || decoded == nil || decoded.ConfigurationRevision == nil || *decoded.ConfigurationRevision != revision {
		t.Fatal("u64 revision must remain exact", err)
	}
	if decoded.Telemetry.DeniedRequests != nil {
		t.Fatal("unknown must not become zero")
	}
}
func TestPolicyClosedBoundedShapes(t *testing.T) {
	bytes, err := json.Marshal(validPolicy())
	if err != nil {
		t.Fatal(err)
	}
	base := string(bytes)
	cases := []string{strings.Replace(base, `"schema_version":1`, `"schema_version":1,"secret":"withheld"`, 1), strings.Replace(base, `"schema_version":1`, `"schema_version":2`, 1), strings.Replace(base, `"schema_version":1`, `"schema_version":1,"schema_version":1`, 1), strings.Replace(base, `"boundary_acknowledged":false`, `"boundary_acknowledged":null`, 1), strings.Replace(base, `"allowed_roles":["primary"]`, `"allowed_roles":["primary","primary"]`, 1), strings.Replace(base, `"allowed_tools":["filesystem.read"]`, `"allowed_tools":null`, 1), strings.Replace(base, `"sandbox_backend":"native"`, `"sandbox_backend":"custom"`, 1), strings.Replace(base, `"label":"safe-model"`, `"label":"https://private.invalid"`, 1), strings.Replace(base, `"denied_requests":null`, `"denied_requests":0`, 1), strings.Replace(base, `"configuration_revision":null`, `"configuration_revision":18446744073709551616`, 1), base + " {}", strings.Repeat(" ", 65537), string([]byte{0xff})}
	for index, value := range cases {
		if _, err = DecodeRuntimePolicyPosture([]byte(value)); !errors.Is(err, ErrRuntimePolicyPosture) {
			t.Fatalf("case %d must fail closed", index)
		}
	}
}

type policyClient struct {
	v1alpha1.AgentRunServiceClient
	calls int
	err   error
}

func (client *policyClient) GetRuntimePolicyPosture(context.Context, *v1alpha1.GetRuntimePolicyPostureRequest, ...grpc.CallOption) (*v1alpha1.GetRuntimePolicyPostureResponse, error) {
	client.calls++
	return nil, client.err
}
func TestPolicyReadDoesNotRetry(t *testing.T) {
	failure := errors.New("unavailable")
	client := &policyClient{err: failure}
	if _, err := GetRuntimePolicyPosture(context.Background(), client); !errors.Is(err, failure) || client.calls != 1 {
		t.Fatal("read must propagate the original failure once")
	}
}
