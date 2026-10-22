import { useEffect, useRef, useState } from "react";
import { Button } from "@colossus/ui";
import { WorkSurfaceHeader } from "@colossus/ui/conversation";
import {
  projectPath,
  request,
  ApiFailure,
  type FleetNode,
  type Task,
  type Thread,
  type ThreadDetailResponse,
} from "./api";
import { sameChatBinding } from "./ChatHistorySidebar.actions";
import { RunComposer, type RunRequest } from "./RunComposer";

export interface ForkDraftSource {
  parent: Thread;
  source: Task;
  key: string;
}
export interface AsideDraftSource extends ForkDraftSource {
  quote: string;
  messageId?: string;
}

function branchComposerRequest(source: Task): RunRequest {
  return {
    input: [],
    plugin_skill_ids: [],
    session_id: null,
    end_user_id: null,
    role: source.request.role,
    mode: "execute",
    research_depth: null,
    research_sources: [],
    plan_action: null,
    branch: null,
    max_turns: 24,
    idempotency_key: "",
  };
}

export function branchTitle(kind: "fork" | "aside", value: string): string {
  const text = `${kind === "fork" ? "Fork of" : "Aside:"} ${value}`
    .replace(/[\s\u0000-\u001f\u007f]+/gu, " ")
    .trim();
  let result = "";
  let bytes = 0;
  const encoder = new TextEncoder();
  for (const point of text) {
    const length = encoder.encode(point).length;
    if (bytes + length > 256) break;
    result += point;
    bytes += length;
  }
  return result;
}

/** A branch is admitted only on Send; the server resolves canonical source context. */
export function ThreadBranchDraft({
  draft,
  kind,
  project,
  nodes,
  disabled,
  onCreated,
  onCancel,
  onChanged,
}: {
  draft: ForkDraftSource | AsideDraftSource;
  kind: "fork" | "aside";
  project: string;
  nodes: FleetNode[];
  disabled: boolean;
  onCreated: (thread: Thread, submitted: RunRequest) => void;
  onCancel: () => void;
  onChanged: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const attempt = useRef<{
    key: string;
    body: {
      source_task_id: string;
      kind: "fork" | "aside";
      expected_revision: number;
      title: string;
      request: RunRequest;
    };
  } | null>(null);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  const quote = "quote" in draft ? draft.quote : "";
  return (
    <section
      className={`thread-branch-draft ${kind === "aside" ? "is-aside" : ""}`}
      aria-label={kind === "fork" ? "New fork" : "New side chat"}
    >
      {kind === "fork" ? (
        <WorkSurfaceHeader
          variant="compact"
          title="Fork chat"
          statusLabel="New conversation"
          actions={
            <Button variant="tertiary" disabled={busy} onClick={onCancel}>
              Discard draft
            </Button>
          }
        />
      ) : (
        <header className="thread-branch-draft-header">
          <h3>Ask about this text</h3>
          <Button variant="tertiary" disabled={busy} onClick={onCancel}>
            Discard draft
          </Button>
        </header>
      )}
      <div className="thread-branch-context">
        <p>
          From <strong>{draft.parent.title || "Untitled chat"}</strong>
        </p>
        {quote ? <blockquote>{quote}</blockquote> : null}
        <p className="field-help">
          Conversation messages are included. Tool traces are omitted. Your
          original chat stays independent.
        </p>
      </div>
      {error ? (
        <p className="alert" role="alert">
          {error}
        </p>
      ) : null}
      <RunComposer
        nodes={nodes}
        nodeId={draft.parent.node_id}
        lockedRuntime
        initialRequest={branchComposerRequest(draft.source)}
        runtimeLabel={kind === "aside" ? "Side chat" : "Forked chat"}
        busy={busy}
        disabled={disabled}
        conversationBranch
        draftSeed={kind === "fork" ? { key: draft.key, text: "" } : undefined}
        label={kind === "aside" ? "Side-chat question" : "Message the fork"}
        action={kind === "aside" ? "Start side chat" : "Start fork"}
        onSubmit={async (submitted) => {
          setBusy(true);
          setError("");
          try {
            if (attempt.current?.key !== submitted.idempotency_key) {
              const [latest, released] = await Promise.all([
                request<ThreadDetailResponse>(
                  `${projectPath(project)}/threads/${encodeURIComponent(draft.parent.thread_id)}`,
                ),
                request<{ task: Task }>(
                  `${projectPath(project)}/tasks/${encodeURIComponent(draft.source.task_id)}`,
                ),
              ]);
              const source = released.task;
              if (
                !sameChatBinding(draft.parent, latest.thread) ||
                latest.thread.archived ||
                source.task_id !== draft.source.task_id ||
                source.thread_id !== draft.parent.thread_id ||
                source.project_id !== project ||
                source.node_id !== draft.parent.node_id ||
                source.subject !== draft.source.subject ||
                source.source_read_only ||
                source.snapshot?.run.archived ||
                source.run_id !== draft.source.run_id ||
                source.snapshot?.run.run_id !== draft.source.run_id ||
                source.snapshot?.run.session_id !== draft.parent.session_id
              ) {
                throw new Error(
                  "The source chat changed. Review its latest state before starting this branch.",
                );
              }
              attempt.current = {
                key: submitted.idempotency_key,
                body: {
                  source_task_id: draft.source.task_id,
                  kind,
                  expected_revision: latest.thread.revision,
                  title: branchTitle(
                    kind,
                    kind === "fork"
                      ? draft.parent.title
                      : quote || draft.parent.title,
                  ),
                  request: quote
                    ? {
                        ...submitted,
                        input: [
                          { text: `Selected text (quoted context):\n${quote}` },
                          ...submitted.input,
                        ],
                      }
                    : submitted,
                },
              };
            }
            const result = await request<{ thread: Thread }>(
              `${projectPath(project)}/threads/${encodeURIComponent(draft.parent.thread_id)}/branches`,
              attempt.current.body,
            );
            if (!alive.current) return false;
            onChanged();
            onCreated(result.thread, submitted);
            return true;
          } catch (error) {
            if (error instanceof ApiFailure && error.status === 409)
              attempt.current = null;
            if (alive.current)
              setError(
                error instanceof Error
                  ? error.message
                  : "The branch could not be confirmed. Retry to reconcile the same request.",
              );
            return false;
          } finally {
            if (alive.current) setBusy(false);
          }
        }}
      />
    </section>
  );
}
