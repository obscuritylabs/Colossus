import { useEffect, useMemo, useRef, useState } from "react";
import { readShellSession, stopShellSession } from "../../api";
import {
  appendShellLog,
  shellIsActive,
  type ShellSession,
  type ShellSnapshot,
} from "../../shellSessions";
import "./active-shells.css";

export function ActiveShells({
  scope,
  sessions,
  error,
  loading = false,
  refresh,
}: {
  scope: string;
  sessions: ShellSession[];
  error: string | null;
  loading?: boolean;
  refresh: () => void;
}) {
  const [revision, retry] = useState(0);
  const [selected, setSelected] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState<ShellSnapshot | null>(null);
  const [log, setLog] = useState("");
  const [gap, setGap] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);
  const [stopping, setStopping] = useState(false);
  const [follow, setFollow] = useState(true);
  const [search, setSearch] = useState("");
  const mutation = useRef(0);
  const logView = useRef<HTMLPreElement>(null);
  const selection = useRef(selected);
  selection.current = selected;
  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let cursor = 0;
    let failures = 0;
    setSnapshot(null);
    setLog("");
    setGap(false);
    setLocalError(null);
    setStopping(false);
    if (!selected) return;
    async function read() {
      try {
        const version = mutation.current;
        const response = await readShellSession(scope, selected!, cursor);
        if (cancelled) return;
        cursor = response.next_sequence;
        failures = 0;
        setLocalError(null);
        if (version === mutation.current) setSnapshot(response);
        setGap(
          (previous) => previous || response.gap || response.session.truncated,
        );
        setLog((previous) => appendShellLog(previous, response.chunks).text);
        if (
          shellIsActive(response.session.status) ||
          cursor < response.session.output_sequence
        )
          timer = setTimeout(() => void read(), 1000);
      } catch {
        if (!cancelled) {
          failures++;
          setLocalError("Could not read this shell. Refresh to reconnect.");
          if (failures <= 3) timer = setTimeout(() => void read(), 2000);
        }
      }
    }
    void read();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [scope, selected, revision]);
  useEffect(() => {
    if (follow && logView.current)
      logView.current.scrollTop = logView.current.scrollHeight;
  }, [follow, log]);
  const visibleLog = useMemo(
    () =>
      search
        ? log
            .split("\n")
            .filter((line) => line.toLowerCase().includes(search.toLowerCase()))
            .join("\n")
        : log,
    [log, search],
  );
  const current =
    snapshot?.session ?? sessions.find((session) => session.id === selected);
  async function stop() {
    if (!selected || stopping) return;
    const id = selected;
    mutation.current++;
    setStopping(true);
    setLocalError(null);
    try {
      const response = await stopShellSession(scope, id);
      if (selection.current === id) setSnapshot(response);
      refresh();
    } catch {
      if (selection.current === id)
        setLocalError(
          "Stop was not confirmed. Refresh the session and try again.",
        );
    } finally {
      if (selection.current === id) setStopping(false);
    }
  }
  return (
    <section className="active-shells" aria-label="Active shells">
      <div className="shells-toolbar">
        <strong>
          Active shells ·{" "}
          {error
            ? "Unavailable"
            : loading
              ? "Loading…"
              : sessions.filter((session) => shellIsActive(session.status))
                  .length}
        </strong>
        <button
          type="button"
          onClick={() => {
            refresh();
            retry((value) => value + 1);
          }}
        >
          Refresh
        </button>
      </div>
      <p className="shells-hint">
        Background shells continue across turns until stopped or their deadline.
        Quitting Desktop stops Managed Local shells; external runtimes keep
        their own lifetime.
      </p>
      {error || localError ? <p role="alert">{error ?? localError}</p> : null}
      <div className="shells-list" aria-label="Managed shell sessions">
        {sessions.length === 0 ? (
          <p role="status">
            {loading
              ? "Loading shell sessions…"
              : error
                ? "The shell list could not be loaded."
                : "No tracked shells in this runtime."}
          </p>
        ) : (
          [...sessions]
            .sort(
              (a, b) =>
                Number(shellIsActive(b.status)) -
                  Number(shellIsActive(a.status)) ||
                b.created_at_ms - a.created_at_ms,
            )
            .map((session) => (
              <button
                type="button"
                key={session.id}
                aria-pressed={selected === session.id}
                onClick={() => {
                  setSelected(session.id);
                  retry((value) => value + 1);
                }}
              >
                <code>{session.command}</code>
                <span>
                  {session.status.replaceAll("_", " ")} · {session.lifetime}
                </span>
              </button>
            ))
        )}
      </div>
      {current ? (
        <div className="shells-detail">
          <div className="shells-toolbar">
            <strong>
              {current.status.replaceAll("_", " ")}
              {current.exit_code === null ? "" : ` · exit ${current.exit_code}`}
            </strong>
            {shellIsActive(current.status) ? (
              <button
                type="button"
                onClick={() => void stop()}
                disabled={stopping || current.status === "stopping"}
              >
                {stopping || current.status === "stopping"
                  ? "Stopping…"
                  : "Stop"}
              </button>
            ) : null}
          </div>
          <dl>
            <dt>Command</dt>
            <dd>
              <code>{current.command}</code>
            </dd>
            <dt>Directory</dt>
            <dd>{current.cwd}</dd>
            <dt>Origin</dt>
            <dd>
              Chat {current.session_id} · run {current.run_id}
            </dd>
            <dt>Started</dt>
            <dd>
              {new Date(current.created_at_ms).toLocaleTimeString()}
              {shellIsActive(current.status)
                ? ` · ${Math.max(0, Math.floor((Date.now() - current.created_at_ms) / 1000))}s elapsed`
                : ""}
            </dd>
            <dt>Deadline</dt>
            <dd>
              {current.deadline_ms === null
                ? "Waiting for launch"
                : new Date(current.deadline_ms).toLocaleString()}
            </dd>
          </dl>
          {current.reason ? <p>{current.reason}</p> : null}
          {gap || log.length >= 65536 ? (
            <p className="shells-hint" role="status">
              Some output is no longer retained or was truncated. Showing
              available released logs.
            </p>
          ) : null}
          <div className="shells-toolbar">
            <input
              aria-label="Search shell output"
              placeholder="Search output"
              value={search}
              onChange={(event) => setSearch(event.target.value)}
            />
            <label>
              <input
                type="checkbox"
                checked={follow}
                onChange={(event) => setFollow(event.target.checked)}
              />{" "}
              Follow
            </label>
            <button
              type="button"
              onClick={() =>
                void navigator.clipboard
                  .writeText(log)
                  .catch(() => setLocalError("Could not copy output."))
              }
            >
              Copy
            </button>
          </div>
          <pre ref={logView} tabIndex={0} aria-label="Released shell output">
            {visibleLog || "No released output yet."}
          </pre>
        </div>
      ) : (
        <p className="shells-hint">Select a shell to inspect its output.</p>
      )}
    </section>
  );
}
