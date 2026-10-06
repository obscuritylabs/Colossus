package colossus

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"io"
	"regexp"
	"unicode/utf8"

	v1alpha1 "github.com/obscuritylabs/colossus/sdk/go/gen/colossus/api/v1alpha1"
	"google.golang.org/grpc"
)

// PolicyProvenance identifies a configuration report rather than an attestation.
type PolicyProvenance string

const PolicyRuntimeReported PolicyProvenance = "runtime_reported"

// PolicySandboxBackend is the runtime's released execution boundary.
type PolicySandboxBackend string

const (
	PolicySandboxNative           PolicySandboxBackend = "native"
	PolicySandboxWindowsJob       PolicySandboxBackend = "windows_job"
	PolicySandboxOCI              PolicySandboxBackend = "oci"
	PolicySandboxExternal         PolicySandboxBackend = "external"
	PolicySandboxDangerFullAccess PolicySandboxBackend = "danger_full_access"
	PolicySandboxUnknown          PolicySandboxBackend = "unknown"
)

// PolicyApprovalMode is native-owned public approval behavior.
type PolicyApprovalMode string

const (
	PolicyApprovalDeny       PolicyApprovalMode = "deny"
	PolicyApprovalAsk        PolicyApprovalMode = "ask"
	PolicyApprovalRiskAuto   PolicyApprovalMode = "risk_auto"
	PolicyApprovalDangerAuto PolicyApprovalMode = "danger_auto"
	PolicyApprovalUnknown    PolicyApprovalMode = "unknown"
)

// PolicyFindingCode is a closed configuration-risk taxonomy, not a violation count.
type PolicyFindingCode string

const (
	PolicyFindingEphemeral        PolicyFindingCode = "storage.ephemeral"
	PolicyFindingPlaintext        PolicyFindingCode = "storage.plaintext"
	PolicyFindingFullAccess       PolicyFindingCode = "sandbox.danger_full_access"
	PolicyFindingSensitiveJournal PolicyFindingCode = "observability.sensitive_journal_payloads"
	PolicyFindingPlaintextOAuth   PolicyFindingCode = "credentials.mcp_oauth_plaintext"
)

// PolicyFindingSeverity is the closed severity of a configuration finding.
type PolicyFindingSeverity string

const PolicyFindingWarning PolicyFindingSeverity = "warning"

// PolicyTelemetryProvenance describes the canonical counter evidence source.
type PolicyTelemetryProvenance string

const PolicyTelemetryUnavailable PolicyTelemetryProvenance = "unavailable"

// PolicyModelLabel contains safe logical profile/model identifiers only.
type PolicyModelLabel struct {
	Profile string `json:"profile"`
	Label   string `json:"label"`
}

// PolicyFinding reports a known configuration warning.
type PolicyFinding struct {
	Code     PolicyFindingCode     `json:"code"`
	Severity PolicyFindingSeverity `json:"severity"`
}

// PolicyTelemetry preserves unknown counters as nil instead of zero.
type PolicyTelemetry struct {
	Provenance         PolicyTelemetryProvenance `json:"provenance"`
	DeniedRequests     *uint64                   `json:"denied_requests"`
	ApprovalRequests   *uint64                   `json:"approval_requests"`
	OutcomeUnknownRuns *uint64                   `json:"outcome_unknown_runs"`
}

// RuntimePolicyPosture is bounded released metadata. It never grants authority.
type RuntimePolicyPosture struct {
	SchemaVersion         uint32               `json:"schema_version"`
	Provenance            PolicyProvenance     `json:"provenance"`
	Fingerprint           string               `json:"fingerprint"`
	ConfigurationRevision *uint64              `json:"configuration_revision"`
	AccessProfile         string               `json:"access_profile"`
	SandboxBackend        PolicySandboxBackend `json:"sandbox_backend"`
	SandboxProfile        string               `json:"sandbox_profile"`
	BoundaryAcknowledged  bool                 `json:"boundary_acknowledged"`
	ApprovalMode          PolicyApprovalMode   `json:"approval_mode"`
	AllowedRoles          []string             `json:"allowed_roles"`
	AllowedTools          []string             `json:"allowed_tools"`
	Capabilities          []string             `json:"capabilities"`
	Models                []PolicyModelLabel   `json:"models"`
	Findings              []PolicyFinding      `json:"findings"`
	Telemetry             PolicyTelemetry      `json:"telemetry"`
}

