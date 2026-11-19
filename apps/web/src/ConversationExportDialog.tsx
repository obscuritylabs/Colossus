import { useEffect, useId, useRef, useState } from "react";
import { Button, DropdownSelect } from "@colossus/ui";
import {
  downloadTranscript,
  MAX_TRANSCRIPT_COPY_BYTES,
  MAX_TRANSCRIPT_DOWNLOAD_BYTES,
  readTranscriptExport,
  type TranscriptFormat,
} from "./transcript-export";
import "./transcript-export.css";

interface PreparedTranscript {
  threadPath: string;
  format: TranscriptFormat;
  text: string;
}

/** Transport and download authority stay in the Web host. */
export function ConversationExportDialog({
  threadPath,
  allowed,
  onClose,
  returnFocus,
}: {
  threadPath: string;
  allowed: boolean;
  onClose: () => void;
  returnFocus: HTMLElement | null;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const operation = useRef<AbortController | null>(null);
  const alive = useRef(false);
  const allowedRef = useRef(allowed);
  allowedRef.current = allowed;
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  const heading = useId(),
    description = useId(),
    formatId = useId();
  const [format, setFormat] = useState<TranscriptFormat>("markdown");
  const context = useRef({ threadPath, format });
  context.current = { threadPath, format };
  const preparedRef = useRef<PreparedTranscript | null>(null);
  const [prepared, setPrepared] = useState<PreparedTranscript | null>(null);
  const [copying, setCopying] = useState(false);
  const [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  useEffect(() => {
    alive.current = true;
    preparedRef.current = null;
    setPrepared(null);
    const element = dialog.current!;
    element.showModal();
    const origin = window.location.pathname + window.location.search;
    const navigation = () => {
      if (window.location.pathname + window.location.search !== origin) {
        operation.current?.abort();
        preparedRef.current = null;
        setPrepared(null);
        closeRef.current();
      }
    };
    window.addEventListener("popstate", navigation);
    window.addEventListener("colossus:web:navigation", navigation);
    return () => {
      alive.current = false;
      operation.current?.abort();
      preparedRef.current = null;
      element.close();
      window.removeEventListener("popstate", navigation);
      window.removeEventListener("colossus:web:navigation", navigation);
      requestAnimationFrame(() => {
        if (returnFocus?.isConnected) returnFocus.focus();
      });
    };
  }, [returnFocus]);
  useEffect(() => {
    operation.current?.abort();
    preparedRef.current = null;
    setPrepared(null);
    setNotice("");
    setError("");
  }, [allowed, threadPath]);

  function clearPrepared() {
    preparedRef.current = null;
    setPrepared(null);
  }

  function close() {
    clearPrepared();
    if (copying) {
      operation.current?.abort();
      onClose();
      return;
    }
    if (operation.current) {
      operation.current.abort();
      operation.current = null;
      setBusy(false);
      setNotice("Export cancelled. Nothing was saved or copied.");
    } else onClose();
  }
  async function start(destination: "save" | "copy") {
    if (operation.current || !allowedRef.current) return;
    clearPrepared();
    const controller = new AbortController();
    operation.current = controller;
    const current = () =>
      alive.current &&
      allowedRef.current &&
      context.current.threadPath === threadPath &&
      context.current.format === format &&
      operation.current === controller &&
      !controller.signal.aborted;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const result = await readTranscriptExport(
        threadPath,
        format,
        controller.signal,
        destination === "copy"
          ? MAX_TRANSCRIPT_COPY_BYTES
          : MAX_TRANSCRIPT_DOWNLOAD_BYTES,
      );
      if (!current()) return;
      if (destination === "copy") {
        const text = new TextDecoder("utf-8", { fatal: true }).decode(
          await result.blob.arrayBuffer(),
        );
        if (!current()) return;
        const next = { threadPath, format, text };
        preparedRef.current = next;
        setPrepared(next);
        setNotice(
          "Transcript ready. Click Copy transcript to copy this snapshot.",
        );
      } else {
        downloadTranscript(result.blob, result.filename);
        setNotice("Download started.");
      }
    } catch (failure) {
      if (current())
        setError(
          failure instanceof Error
            ? failure.message
            : "The transcript could not be exported. Try the loaded conversation actions instead.",
        );
    } finally {
      if (alive.current && operation.current === controller) {
        operation.current = null;
        setBusy(false);
      }
    }
  }

  async function copyPrepared() {
    const ready = preparedRef.current;
    if (
      operation.current ||
      !alive.current ||
      !allowedRef.current ||
      !ready ||
      ready.threadPath !== threadPath ||
      ready.format !== format
    )
      return;
    const controller = new AbortController();
    operation.current = controller;
    setBusy(true);
    setCopying(true);
    setError("");
    setNotice("");
    const current = () =>
      alive.current &&
      allowedRef.current &&
      !controller.signal.aborted &&
      operation.current === controller &&
      preparedRef.current === ready &&
      context.current.threadPath === ready.threadPath &&
      context.current.format === ready.format;
    try {
      // Invoke inside this click's activation, before the first asynchronous wait.
      const written = navigator.clipboard.writeText(ready.text);
      await written;
      if (current()) {
        clearPrepared();
        setNotice("Transcript copied.");
      }
    } catch {
      if (current())
        setError(
          "The transcript could not be copied. Click Copy transcript to retry, or save it instead.",
        );
    } finally {
      if (alive.current && operation.current === controller) {
        operation.current = null;
        setBusy(false);
        setCopying(false);
      }
    }
  }
  const copyReady =
    allowed &&
    prepared !== null &&
    prepared.threadPath === threadPath &&
    prepared.format === format;
  return (
    <dialog
      ref={dialog}
      className="web-transcript-export"
      aria-labelledby={heading}
      aria-describedby={description}
      onCancel={(event) => {
        event.preventDefault();
        close();
      }}
    >
      <header>
        <h2 id={heading}>Export conversation</h2>
      </header>
      <div className="web-transcript-export-content">
        <p id={description}>
          Export the retained conversation across all its runs, including
          messages outside loaded history. The snapshot contains released user
          and assistant messages.
        </p>
        <p>
          Tool activity and hidden reasoning are excluded. Text attachment
          content already saved in messages is included. Image and other
          attachment references are metadata placeholders; export does not fetch
          files or embed image bytes. Running conversations include messages
          committed when export starts.
        </p>
        <label htmlFor={formatId}>Format</label>
        <DropdownSelect
          id={formatId}
          density="touch"
          disabled={busy || !allowed}
          value={format}
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
          Save supports up to 64 MiB. Copy supports up to 2 MiB. If full history
          is unavailable, use the loaded conversation actions.
        </p>
        {!allowed ? (
          <p role="status">
            Read access is required to export this conversation.
          </p>
        ) : null}
        {busy ? (
          <p role="status" aria-live="polite">
            {copying
              ? "Copying transcript…"
              : "Reading the complete conversation snapshot…"}
          </p>
        ) : null}
        {notice ? <p role="status">{notice}</p> : null}
        {error ? <p role="alert">{error}</p> : null}
      </div>
      <footer>
        <Button variant="secondary" onClick={close}>
          {busy && !copying ? "Cancel export" : "Close"}
        </Button>
        <Button
          variant="secondary"
          disabled={busy || !allowed}
          onClick={() => {
            if (copyReady) void copyPrepared();
            else void start("copy");
          }}
        >
          {copyReady ? "Copy transcript" : "Prepare copy"}
        </Button>
        <Button disabled={busy || !allowed} onClick={() => void start("save")}>
          Save transcript
        </Button>
      </footer>
    </dialog>
  );
}
