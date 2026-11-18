import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { IconDots, IconDownload } from "@tabler/icons-react";
import { Button, DropdownSelect } from "@colossus/ui";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@colossus/ui/components/ui/dropdown-menu";
import type {
  ConversationExportHost,
  TranscriptDestination,
  TranscriptExportProgress,
  TranscriptFormat,
} from "../transcript-export";
import { MAX_TRANSCRIPT_COPY_BYTES } from "../transcript-export";
import "./conversation-export.css";

export function ConversationExport({
  host,
  onOpen,
}: {
  host: ConversationExportHost;
  onOpen: () => void;
}) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  return (
    <>
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <button
            ref={trigger}
            className="icon-button"
            type="button"
            aria-label="Conversation actions"
            title="Conversation actions"
          >
            <IconDots size={18} aria-hidden />
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="end"
          aria-label="Conversation actions"
          aria-labelledby={undefined}
        >
          <DropdownMenuItem
            onSelect={() => {
              onOpen();
              setOpen(true);
            }}
          >
            <IconDownload size={16} aria-hidden />
            Export conversation…
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      {open ? (
        <ExportDialog
          key={`${host.scopeKey}:${host.available}`}
          host={host}
          returnFocus={trigger.current}
          onClose={() => setOpen(false)}
        />
      ) : null}
    </>
  );
}