// ErrRuntimePolicyPosture omits malformed response contents and private values.
var ErrRuntimePolicyPosture = errors.New("invalid bounded runtime policy metadata")
var policyIdentifier = regexp.MustCompile(`^[A-Za-z0-9._-]{1,128}$`)
var policyFingerprint = regexp.MustCompile(`^[0-9a-f]{64}$`)

func policyJSON(decoder *json.Decoder, depth int) (any, error) {
	if depth > 8 {
		return nil, ErrRuntimePolicyPosture
	}
	token, err := decoder.Token()
	if err != nil {
		return nil, ErrRuntimePolicyPosture
	}
	if delimiter, ok := token.(json.Delim); ok {
		if delimiter == '{' {
			object := map[string]any{}
			for decoder.More() {
				token, err := decoder.Token()
				if err != nil {
					return nil, ErrRuntimePolicyPosture
				}
				key, ok := token.(string)
				if !ok {
					return nil, ErrRuntimePolicyPosture
				}
				if _, duplicate := object[key]; duplicate {
					return nil, ErrRuntimePolicyPosture
				}
				value, err := policyJSON(decoder, depth+1)
				if err != nil {
					return nil, err
				}
				object[key] = value
			}
			end, err := decoder.Token()
			if err != nil || end != json.Delim('}') {
				return nil, ErrRuntimePolicyPosture
			}
			return object, nil
		}
		if delimiter == '[' {
			array := []any{}
			for decoder.More() {
				value, err := policyJSON(decoder, depth+1)
				if err != nil {
					return nil, err
				}
				array = append(array, value)
			}
			end, err := decoder.Token()
			if err != nil || end != json.Delim(']') {
				return nil, ErrRuntimePolicyPosture
			}
			return array, nil
		}
		return nil, ErrRuntimePolicyPosture
	}
	return token, nil
}
func policyObject(value any, required []string, optional ...string) (map[string]any, bool) {
	object, ok := value.(map[string]any)
	if !ok {
		return nil, false
	}
	allowed := map[string]bool{}
	for _, key := range required {
		allowed[key] = true
		if _, exists := object[key]; !exists {
			return nil, false
		}
	}
	for _, key := range optional {
		allowed[key] = true
	}
	for key := range object {
		if !allowed[key] {
			return nil, false
		}
	}
	return object, true
}
func policyList(values []string, max int) bool {
	if values == nil || len(values) > max {
		return false
	}
	for i, value := range values {
		if !policyIdentifier.MatchString(value) || i > 0 && values[i-1] >= value {
			return false
		}
	}
	return true
}
func policyChoice[T ~string](value T, allowed ...T) bool {
	for _, item := range allowed {
		if value == item {
			return true
		}
	}
	return false
}

