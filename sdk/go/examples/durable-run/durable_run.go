// Package durablerun provides an application core for one durable Colossus run.
//
// Connection and credential loading belong in trusted application composition. Pass
// a generated client created from colossus.Dial; never place the bearer in argv, an
// environment variable, a descriptor, a log, or renderer memory.
package durablerun

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"sort"

	colossus "github.com/obscuritylabs/colossus/sdk/go"
	v1alpha1 "github.com/obscuritylabs/colossus/sdk/go/gen/colossus/api/v1alpha1"
	"google.golang.org/grpc"
)

// Result is released terminal output plus the bounded tool names seen in the feed.
type Result struct {
	RunID     string
	Output    string
	ToolNames []string
}

// InteractionHandler explicitly decides one caller-respondable prompt or approval.
type InteractionHandler func(
	context.Context,
	*v1alpha1.Interaction,
) (*v1alpha1.RespondInteractionRequest, error)

// RunFailedError preserves terminal retry and outcome metadata.
type RunFailedError struct {
	Reason           string
	Message          string
	Recoverable      bool
	OutcomeCertainty v1alpha1.OutcomeCertainty
	HTTPStatus       *uint32
	RetryAfterMS     *uint64
}

func (failure *RunFailedError) Error() string {
	return failure.Message
}

// RunPrompt creates once, resumes its read-only watch, and returns released output.
func RunPrompt(
	ctx context.Context,
	client v1alpha1.AgentRunServiceClient,
	prompt string,
	mode v1alpha1.RunMode,
	handleInteraction InteractionHandler,
) (*Result, error) {
	createKey, err := idempotencyKey("create")
	if err != nil {
		return nil, err
	}
	created, err := client.CreateRun(ctx, &v1alpha1.CreateRunRequest{
		Input: []*v1alpha1.ContentPart{{
			Content: &v1alpha1.ContentPart_Text{
				Text: &v1alpha1.TextContent{Text: prompt},
			},
		}},
		Role:           "primary",
		Mode:           mode,
		MaxTurns:       12,
		IdempotencyKey: createKey,
	})
	if err != nil {
		if detail, ok := colossus.DecodeColossusRPCError(err); ok {
			return nil, fmt.Errorf(
				"CreateRun failed: %s; retryable=%t; outcome=%s: %w",
				detail.Reason,
				detail.Retryable,
				detail.OutcomeCertainty,
				err,
			)
		}
		return nil, err
	}
	runID := created.GetRun().GetRunId()
	if runID == "" {
		return nil, errors.New("CreateRun returned no durable run identity")
	}

	watcher, err := colossus.NewRunWatcher(colossus.RunWatchOptions[*v1alpha1.RunUpdate]{
		RunID: runID,
		Open: func(
			watchContext context.Context,
			watchedRunID string,
			afterSequence uint64,
		) (colossus.RunStream[*v1alpha1.RunUpdate], error) {
			stream, openErr := client.WatchRun(watchContext, &v1alpha1.WatchRunRequest{
				RunId:         watchedRunID,
				AfterSequence: afterSequence,
			})
			if openErr != nil {
				return nil, openErr
			}
			return &generatedRunStream{stream: stream}, nil
		},
		Reconcile: func(
			reconcileContext context.Context,
			watchedRunID string,
			_ uint64,
		) (colossus.RunWatchReconciliation, error) {
			response, getErr := client.GetRun(
				reconcileContext,
				&v1alpha1.GetRunRequest{RunId: watchedRunID},
			)
			if getErr != nil {
				return colossus.RunWatchReconciliation{}, getErr
			}
			run := response.GetRun()
			return colossus.RunWatchReconciliation{
				RunID:        run.GetRunId(),
				LastSequence: run.GetLastSequence(),
				Terminal: run.GetResult() != nil ||
					run.GetFailure() != nil ||
					run.GetCancellation() != nil,
			}, nil
		},
		IsTerminal: colossus.IsTerminalRunUpdate[*v1alpha1.RunUpdate],
	})
	if err != nil {
		return nil, err
	}

	toolNames := make(map[string]struct{})
	for {
		item, receiveErr := watcher.Recv(ctx)
		if errors.Is(receiveErr, io.EOF) {
			return nil, errors.New("run watch ended without an exact terminal update")
		}
		if receiveErr != nil {
			return nil, receiveErr
		}
		update := item.Value
		switch {
		case update.GetToolActivity() != nil:
			toolNames[update.GetToolActivity().GetToolName()] = struct{}{}
		case update.GetInteraction() != nil:
			interaction := update.GetInteraction()
			if !interaction.GetRespondableByCaller() {
				continue
			}
			if handleInteraction == nil {
				return nil, errors.New(
					"run is waiting for an interaction; supply an InteractionHandler and " +
						"resume from the last durable cursor",
				)
			}
			request, responseErr := handleInteraction(ctx, interaction)
			if responseErr != nil {
				return nil, responseErr
			}
			if request == nil {
				return nil, errors.New("interaction handler returned no response")
			}
			if _, responseErr = client.RespondInteraction(ctx, request); responseErr != nil {
				return nil, responseErr
			}
		case update.GetResult() != nil:
			names := make([]string, 0, len(toolNames))
			for name := range toolNames {
				names = append(names, name)
			}
			sort.Strings(names)
			return &Result{
				RunID:     runID,
				Output:    update.GetResult().GetOutput(),
				ToolNames: names,
			}, nil
		case update.GetFailure() != nil:
			failure := update.GetFailure().GetFailure()
			if failure == nil {
				return nil, errors.New("run failed without released failure detail")
			}
			return nil, &RunFailedError{
				Reason:           failure.GetReason(),
				Message:          failure.GetMessage(),
				Recoverable:      failure.GetRecoverable(),
				OutcomeCertainty: failure.GetOutcomeCertainty(),
				HTTPStatus:       failure.HttpStatus,
				RetryAfterMS:     failure.RetryAfterMs,
			}
		case update.GetCancellation() != nil:
			return nil, fmt.Errorf("run cancelled: %s", update.GetCancellation().GetMessage())
		}
	}
}

