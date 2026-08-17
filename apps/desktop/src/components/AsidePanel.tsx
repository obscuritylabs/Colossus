import {
  IconArrowUp,
  IconMessageCirclePlus,
  IconPlus,
  IconX,
} from "@tabler/icons-react";
import { useEffect, useRef, useState } from "react";

import { isTerminalStatus } from "../types";
import type {
  Aside,
  CommandError,
  Interaction,
  InteractionAnswer,
} from "../types";
import type { RunView } from "../state";
import { InteractionCard } from "./InteractionCard";
import { RunTimeline } from "./RunTimeline";

export interface AsideDraft {
  sourceRunId: string;
  quote: string;
}

interface AsidePanelProps {
  draft: AsideDraft | null;
  view: RunView | undefined;
  conversationViews: readonly RunView[];
  history: readonly Aside[];
  busy: boolean;
  error: CommandError | null;
  readOnly: boolean;
  onCreate: (prompt: string, draft: AsideDraft) => Promise<boolean>;
  onContinue: (prompt: string, view: RunView) => Promise<boolean>;
  onOpen: (aside: Aside) => Promise<void>;
  onNew: () => void;
  onRespond: (
    interaction: Interaction,
    response: InteractionAnswer,
  ) => Promise<void>;
  onClose: (view: RunView | undefined) => Promise<boolean>;
  onDismiss: () => void;
}

export function AsidePanel({
  draft,
  view,
  conversationViews,
  history,
  busy,
  error,
  readOnly,
  onCreate,
  onContinue,
  onOpen,
  onNew,
  onRespond,
  onClose,
  onDismiss,
}: AsidePanelProps) {
  const [prompt, setPrompt] = useState("");
  const [confirmClose, setConfirmClose] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    setPrompt("");
    setConfirmClose(false);
    requestAnimationFrame(() => inputRef.current?.focus());
  }, [draft, view?.run.runId]);

  async function submit() {
    const clean = prompt.trim();
    if (clean === "" || busy) {
      return;
    }
    const accepted =
      view === undefined
        ? draft !== null && (await onCreate(clean, draft))
        : await onContinue(clean, view);
    if (accepted) {
      setPrompt("");
    }
  }

  async function requestDismiss() {
    if (
      view !== undefined &&
      !isTerminalStatus(view.run.status) &&
      !confirmClose
    ) {
      setConfirmClose(true);
      return;
    }
    if (await onClose(view)) {
      onDismiss();
    }
  }

  return (
    <section className="aside-panel" aria-label="Aside conversation">
      <header className="aside-header">
        <div>
          <p className="eyebrow">Side conversation</p>
          <h3>
            Aside
            {readOnly ? <span className="aside-readonly">Archived</span> : null}
          </h3>
          <p className="aside-context-copy">
            Thread messages included · tool traces omitted
          </p>
        </div>
        <div className="aside-header-actions">
          {view !== undefined ? (
            <button
              className="icon-button"
              type="button"
              aria-label="Start a new Aside"
              disabled={busy}
              onClick={onNew}
            >
              <IconPlus size={17} stroke={1.8} aria-hidden="true" />
            </button>
          ) : null}
          <button
            className="icon-button"
            type="button"
            aria-label="Close Aside"
            disabled={busy}
            onClick={() => void requestDismiss()}
          >
            <IconX size={18} stroke={1.8} aria-hidden="true" />
          </button>
        </div>
      </header>

      {confirmClose ? (
        <div className="aside-close-confirm" role="alert">
          <strong>Stop and archive this Aside?</strong>
          <p>
            Its answer may be incomplete, but you can reopen it from this
            thread.
          </p>
          <div>
            <button
              className="button secondary compact"
              type="button"
              disabled={busy}
              onClick={() => setConfirmClose(false)}
            >
              Keep open
            </button>
            <button
              className="button danger compact"
              type="button"
              disabled={busy}
              onClick={() => void requestDismiss()}
            >
              {busy ? "Stopping…" : "Stop & archive"}
            </button>
          </div>
        </div>
      ) : null}

      <div className="aside-scroll">
        {view === undefined ? (
          <div className="aside-empty">
            <span className="aside-empty-icon" aria-hidden="true">
              <IconMessageCirclePlus size={22} stroke={1.6} />
            </span>
            <h4>Explore without changing the main thread</h4>
            <p>
              Aside inherits the thread’s user and assistant messages through
              this moment. Its replies stay separate and are archived when you
              close it.
            </p>
            {draft?.quote ? (
              <blockquote className="aside-quote">{draft.quote}</blockquote>
            ) : null}
          </div>
        ) : (
          <div className="aside-timeline">
            {conversationViews.map((conversationView) => (
              <RunTimeline
                view={conversationView}
                key={conversationView.run.runId}
              />
            ))}
            {view.pendingInteractions.map((interaction) => (
              <InteractionCard
                key={interaction.interactionId}
                interaction={interaction}
                onRespond={onRespond}
              />
            ))}
          </div>
        )}

        {view === undefined && history.length > 0 ? (
          <section className="aside-history" aria-label="Past Asides">
            <h4>Past Asides</h4>
            {history.map((aside) => (
              <button
                type="button"
                key={aside.run.sessionId}
                disabled={busy}
                onClick={() => void onOpen(aside)}
              >
                <span>{aside.run.title}</span>
                <small>{aside.closed ? "Archived" : aside.run.status}</small>
              </button>
            ))}
          </section>
        ) : null}
      </div>

      <div className="aside-composer">
        <textarea
          ref={inputRef}
          rows={3}
          maxLength={65_536}
          value={prompt}
          disabled={busy || draft === null || readOnly}
          placeholder={
            readOnly
              ? "This Aside is archived. Start a new Aside to continue exploring."
              : view === undefined
                ? "Ask about this thread…"
                : "Continue this Aside…"
          }
          aria-label="Aside message"
          onChange={(event) => setPrompt(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              void submit();
            }
          }}
        />
        <button
          className="aside-send"
          type="button"
          aria-label="Send Aside message"
          disabled={busy || draft === null || readOnly || prompt.trim() === ""}
          onClick={() => void submit()}
        >
          <IconArrowUp size={17} stroke={2} aria-hidden="true" />
        </button>
        <p>Enter to send · Shift+Enter for a new line</p>
      </div>
      {error !== null ? (
        <p className="aside-error" role="alert">
          {error.message}
        </p>
      ) : null}
    </section>
  );
}