// DecodeRuntimePolicyPosture validates closed JSON before releasing a typed DTO.
func DecodeRuntimePolicyPosture(payload []byte) (*RuntimePolicyPosture, error) {
	if len(payload) == 0 || len(payload) > 64*1024 || !utf8.Valid(payload) {
		return nil, ErrRuntimePolicyPosture
	}
	decoder := json.NewDecoder(bytes.NewReader(payload))
	decoder.UseNumber()
	raw, err := policyJSON(decoder, 0)
	if err != nil {
		return nil, ErrRuntimePolicyPosture
	}
	if _, err = decoder.Token(); err != io.EOF {
		return nil, ErrRuntimePolicyPosture
	}
	object, ok := policyObject(raw, []string{"schema_version", "provenance", "fingerprint", "access_profile", "sandbox_backend", "sandbox_profile", "boundary_acknowledged", "approval_mode", "allowed_roles", "allowed_tools", "capabilities", "models", "findings", "telemetry"}, "configuration_revision")
	if !ok {
		return nil, ErrRuntimePolicyPosture
	}
	if _, ok := object["boundary_acknowledged"].(bool); !ok {
		return nil, ErrRuntimePolicyPosture
	}
	for _, key := range []string{"models", "findings"} {
		items, ok := object[key].([]any)
		if !ok {
			return nil, ErrRuntimePolicyPosture
		}
		for _, item := range items {
			required := []string{"profile", "label"}
			if key == "findings" {
				required = []string{"code", "severity"}
			}
			if _, ok = policyObject(item, required); !ok {
				return nil, ErrRuntimePolicyPosture
			}
		}
	}
	if _, ok = policyObject(object["telemetry"], []string{"provenance"}, "denied_requests", "approval_requests", "outcome_unknown_runs"); !ok {
		return nil, ErrRuntimePolicyPosture
	}
	var posture RuntimePolicyPosture
	strict := json.NewDecoder(bytes.NewReader(payload))
	strict.DisallowUnknownFields()
	if strict.Decode(&posture) != nil {
		return nil, ErrRuntimePolicyPosture
	}
	if posture.SchemaVersion != 1 || posture.Provenance != PolicyRuntimeReported || !policyFingerprint.MatchString(posture.Fingerprint) || !policyIdentifier.MatchString(posture.AccessProfile) || !policyIdentifier.MatchString(posture.SandboxProfile) || !policyChoice(posture.SandboxBackend, PolicySandboxNative, PolicySandboxWindowsJob, PolicySandboxOCI, PolicySandboxExternal, PolicySandboxDangerFullAccess, PolicySandboxUnknown) || !policyChoice(posture.ApprovalMode, PolicyApprovalDeny, PolicyApprovalAsk, PolicyApprovalRiskAuto, PolicyApprovalDangerAuto, PolicyApprovalUnknown) || !policyList(posture.AllowedRoles, 64) || !policyList(posture.AllowedTools, 256) || !policyList(posture.Capabilities, 256) || posture.Models == nil || len(posture.Models) > 64 || posture.Findings == nil || len(posture.Findings) > 16 || posture.Telemetry.Provenance != "unavailable" || posture.Telemetry.DeniedRequests != nil || posture.Telemetry.ApprovalRequests != nil || posture.Telemetry.OutcomeUnknownRuns != nil {
		return nil, ErrRuntimePolicyPosture
	}
	for _, model := range posture.Models {
		if !policyIdentifier.MatchString(model.Profile) || !policyIdentifier.MatchString(model.Label) {
			return nil, ErrRuntimePolicyPosture
		}
	}
	for _, finding := range posture.Findings {
		if finding.Severity != "warning" || !policyChoice(finding.Code, PolicyFindingEphemeral, PolicyFindingPlaintext, PolicyFindingFullAccess, PolicyFindingSensitiveJournal, PolicyFindingPlaintextOAuth) {
			return nil, ErrRuntimePolicyPosture
		}
	}
	return &posture, nil
}

// GetRuntimePolicyPosture reads once through an authenticated generated client.
func GetRuntimePolicyPosture(ctx context.Context, client v1alpha1.AgentRunServiceClient, options ...grpc.CallOption) (*RuntimePolicyPosture, error) {
	if client == nil {
		return nil, ErrRuntimePolicyPosture
	}
	response, err := client.GetRuntimePolicyPosture(ctx, &v1alpha1.GetRuntimePolicyPostureRequest{}, options...)
	if err != nil {
		return nil, err
	}
	if response == nil {
		return nil, ErrRuntimePolicyPosture
	}
	return DecodeRuntimePolicyPosture(response.PolicyJson)
}
