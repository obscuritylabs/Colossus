package colossus

import (
	"errors"
	"testing"

	v1alpha1 "github.com/obscuritylabs/colossus/sdk/go/gen/colossus/api/v1alpha1"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
	"google.golang.org/protobuf/types/known/anypb"
	"google.golang.org/protobuf/types/known/durationpb"
)

func richError(
	t *testing.T,
	detail *v1alpha1.ColossusErrorDetail,
	detailCount int,
) error {
	t.Helper()
	value, err := proto.Marshal(detail)
	if err != nil {
		t.Fatal(err)
	}
	statusMessage := status.New(codes.InvalidArgument, "request rejected").Proto()
	for range detailCount {
		statusMessage.Details = append(statusMessage.Details, &anypb.Any{
			TypeUrl: colossusErrorTypeURL,
			Value:   value,
		})
	}
	return status.FromProto(statusMessage).Err()
}

func validErrorDetail() *v1alpha1.ColossusErrorDetail {
	return &v1alpha1.ColossusErrorDetail{
		Reason:           "INVALID_ARGUMENT",
		RequestId:        "request-123",
		Retryable:        false,
		RetryAfter:       &durationpb.Duration{Seconds: 2, Nanos: 500_000_000},
		OutcomeCertainty: v1alpha1.OutcomeCertainty_OUTCOME_CERTAINTY_KNOWN,
		Violations: []*v1alpha1.FieldViolation{{
			Field:       "prompt",
			Description: "must not be empty",
		}},
	}
}

func TestDecodeColossusRPCError(t *testing.T) {
	t.Parallel()
	decoded, ok := DecodeColossusRPCError(richError(t, validErrorDetail(), 1))
	if !ok {
		t.Fatal("expected rich error to decode")
	}
	if decoded.Code != codes.InvalidArgument ||
		decoded.Message != "request rejected" ||
		decoded.Reason != "INVALID_ARGUMENT" ||
		decoded.RequestID != "request-123" ||
		decoded.Retryable ||
		decoded.OutcomeCertainty != ErrorOutcomeKnown {
		t.Fatalf("unexpected decoded error: %#v", decoded)
	}
	if decoded.RetryAfter == nil ||
		decoded.RetryAfter.Seconds != 2 ||
		decoded.RetryAfter.Nanos != 500_000_000 {
		t.Fatalf("unexpected retry-after: %#v", decoded.RetryAfter)
	}
	if len(decoded.Violations) != 1 ||
		decoded.Violations[0].Field != "prompt" ||
		decoded.Violations[0].Description != "must not be empty" {
		t.Fatalf("unexpected violations: %#v", decoded.Violations)
	}
}

func TestDecodeColossusRPCErrorRejectsInvalidDetails(t *testing.T) {
	t.Parallel()
	oversized := validErrorDetail()
	oversized.Reason = string(make([]byte, maxErrorReasonBytes+1))
	unspecified := validErrorDetail()
	unspecified.OutcomeCertainty =
		v1alpha1.OutcomeCertainty_OUTCOME_CERTAINTY_UNSPECIFIED

	for name, err := range map[string]error{
		"not a grpc status":   errors.New("ordinary error"),
		"duplicate detail":    richError(t, validErrorDetail(), 2),
		"oversized detail":    richError(t, oversized, 1),
		"unspecified outcome": richError(t, unspecified, 1),
	} {
		t.Run(name, func(t *testing.T) {
			t.Parallel()
			if decoded, ok := DecodeColossusRPCError(err); ok || decoded != nil {
				t.Fatalf("unexpected decoded error: %#v", decoded)
			}
		})
	}
}

func TestDecodeColossusRPCErrorIgnoresUnknownAnyWithoutResolution(t *testing.T) {
	t.Parallel()
	statusMessage := status.New(codes.Internal, "internal").Proto()
	statusMessage.Details = []*anypb.Any{{
		TypeUrl: "type.googleapis.com/example.ExternalDetail",
		Value:   []byte("opaque"),
	}}
	if decoded, ok := DecodeColossusRPCError(status.FromProto(statusMessage).Err()); ok ||
		decoded != nil {
		t.Fatalf("unexpected decoded error: %#v", decoded)
	}
}