// DenyApproval is a safe example handler. It never answers an ordinary user prompt.
func DenyApproval(
	_ context.Context,
	interaction *v1alpha1.Interaction,
) (*v1alpha1.RespondInteractionRequest, error) {
	approval := interaction.GetApproval()
	if approval == nil {
		return nil, errors.New("ordinary prompts require an application-specific answer")
	}
	key, err := idempotencyKey("interaction")
	if err != nil {
		return nil, err
	}
	return &v1alpha1.RespondInteractionRequest{
		RunId:          interaction.GetRunId(),
		InteractionId:  interaction.GetInteractionId(),
		Etag:           interaction.GetEtag(),
		IdempotencyKey: key,
		Response: &v1alpha1.RespondInteractionRequest_ApprovalAnswer{
			ApprovalAnswer: &v1alpha1.ApprovalAnswer{
				Approved:    false,
				RequestHash: approval.GetRequestHash(),
			},
		},
	}, nil
}

type generatedRunStream struct {
	stream grpc.ServerStreamingClient[v1alpha1.WatchRunResponse]
}

func (stream *generatedRunStream) Recv() (colossus.RunFeedItem[*v1alpha1.RunUpdate], error) {
	response, err := stream.stream.Recv()
	if err != nil {
		return colossus.RunFeedItem[*v1alpha1.RunUpdate]{}, err
	}
	update := response.GetUpdate()
	if update == nil {
		return colossus.RunFeedItem[*v1alpha1.RunUpdate]{}, errors.New(
			"WatchRun returned an empty response",
		)
	}
	return colossus.RunFeedItem[*v1alpha1.RunUpdate]{
		RunID:    update.GetRunId(),
		Sequence: update.GetSequence(),
		Value:    update,
	}, nil
}

func idempotencyKey(operation string) (string, error) {
	var random [16]byte
	if _, err := rand.Read(random[:]); err != nil {
		return "", fmt.Errorf("generate idempotency key: %w", err)
	}
	return "sdk-example-" + operation + "-" + hex.EncodeToString(random[:]), nil
}
