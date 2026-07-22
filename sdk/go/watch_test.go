package colossus

import (
	"context"
	"errors"
	"io"
	"testing"
	"time"

	v1alpha1 "github.com/obscuritylabs/colossus/sdk/go/gen/colossus/api/v1alpha1"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

type watchValue struct {
	terminal bool
}

type fakeStream struct {
	items []RunFeedItem[watchValue]
	index int
	err   error
}

func (stream *fakeStream) Recv() (RunFeedItem[watchValue], error) {
	if stream.index < len(stream.items) {
		item := stream.items[stream.index]
		stream.index++
		return item, nil
	}
	return RunFeedItem[watchValue]{}, stream.err
}

func noSleep(context.Context, time.Duration) error {
	return nil
}

func reconcileTerminal(
	_ context.Context,
	runID string,
	lastSequence uint64,
) (RunWatchReconciliation, error) {
	return RunWatchReconciliation{
		RunID:        runID,
		LastSequence: lastSequence,
		Terminal:     true,
	}, nil
}

func TestTerminalRunUpdateCases(t *testing.T) {
	t.Parallel()
	for _, caseName := range []string{
		RunUpdateCaseResult,
		RunUpdateCaseFailure,
		RunUpdateCaseCancellation,
	} {
		if !IsTerminalRunUpdateCase(caseName) {
			t.Fatalf("expected %q to be terminal", caseName)
		}
	}
	for _, caseName := range []string{"", "state", "notice", "message"} {
		if IsTerminalRunUpdateCase(caseName) {
			t.Fatalf("expected %q to be non-terminal", caseName)
		}
	}
}

func TestTerminalRunUpdates(t *testing.T) {
	t.Parallel()
	var terminalPredicate func(*v1alpha1.RunUpdate) bool = IsTerminalRunUpdate[*v1alpha1.RunUpdate]
	for name, update := range map[string]*v1alpha1.RunUpdate{
		"result": {
			Update: &v1alpha1.RunUpdate_Result{},
		},
		"failure": {
			Update: &v1alpha1.RunUpdate_Failure{},
		},
		"cancellation": {
			Update: &v1alpha1.RunUpdate_Cancellation{},
		},
	} {
		if !terminalPredicate(update) {
			t.Fatalf("expected %s update to be terminal", name)
		}
	}
	if IsTerminalRunUpdate(&v1alpha1.RunUpdate{
		Update: &v1alpha1.RunUpdate_State{},
	}) {
		t.Fatal("state update must remain non-terminal")
	}
	if IsTerminalRunUpdate(&v1alpha1.RunUpdate{}) {
		t.Fatal("unset update must remain non-terminal")
	}
}

func TestWatcherReconnectsFromCursorAndDropsDuplicates(t *testing.T) {
	t.Parallel()
	var openedAfter []uint64
	attempt := 0
	watcher, err := NewRunWatcher(RunWatchOptions[watchValue]{
		RunID: "run-1",
		Open: func(_ context.Context, runID string, after uint64) (RunStream[watchValue], error) {
			openedAfter = append(openedAfter, after)
			attempt++
			if attempt == 1 {
				return &fakeStream{
					items: []RunFeedItem[watchValue]{
						{RunID: runID, Sequence: 1, Value: watchValue{}},
					},
					err: status.Error(codes.Unavailable, "transient"),
				}, nil
			}
			return &fakeStream{
				items: []RunFeedItem[watchValue]{
					{RunID: runID, Sequence: 1, Value: watchValue{}},
					{RunID: runID, Sequence: 2, Value: watchValue{terminal: true}},
				},
				err: io.EOF,
			}, nil
		},
		Reconcile:  reconcileTerminal,
		IsTerminal: func(value watchValue) bool { return value.terminal },
		Sleep:      noSleep,
	})
	if err != nil {
		t.Fatal(err)
	}

	first, err := watcher.Recv(context.Background())
	if err != nil || first.Sequence != 1 {
		t.Fatalf("unexpected first update %#v, %v", first, err)
	}
	second, err := watcher.Recv(context.Background())
	if err != nil || second.Sequence != 2 {
		t.Fatalf("unexpected second update %#v, %v", second, err)
	}
	if _, err := watcher.Recv(context.Background()); !errors.Is(err, io.EOF) {
		t.Fatalf("expected terminal EOF, got %v", err)
	}
	if len(openedAfter) != 2 || openedAfter[0] != 0 || openedAfter[1] != 1 {
		t.Fatalf("unexpected reconnect cursors %#v", openedAfter)
	}
}

func TestWatcherCleanEOFAtTerminalCursorDoesNotReconnect(t *testing.T) {
	t.Parallel()
	attempts := 0
	watcher, err := NewRunWatcher(RunWatchOptions[watchValue]{
		RunID:         "run-1",
		AfterSequence: 9,
		Open: func(_ context.Context, _ string, after uint64) (RunStream[watchValue], error) {
			attempts++
			if after != 9 {
				t.Fatalf("unexpected cursor %d", after)
			}
			return &fakeStream{err: io.EOF}, nil
		},
		Reconcile:  reconcileTerminal,
		IsTerminal: func(value watchValue) bool { return value.terminal },
		Sleep: func(context.Context, time.Duration) error {
			t.Fatal("clean EOF must not sleep or reconnect")
			return nil
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := watcher.Recv(context.Background()); !errors.Is(err, io.EOF) {
		t.Fatalf("expected clean EOF, got %v", err)
	}
	if attempts != 1 {
		t.Fatalf("expected one stream, got %d", attempts)
	}
}

func TestWatcherFailsClosedOnGap(t *testing.T) {
	t.Parallel()
	watcher, err := NewRunWatcher(RunWatchOptions[watchValue]{
		RunID: "run-1",
		Open: func(_ context.Context, runID string, _ uint64) (RunStream[watchValue], error) {
			return &fakeStream{
				items: []RunFeedItem[watchValue]{
					{RunID: runID, Sequence: 2, Value: watchValue{terminal: true}},
				},
			}, nil
		},
		Reconcile:  reconcileTerminal,
		IsTerminal: func(value watchValue) bool { return value.terminal },
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := watcher.Recv(context.Background()); !errors.Is(err, ErrRunFeedGap) {
		t.Fatalf("expected gap error, got %v", err)
	}
}

func TestWatcherRetriesOnlyUnavailable(t *testing.T) {
	t.Parallel()
	attempt := 0
	watcher, err := NewRunWatcher(RunWatchOptions[watchValue]{
		RunID: "run-1",
		Open: func(_ context.Context, runID string, _ uint64) (RunStream[watchValue], error) {
			attempt++
			if attempt == 1 {
				return nil, status.Error(codes.Unavailable, "transient")
			}
			return &fakeStream{
				items: []RunFeedItem[watchValue]{
					{RunID: runID, Sequence: 1, Value: watchValue{terminal: true}},
				},
			}, nil
		},
		Reconcile:  reconcileTerminal,
		IsTerminal: func(value watchValue) bool { return value.terminal },
		Sleep:      noSleep,
	})
	if err != nil {
		t.Fatal(err)
	}
	item, err := watcher.Recv(context.Background())
	if err != nil || item.Sequence != 1 || attempt != 2 {
		t.Fatalf("unexpected retry result %#v, %v, attempts=%d", item, err, attempt)
	}
}

func TestWatcherCleanEOFFailsWithoutExactTerminalReconciliation(t *testing.T) {
	t.Parallel()
	watcher, err := NewRunWatcher(RunWatchOptions[watchValue]{
		RunID: "run-1",
		Open: func(context.Context, string, uint64) (RunStream[watchValue], error) {
			return &fakeStream{err: io.EOF}, nil
		},
		Reconcile: func(
			_ context.Context,
			runID string,
			lastSequence uint64,
		) (RunWatchReconciliation, error) {
			return RunWatchReconciliation{
				RunID:        runID,
				LastSequence: lastSequence + 1,
				Terminal:     true,
			}, nil
		},
		IsTerminal: func(value watchValue) bool { return value.terminal },
	})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := watcher.Recv(context.Background()); !errors.Is(
		err,
		ErrRunFeedReconciliation,
	) {
		t.Fatalf("expected reconciliation error, got %v", err)
	}
}
