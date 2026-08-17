import {
  IconAlertTriangle,
  IconClock,
  IconPencil,
  IconRotateClockwise,
  IconTrash,
} from "@tabler/icons-react";
import { useState } from "react";
import type { KeyboardEvent } from "react";

import type { QueuedMessage } from "../message-queue";

interface NextUpQueueProps {
  messages: readonly QueuedMessage[];
  onEdit: (messageId: string, prompt: string) => void;
  onDelete: (messageId: string) => void;
  onRetry: (messageId: string) => void;
}

export function NextUpQueue({
  messages,
  onEdit,
  onDelete,
  onRetry,
}: NextUpQueueProps) {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draft, setDraft] = useState("");

  if (messages.length === 0) {
    return null;
  }

  function beginEditing(message: QueuedMessage) {
    setEditingId(message.id);
    setDraft(message.prompt);
  }

  function finishEditing(messageId: string) {
    const prompt = draft.trim();
    if (prompt.length === 0) {
      return;
    }
    onEdit(messageId, prompt);
    setEditingId(null);
    setDraft("");
  }

  function handleEditKeyDown(
    event: KeyboardEvent<HTMLTextAreaElement>,
    messageId: string,
  ) {
    if (event.key === "Escape") {
      event.preventDefault();
      setEditingId(null);
      setDraft("");
      return;
    }
    if (
      event.key === "Enter" &&
      (event.metaKey || event.ctrlKey) &&
      !event.nativeEvent.isComposing
    ) {
      event.preventDefault();
      finishEditing(messageId);
    }
  }

  return (
    <section className="next-up-queue" aria-label="Next up">
      <header>
        <span className="next-up-heading">
          <IconClock size={15} stroke={1.8} aria-hidden="true" />
          <strong>Next up</strong>
          <span className="next-up-count">{messages.length}</span>
        </span>
        <span>Sent in order when this thread is ready.</span>
      </header>
      <ol>
        {messages.map((message, index) => {
          const editing = editingId === message.id;
          const uncertain = message.error?.outcomeUnknown === true;
          return (
            <li
              className={`next-up-item is-${message.state}${uncertain ? " is-uncertain" : ""}`}
              key={message.id}
            >
              <div className="next-up-item-main">
                <span className="next-up-position" aria-hidden="true">
                  {message.state === "sending" ? (
                    <span className="spinner" />
                  ) : message.state === "failed" ? (
                    <IconAlertTriangle size={14} stroke={1.9} />
                  ) : (
                    index + 1
                  )}
                </span>
                {editing ? (
                  <div className="next-up-editor">
                    <textarea
                      value={draft}
                      rows={2}
                      maxLength={65_536}
                      aria-label="Edit queued message"
                      autoFocus
                      onChange={(event) => setDraft(event.target.value)}
                      onKeyDown={(event) =>
                        handleEditKeyDown(event, message.id)
                      }
                    />
                    <div>
                      <button
                        className="text-button"
                        type="button"
                        disabled={draft.trim().length === 0}
                        onClick={() => finishEditing(message.id)}
                      >
                        Save
                      </button>
                      <button
                        className="text-button"
                        type="button"
                        onClick={() => {
                          setEditingId(null);
                          setDraft("");
                        }}
                      >
                        Cancel
                      </button>
                    </div>
                  </div>
                ) : (
                  <div className="next-up-copy">
                    <p>{message.prompt}</p>
                    <span>
                      {message.mode === "plan"
                        ? "Plan"
                        : message.mode === "research"
                          ? "Research"
                          : "Execute"}
                      {message.attachments.length === 0
                        ? ""
                        : ` · ${message.attachments.length} attachment${message.attachments.length === 1 ? "" : "s"}`}
                      {message.state === "sending"
                        ? " · Sending…"
                        : message.state === "failed"
                          ? uncertain
                            ? " · Delivery uncertain"
                            : " · Not sent"
                          : ""}
                    </span>
                    {message.error === null ? null : (
                      <span className="next-up-error">
                        {message.error.message}
                      </span>
                    )}
                  </div>
                )}
              </div>
              {editing ? null : (
                <div className="next-up-actions">
                  {message.state === "failed" ? (
                    <button
                      className="icon-button"
                      type="button"
                      aria-label="Retry queued message"
                      title="Retry with the same request identity"
                      onClick={() => onRetry(message.id)}
                    >
                      <IconRotateClockwise
                        size={15}
                        stroke={1.8}
                        aria-hidden="true"
                      />
                    </button>
                  ) : null}
                  <button
                    className="icon-button"
                    type="button"
                    aria-label="Edit queued message"
                    title={
                      uncertain
                        ? "Refresh the thread before changing an uncertain delivery"
                        : "Edit queued message"
                    }
                    disabled={message.state === "sending" || uncertain}
                    onClick={() => beginEditing(message)}
                  >
                    <IconPencil size={15} stroke={1.8} aria-hidden="true" />
                  </button>
                  <button
                    className="icon-button"
                    type="button"
                    aria-label="Delete queued message"
                    title="Remove from Next up"
                    disabled={message.state === "sending"}
                    onClick={() => onDelete(message.id)}
                  >
                    <IconTrash size={15} stroke={1.8} aria-hidden="true" />
                  </button>
                </div>
              )}
            </li>
          );
        })}
      </ol>
    </section>
  );
}
