package colossus

import (
	"unicode/utf8"

	v1alpha1 "github.com/obscuritylabs/colossus/sdk/go/gen/colossus/api/v1alpha1"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"
)

const (
	colossusErrorTypeURL               = "type.googleapis.com/colossus.api.v1alpha1.ColossusErrorDetail"
	maxStatusBytes                     = 64 * 1024
	maxStatusMessageBytes              = 1024
	maxStatusDetails                   = 16
	maxErrorDetailBytes                = 16 * 1024
	maxErrorReasonBytes                = 128
	maxErrorRequestIDBytes             = 128
	maxErrorViolations                 = 32
	maxViolationFieldBytes             = 256
	maxViolationDescriptionBytes       = 1024
	maxProtobufDurationSeconds   int64 = 315_576_000_000
)

// ErrorOutcomeCertainty is the stable SDK representation of effect certainty.
type ErrorOutcomeCertainty string

const (
	// ErrorOutcomeKnown means durable evidence establishes the operation outcome.
	ErrorOutcomeKnown ErrorOutcomeCertainty = "known"
	// ErrorOutcomeUnknown means an effect may have started without terminal evidence.
	ErrorOutcomeUnknown ErrorOutcomeCertainty = "unknown"
)

// ColossusFieldViolation is one bounded, user-correctable validation failure.
type ColossusFieldViolation struct {
	Field       string
	Description string
}

// ColossusRetryAfter is a non-negative protobuf duration without time.Duration overflow.
type ColossusRetryAfter struct {
	Seconds int64
	Nanos   int32
}

// ColossusRPCError is a bounded, transport-independent rich gRPC error.
//
// Retryable is informational. The SDK never automatically retries effectful
// calls, and callers must reconcile Unknown outcomes against durable state before
// considering an operation replay.
type ColossusRPCError struct {
	Code             codes.Code
	Message          string
	Reason           string
	RequestID        string
	Retryable        bool
	RetryAfter       *ColossusRetryAfter
	OutcomeCertainty ErrorOutcomeCertainty
	Violations       []ColossusFieldViolation
}

// DecodeColossusRPCError decodes one canonical Colossus detail from a gRPC error.
//
// Malformed, oversized, duplicated, or non-Colossus details return false. The
// decoder never resolves an Any type URL, logs content, or retries an RPC.
func DecodeColossusRPCError(err error) (*ColossusRPCError, bool) {
	rpcStatus, ok := status.FromError(err)
	if !ok || rpcStatus == nil {
		return nil, false
	}
	statusMessage := rpcStatus.Proto()
	if statusMessage == nil ||
		statusMessage.Code < int32(codes.Canceled) ||
		statusMessage.Code > int32(codes.Unauthenticated) ||
		len(statusMessage.Message) > maxStatusMessageBytes ||
		!utf8.ValidString(statusMessage.Message) ||
		len(statusMessage.Details) > maxStatusDetails ||
		proto.Size(statusMessage) > maxStatusBytes {
		return nil, false
	}

	var packedDetail []byte
	for _, detail := range statusMessage.Details {
		if detail == nil || detail.TypeUrl != colossusErrorTypeURL {
			continue
		}
		if packedDetail != nil || len(detail.Value) > maxErrorDetailBytes {
			return nil, false
		}
		packedDetail = detail.Value
	}
	if packedDetail == nil {
		return nil, false
	}

	detail := &v1alpha1.ColossusErrorDetail{}
	if err := proto.Unmarshal(packedDetail, detail); err != nil {
		return nil, false
	}
	if len(detail.Reason) > maxErrorReasonBytes ||
		len(detail.RequestId) > maxErrorRequestIDBytes ||
		!utf8.ValidString(detail.Reason) ||
		!utf8.ValidString(detail.RequestId) ||
		len(detail.Violations) > maxErrorViolations {
		return nil, false
	}

	var outcome ErrorOutcomeCertainty
	switch detail.OutcomeCertainty {
	case v1alpha1.OutcomeCertainty_OUTCOME_CERTAINTY_KNOWN:
		outcome = ErrorOutcomeKnown
	case v1alpha1.OutcomeCertainty_OUTCOME_CERTAINTY_UNKNOWN:
		outcome = ErrorOutcomeUnknown
	default:
		return nil, false
	}

	violations := make([]ColossusFieldViolation, 0, len(detail.Violations))
	for _, violation := range detail.Violations {
		if violation == nil ||
			len(violation.Field) > maxViolationFieldBytes ||
			len(violation.Description) > maxViolationDescriptionBytes ||
			!utf8.ValidString(violation.Field) ||
			!utf8.ValidString(violation.Description) {
			return nil, false
		}
		violations = append(violations, ColossusFieldViolation{
			Field:       violation.Field,
			Description: violation.Description,
		})
	}

	var retryAfter *ColossusRetryAfter
	if detail.RetryAfter != nil {
		if err := detail.RetryAfter.CheckValid(); err != nil ||
			detail.RetryAfter.Seconds < 0 ||
			detail.RetryAfter.Seconds > maxProtobufDurationSeconds ||
			detail.RetryAfter.Nanos < 0 {
			return nil, false
		}
		retryAfter = &ColossusRetryAfter{
			Seconds: detail.RetryAfter.Seconds,
			Nanos:   detail.RetryAfter.Nanos,
		}
	}

	return &ColossusRPCError{
		Code:             codes.Code(statusMessage.Code),
		Message:          statusMessage.Message,
		Reason:           detail.Reason,
		RequestID:        detail.RequestId,
		Retryable:        detail.Retryable,
		RetryAfter:       retryAfter,
		OutcomeCertainty: outcome,
		Violations:       violations,
	}, true
}
