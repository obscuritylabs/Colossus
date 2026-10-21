import { useEffect, useRef, useState } from "react";
import { Button } from "@colossus/ui";
import { IconPaperclip } from "@tabler/icons-react";
import { projectPath, request } from "./api";
import type { Task } from "./api";
import { errorMessage } from "./resources";
import "./hosted-files.css";

export interface HostedAttachment {
  artifact_id: string;
  file_name: string;
  size_bytes: number;
}

/** Retained opaque input references remain downloadable after a conversation reload. */
export function HostedFileLinks({
  project,
  task,
}: {
  project: string;
  task: Task | undefined;
}) {
  const files =
    task?.request.input.flatMap((part) =>
      part.artifact ? [part.artifact] : [],
    ) ?? [];
  if (!files.length) return null;
  return (
    <p className="field-help">
      {files.map((id, index) => (
        <span key={id}>
          {index ? " · " : ""}
          <a
            href={`${projectPath(project)}/hosted/files/${encodeURIComponent(id)}`}
            download
          >
            Download attachment {index + 1}
          </a>
        </span>
      ))}
    </p>
  );
}

/** Uploads are owned by the authenticated account; names never select host paths. */
export function HostedAttachments({
  project,
  files,
  onChange,
  disabled,
  onBusy,
}: {
  project: string;
  files: HostedAttachment[];
  onChange: (files: HostedAttachment[]) => void;
  disabled: boolean;
  onBusy: (busy: boolean) => void;
}) {
  const input = useRef<HTMLInputElement>(null);
  const active = useRef(false);
  const lifetime = useRef(new AbortController());
  useEffect(() => {
    lifetime.current = new AbortController();
    return () => lifetime.current.abort();
  }, []);
  const [error, setError] = useState("");
  const [uploading, setUploading] = useState(false);
  async function upload(file: File) {
    if (active.current || disabled) return;
    setError("");
    if (file.size > 256 * 1024 || files.length >= 4) {
      setError("Attach up to four text files, each no larger than 256 KiB.");
      return;
    }
    const extension = file.name.split(".").pop()?.toLowerCase();
    const mediaType = (
      {
        txt: "text/plain",
        md: "text/markdown",
        csv: "text/csv",
        json: "application/json",
      } as Record<string, string>
    )[extension ?? ""];
    if (!mediaType) {
      setError("Choose a TXT, Markdown, CSV, or JSON file.");
      return;
    }
    active.current = true;
    setUploading(true);
    onBusy(true);
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      new TextDecoder("utf-8", { fatal: true }).decode(bytes);
      let binary = "";
      for (const byte of bytes) binary += String.fromCharCode(byte);
      const result = await request<{ artifact: HostedAttachment }>(
        `${projectPath(project)}/hosted/files`,
        {
          file_name: file.name,
          media_type: mediaType,
          content_base64: btoa(binary),
          idempotency_key: crypto.randomUUID(),
        },
        lifetime.current.signal,
      );
      if (!lifetime.current.signal.aborted)
        onChange([...files, result.artifact]);
    } catch (cause) {
      if (!lifetime.current.signal.aborted) setError(errorMessage(cause));
    } finally {
      active.current = false;
      if (!lifetime.current.signal.aborted) {
        setUploading(false);
        onBusy(false);
      }
    }
  }
  return (
    <>
      <input
        ref={input}
        type="file"
        hidden
        accept=".txt,.md,.csv,.json"
        aria-label="Choose an attachment"
        onChange={(event) => {
          const file = event.target.files?.[0];
          event.target.value = "";
          if (file) void upload(file);
        }}
      />
      <Button
        type="button"
        variant="tertiary"
        disabled={disabled || uploading || files.length >= 4}
        onClick={() => input.current?.click()}
      >
        <IconPaperclip size={16} aria-hidden="true" />
        {uploading ? "Uploading…" : "Attach file"}
      </Button>
      {files.map((file) => (
        <Button
          type="button"
          key={file.artifact_id}
          variant="tertiary"
          className="hosted-file-chip"
          title={file.file_name}
          disabled={disabled || uploading}
          aria-label={`Remove ${file.file_name}`}
          onClick={() =>
            onChange(
              files.filter((item) => item.artifact_id !== file.artifact_id),
            )
          }
        >
          <span className="hosted-file-name">{file.file_name}</span>
          <span aria-hidden="true">×</span>
        </Button>
      ))}
      {error ? <span role="alert">{error}</span> : null}
    </>
  );
}
