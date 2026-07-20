package colossus

import (
	"context"
	"errors"
	"io"
	"math"
	"time"

	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/reflect/protoreflect"
)

var (
	// ErrRunFeedGap means released output may be incomplete and presentation must stop.
	ErrRunFeedGap = errors.New("watch stream contains a sequence gap")
	// ErrRunFeedIdentity means a stream returned an update for a different run.
	ErrRunFeedIdentity = errors.New("watch stream returned a different run_id")
	// ErrRunFeedReconciliation means clean EOF was not terminal at the exact cursor.
	ErrRunFeedReconciliation = errors.New("clean watch EOF was not terminal at the exact cursor")
)

const (
	// RunUpdateCaseResult is the successful terminal v1alpha1 RunUpdate variant.
	RunUpdateCaseResult = "result"
	// RunUpdateCaseFailure is the failed terminal v1alpha1 RunUpdate variant.
	RunUpdateCaseFailure = "failure"
	// RunUpdateCaseCancellation is the cancelled terminal v1alpha1 RunUpdate variant.
	RunUpdateCaseCancellation = "cancellation"
)

// IsTerminalRunUpdateCase returns true only for exact terminal v1alpha1 oneof cases.
func IsTerminalRunUpdateCase(caseName string) bool {
	switch caseName {
	case RunUpdateCaseResult, RunUpdateCaseFailure, RunUpdateCaseCancellation:
		return true
	default:
		return false
	}
}

// IsTerminalRunUpdate detects the exact terminal oneof variants on a generated
// colossus.api.v1alpha1.RunUpdate without depending on a generated package path.
func IsTerminalRunUpdate[Update protoreflect.ProtoMessage](update Update) bool {
	var protoUpdate protoreflect.ProtoMessage = update
	if protoUpdate == nil {
		return false
	}
	message := protoUpdate.ProtoReflect()
	if !message.IsValid() ||
		message.Descriptor().FullName() != "colossus.api.v1alpha1.RunUpdate" {
		return false
	}
	updateOneof := message.Descriptor().Oneofs().ByName("update")
	if updateOneof == nil {
		return false
	}
	field := message.WhichOneof(updateOneof)
	return field != nil && IsTerminalRunUpdateCase(string(field.Name()))
}

// RunFeedItem is one released update and its at-least-once delivery identity.
type RunFeedItem[Value any] struct {
	RunID    string
	Sequence uint64
	Value    Value
}

// RunStream is the small adapter required around a generated WatchRun client stream.
type RunStream[Value any] interface {
	Recv() (RunFeedItem[Value], error)
}

// RunStreamFunc adapts a function around a generated client stream.
type RunStreamFunc[Value any] func() (RunFeedItem[Value], error)

// Recv implements RunStream.
func (function RunStreamFunc[Value]) Recv() (RunFeedItem[Value], error) {
	return function()
}

// OpenRunWatch opens a stream after an exclusive cursor.
type OpenRunWatch[Value any] func(
	context.Context,
	string,
	uint64,
) (RunStream[Value], error)

// RunWatchReconciliation is GetRun evidence proving a clean stream close is final.
type RunWatchReconciliation struct {
	RunID        string
	LastSequence uint64
	Terminal     bool
}

// RunWatchOptions configures a durable read-only run feed.
type RunWatchOptions[Value any] struct {
	RunID          string
	AfterSequence  uint64
	Open           OpenRunWatch[Value]
	Reconcile      func(context.Context, string, uint64) (RunWatchReconciliation, error)
	IsTerminal     func(Value) bool
	IsRetryable    func(error) bool
	InitialBackoff time.Duration
	MaximumBackoff time.Duration
	Sleep          func(context.Context, time.Duration) error
}

// RunWatcher removes duplicates and reconnects from the last durable cursor.
type RunWatcher[Value any] struct {
	options RunWatchOptions[Value]
	cursor  uint64
	backoff time.Duration
	stream  RunStream[Value]
	done    bool
}