function ExportDialog({
  host,
  returnFocus,
  onClose,
}: {
  host: ConversationExportHost;
  returnFocus: HTMLElement | null;
  onClose: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const operation = useRef<{
    id: string;
    cancellation: AbortController;
    cancelNative: ConversationExportHost["cancel"];
    phase: "reading" | "prepared" | "copying";
  } | null>(null);
  const alive = useRef(true);
  const heading = useId();
  const description = useId();
  const formatId = useId();
  const [format, setFormat] = useState<TranscriptFormat>("markdown");
  const [busy, setBusy] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [progress, setProgress] = useState<TranscriptExportProgress | null>(
    null,
  );
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [prepared, setPrepared] = useState<{
    operationId: string;
    text: string;
    messages: number;
    signal: AbortSignal;
    copy: ConversationExportHost["copy"];
  } | null>(null);
  useEffect(() => {
    alive.current = true;
    const element = dialog.current!;
    element.showModal();
    return () => {
      alive.current = false;
      element.close();
      const current = operation.current;
      if (current) {
        current.cancellation.abort();
        if (current.phase === "reading")
          void current.cancelNative(current.id).catch(() => undefined);
      }
      requestAnimationFrame(() => {
        if (returnFocus?.isConnected) returnFocus.focus();
      });
    };
  }, [returnFocus]);

  function clearPrepared() {
    if (operation.current?.phase === "prepared") {
      operation.current.cancellation.abort();
      operation.current = null;
    }
    setPrepared(null);
  }

  async function cancel() {
    const current = operation.current;
    if (!current || current.phase === "prepared") {
      clearPrepared();
      onClose();
      return;
    }
    // Stop a released Copy reply locally before waiting for native cancellation.
    // Native may have completed its read and already dropped its registration.
    current.cancellation.abort();
    setCancelling(true);
    if (current.phase === "copying") return;
    try {
      await current.cancelNative(current.id);
    } catch {
      if (alive.current) {
        setError(
          "Cancellation could not be confirmed. Waiting for the export result.",
        );
        setCancelling(false);
      }
    }
  }

  async function start(destination: TranscriptDestination) {
    if (
      (operation.current && operation.current.phase !== "prepared") ||
      !host.available
    )
      return;
    clearPrepared();
    const id = crypto.randomUUID();
    const cancellation = new AbortController();
    // Keep the cancellation adapter from this exact selection even after a rerender.
    operation.current = {
      id,
      cancellation,
      cancelNative: host.cancel,
      phase: "reading",
    };
    setBusy(true);
    setError("");
    setNotice("");
    setProgress(null);
    try {
      const result = await host.start({
        operationId: id,
        format,
        destination,
        signal: cancellation.signal,
        onProgress: (next) => {
          if (
            alive.current &&
            operation.current?.id === id &&
            !cancellation.signal.aborted
          )
            setProgress(next);
        },
      });
      if (!alive.current || operation.current?.id !== id) return;
      if (result.status === "copy" && !cancellation.signal.aborted) {
        if (
          typeof result.text !== "string" ||
          new TextEncoder().encode(result.text).byteLength >
            MAX_TRANSCRIPT_COPY_BYTES
        )
          throw new Error(
            "The transcript exceeds the copy limit. Choose Save instead.",
          );
        operation.current.phase = "prepared";
        setPrepared({
          operationId: id,
          text: result.text,
          messages: result.messages,
          signal: cancellation.signal,
          copy: host.copy,
        });
        setNotice(
          `Transcript ready (${result.messages.toLocaleString()} messages). Choose Copy transcript to copy it.`,
        );
        return;
      }
      setNotice(
        result.status === "saved"
          ? `Conversation saved (${result.messages.toLocaleString()} messages).`
          : result.status === "copied"
            ? `Transcript copied (${result.messages.toLocaleString()} messages).`
            : "Export cancelled. Nothing was saved or copied.",
      );
    } catch (failure) {
      if (alive.current && operation.current?.id === id)
        setError(
          failure instanceof Error
            ? failure.message
            : "The conversation could not be exported. Nothing was saved or copied.",
        );
    } finally {
      if (alive.current && operation.current?.id === id) {
        if (operation.current.phase !== "prepared") operation.current = null;
        setBusy(false);
        setCancelling(false);
        setProgress(null);
      }
    }
  }

  async function copyPrepared() {
    const current = prepared;
    if (
      !current ||
      !host.available ||
      busy ||
      operation.current?.id !== current.operationId ||
      current.signal.aborted
    )
      return;
    operation.current.phase = "copying";
    setPrepared(null);
    setBusy(true);
    setNotice("");
    setError("");
    try {
      // The host invokes clipboard.writeText in this click's synchronous stack.
      const writing = current.copy(current.text, current.signal);
      await writing;
      if (alive.current && operation.current?.id === current.operationId)
        setNotice(
          `Transcript copied (${current.messages.toLocaleString()} messages).`,
        );
    } catch (failure) {
      if (alive.current && operation.current?.id === current.operationId)
        setError(
          failure instanceof Error
            ? failure.message
            : "The clipboard could not be written. Choose Save instead.",
        );
    } finally {
      if (alive.current && operation.current?.id === current.operationId) {
        operation.current.cancellation.abort();
        operation.current = null;
        setBusy(false);
        setCancelling(false);
      }
    }
  }

  return createPortal(
    <dialog
      ref={dialog}
      className="settings-dialog conversation-export-dialog"
      aria-labelledby={heading}
      aria-describedby={description}
      onKeyDown={(event) => event.stopPropagation()}
      onCancel={(event) => {
        event.preventDefault();
        void cancel();
      }}
    >
      <header>
        <h3 id={heading}>Export conversation</h3>
      </header>
      <div className="conversation-export-content">
        <p id={description}>
          Export the retained conversation across all its runs, including
          messages outside loaded history. The snapshot includes visible user
          and assistant messages.
        </p>
        <p className="muted">
          Tools and hidden reasoning are excluded. Text attachment content
          already saved in messages is included. Image and other attachment
          references are metadata placeholders; export does not fetch files or
          embed image bytes. Running conversations include messages committed
          when export starts. Exports can contain sensitive text you shared.
        </p>
        {!host.available ? (
          <p role="status">
            This runtime does not support full conversation export. Update the
            runtime and reconnect.
          </p>
        ) : null}
        <label htmlFor={formatId}>Format</label>
        <DropdownSelect
          id={formatId}
          value={format}
          disabled={busy}
          onChange={(event) => {
            clearPrepared();
            setNotice("");
            setError("");
            setFormat(event.target.value as TranscriptFormat);
          }}
        >
          <option value="markdown">Markdown (.md)</option>
          <option value="text">Plain text (.txt)</option>
          <option value="json">JSON (.json)</option>
        </DropdownSelect>
        <p className="muted">
          Save supports up to 256 MiB and five minutes of reading. Choose a new
          filename; existing files are kept. Copy supports transcripts up to 2
          MiB.
        </p>
        {busy ? (
          <p role="status" aria-live="polite">
            {cancelling
              ? "Cancelling export…"
              : operation.current?.phase === "copying"
                ? "Copying transcript…"
                : progress?.phase === "choosing_destination"
                  ? "Choose a new file in the Save dialog, or cancel that dialog."
                  : progress?.phase === "saving"
                    ? "Saving the complete transcript…"
                    : `Reading canonical history… ${progress?.messagesRead.toLocaleString() ?? "0"} messages`}
          </p>
        ) : null}
        {notice ? <p role="status">{notice}</p> : null}
        {error ? <p role="alert">{error}</p> : null}
      </div>
      <footer>
        <Button
          variant="secondary"
          disabled={cancelling}
          onClick={() => void cancel()}
        >
          {busy ? "Cancel export" : "Close"}
        </Button>
        <Button
          variant="secondary"
          disabled={busy || !host.available}
          onClick={() => void (prepared ? copyPrepared() : start("copy"))}
        >
          {prepared ? "Copy transcript" : "Prepare copy"}
        </Button>
        <Button
          variant="primary"
          disabled={busy || !host.available}
          onClick={() => void start("save")}
        >
          Save transcript…
        </Button>
      </footer>
    </dialog>,
    document.body,
  );
}