// NewRunWatcher validates a durable run watch without opening it.
func NewRunWatcher[Value any](
	options RunWatchOptions[Value],
) (*RunWatcher[Value], error) {
	if options.RunID == "" {
		return nil, errors.New("run_id must not be empty")
	}
	if options.Open == nil || options.Reconcile == nil || options.IsTerminal == nil {
		return nil, errors.New("watch open, reconciliation, and terminal functions are required")
	}
	if options.InitialBackoff == 0 {
		options.InitialBackoff = 250 * time.Millisecond
	}
	if options.MaximumBackoff == 0 {
		options.MaximumBackoff = 5 * time.Second
	}
	if options.InitialBackoff < 0 || options.MaximumBackoff < options.InitialBackoff {
		return nil, errors.New("watch backoff bounds are invalid")
	}
	if options.IsRetryable == nil {
		options.IsRetryable = func(err error) bool {
			return status.Code(err) == codes.Unavailable
		}
	}
	if options.Sleep == nil {
		options.Sleep = sleepContext
	}

	return &RunWatcher[Value]{
		options: options,
		cursor:  options.AfterSequence,
		backoff: options.InitialBackoff,
	}, nil
}

func sleepContext(ctx context.Context, duration time.Duration) error {
	timer := time.NewTimer(duration)
	defer timer.Stop()
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-timer.C:
		return nil
	}
}

func (watcher *RunWatcher[Value]) reconnect(ctx context.Context, err error) error {
	if !watcher.options.IsRetryable(err) {
		return err
	}
	watcher.stream = nil
	if err := watcher.options.Sleep(ctx, watcher.backoff); err != nil {
		return err
	}
	if watcher.backoff <= watcher.options.MaximumBackoff/2 {
		watcher.backoff *= 2
	} else {
		watcher.backoff = watcher.options.MaximumBackoff
	}
	return nil
}

// Recv returns the next unique contiguous update, reconnecting only this read stream.
func (watcher *RunWatcher[Value]) Recv(ctx context.Context) (RunFeedItem[Value], error) {
	var zero RunFeedItem[Value]
	if watcher.done {
		return zero, io.EOF
	}

	for {
		if err := ctx.Err(); err != nil {
			return zero, err
		}
		if watcher.stream == nil {
			stream, err := watcher.options.Open(ctx, watcher.options.RunID, watcher.cursor)
			if err != nil {
				if err := watcher.reconnect(ctx, err); err != nil {
					return zero, err
				}
				continue
			}
			if stream == nil {
				return zero, errors.New("watch open returned a nil stream")
			}
			watcher.stream = stream
		}

		item, err := watcher.stream.Recv()
		if err != nil {
			if errors.Is(err, io.EOF) {
				reconciled, reconcileErr := watcher.options.Reconcile(
					ctx,
					watcher.options.RunID,
					watcher.cursor,
				)
				if reconcileErr != nil {
					if err := watcher.reconnect(ctx, reconcileErr); err != nil {
						return zero, err
					}
					continue
				}
				if reconciled.RunID != watcher.options.RunID ||
					reconciled.LastSequence != watcher.cursor ||
					!reconciled.Terminal {
					return zero, ErrRunFeedReconciliation
				}
				watcher.done = true
				return zero, io.EOF
			}
			if err := watcher.reconnect(ctx, err); err != nil {
				return zero, err
			}
			continue
		}
		if item.RunID != watcher.options.RunID {
			return zero, ErrRunFeedIdentity
		}
		if item.Sequence <= watcher.cursor {
			continue
		}
		if watcher.cursor == math.MaxUint64 || item.Sequence != watcher.cursor+1 {
			return zero, ErrRunFeedGap
		}

		watcher.cursor = item.Sequence
		watcher.backoff = watcher.options.InitialBackoff
		if watcher.options.IsTerminal(item.Value) {
			watcher.done = true
		}
		return item, nil
	}
}
