import {
  IconAlertTriangle,
  IconPlayerStop,
  IconPlus,
  IconRefresh,
  IconTerminal2,
  IconX,
} from "@tabler/icons-react";
import { Terminal } from "@xterm/xterm";
import { useCallback, useEffect, useRef, useState } from "react";

import {
  closeTerminal,
  openTerminal,
  resizeTerminal,
  signalTerminal,
  terminalContext,
  writeTerminal,
} from "./api";
import {
  decodeTerminalOutput,
  terminalContextChanged,
  terminalDimensions,
  terminalInputChunks,
  terminalLaunchRequested,
} from "./terminal-model";
import type { TerminalContext, TerminalEvent, TerminalKind } from "./types";
import "@xterm/xterm/css/xterm.css";

const MAX_TERMINAL_TABS = 8;
const MAX_SINGLE_TERMINAL_INPUT_BYTES = 256 * 1024;
const MAX_PENDING_TERMINAL_INPUT_BYTES = 512 * 1024;
const TERMINAL_CONTEXT_REFRESH_MS = 1_000;

interface LocalTerminalTab {
  id: string;
  kind: TerminalKind;
  title: string;
}

async function sendTerminalInput(sessionId: string, value: string) {
  for (const dataBase64 of terminalInputChunks(value)) {
    await writeTerminal(sessionId, dataBase64);
  }
}

interface TerminalPaneProps {
  tab: LocalTerminalTab;
  workspaceId: string;
  contextGeneration: number;
  active: boolean;
}

function TerminalPane({
  tab,
  workspaceId,
  contextGeneration,
  active,
}: TerminalPaneProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const sessionIdRef = useRef<string | null>(null);
  const activeRef = useRef(active);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [exited, setExited] = useState(false);

  useEffect(() => {
    activeRef.current = active;
  }, [active]);

  useEffect(() => {
    const container = containerRef.current;
    if (container === null) {
      return;
    }
    const terminal = new Terminal({
      allowProposedApi: false,
      allowTransparency: false,
      convertEol: false,
      cursorBlink: true,
      cursorStyle: "bar",
      disableStdin: false,
      fontFamily:
        '"SFMono-Regular", "Cascadia Code", "Liberation Mono", monospace',
      fontSize: 13,
      lineHeight: 1.2,
      scrollback: 5_000,
      theme: {
        background: "#07101d",
        foreground: "#dce8f7",
        cursor: "#6aa2ff",
        cursorAccent: "#07101d",
        selectionBackground: "#244b78",
        black: "#07101d",
        blue: "#4389ff",
        brightBlue: "#79aaff",
        cyan: "#45c8ed",
        green: "#48c78e",
        red: "#f1767f",
        yellow: "#efb65b",
      },
      windowOptions: {},
    });
    terminal.open(container);
    terminalRef.current = terminal;
    terminal.writeln(
      "\x1b[38;2;106;162;255mColossus TUI\x1b[0m — authenticated; policy and audit enforced",
    );
    const initial = terminalDimensions(
      container.clientWidth,
      container.clientHeight,
    );
    terminal.resize(initial.cols, initial.rows);
    let disposed = false;
    let inputDisposable: { dispose: () => void } | null = null;
    let inputQueue = Promise.resolve();
    let pendingInputBytes = 0;

    const handleEvent = (event: TerminalEvent) => {
      if (disposed) {
        return;
      }
      if (event.type === "output") {
        try {
          terminal.write(decodeTerminalOutput(event.dataBase64));
        } catch {
          setError("Terminal output exceeded the renderer safety limit.");
        }
        return;
      }
      if (event.type === "exited") {
        setExited(true);
        terminal.writeln(
          `\r\n\x1b[38;2;145;162;184mProcess exited${
            event.exitCode === null ? "" : ` with code ${event.exitCode}`
          }.\x1b[0m`,
        );
        return;
      }
      setError(event.message);
      terminal.writeln(`\r\n\x1b[38;2;241;118;127m${event.message}\x1b[0m`);
    };

    void openTerminal(
      workspaceId,
      contextGeneration,
      tab.kind,
      initial.rows,
      initial.cols,
      handleEvent,
    )
      .then((sessionId) => {
        if (disposed) {
          return closeTerminal(sessionId);
        }
        sessionIdRef.current = sessionId;
        setSessionId(sessionId);
        inputDisposable = terminal.onData((data) => {
          const inputBytes = new TextEncoder().encode(data).byteLength;
          if (
            inputBytes > MAX_SINGLE_TERMINAL_INPUT_BYTES ||
            pendingInputBytes + inputBytes > MAX_PENDING_TERMINAL_INPUT_BYTES
          ) {
            setError(
              "Terminal input is arriving faster than it can be delivered.",
            );
            return;
          }
          pendingInputBytes += inputBytes;
          inputQueue = inputQueue
            .then(() => sendTerminalInput(sessionId, data))
            .catch(() => {
              if (!disposed) {
                setError("Terminal input could not be delivered.");
              }
            })
            .finally(() => {
              pendingInputBytes -= inputBytes;
            });
        });
      })
      .catch((reason: unknown) => {
        if (disposed) {
          return;
        }
        const message =
          reason instanceof Error
            ? reason.message
            : "The terminal could not be opened.";
        setError(message);
        terminal.writeln(`\r\n\x1b[38;2;241;118;127m${message}\x1b[0m`);
      });

    const observer = new ResizeObserver(() => {
      if (
        !activeRef.current ||
        container.clientWidth === 0 ||
        container.clientHeight === 0
      ) {
        return;
      }
      const next = terminalDimensions(
        container.clientWidth,
        container.clientHeight,
      );
      terminal.resize(next.cols, next.rows);
      const sessionId = sessionIdRef.current;
      if (sessionId !== null) {
        void resizeTerminal(sessionId, next.rows, next.cols).catch(() => {
          setError("Terminal resize could not be delivered.");
        });
      }
    });
    observer.observe(container);

    return () => {
      disposed = true;
      observer.disconnect();
      const sessionId = sessionIdRef.current;
      sessionIdRef.current = null;
      setSessionId(null);
      if (sessionId !== null) {
        void closeTerminal(sessionId).catch(() => undefined);
      }
      inputDisposable?.dispose();
      terminal.dispose();
      terminalRef.current = null;
    };
  }, [contextGeneration, tab.id, tab.kind, workspaceId]);

  useEffect(() => {
    if (!active) {
      return;
    }
    const frame = requestAnimationFrame(() => {
      const container = containerRef.current;
      const terminal = terminalRef.current;
      if (container === null || terminal === null) {
        return;
      }
      const next = terminalDimensions(
        container.clientWidth,
        container.clientHeight,
      );
      terminal.resize(next.cols, next.rows);
      terminal.focus();
      const sessionId = sessionIdRef.current;
      if (sessionId !== null) {
        void resizeTerminal(sessionId, next.rows, next.cols).catch(
          () => undefined,
        );
      }
    });
    return () => cancelAnimationFrame(frame);
  }, [active]);

  return (
    <section
      className="terminal-pane"
      hidden={!active}
      aria-label={`${tab.title} terminal`}
    >
      <div ref={containerRef} className="terminal-emulator" />
      <div className="terminal-pane-controls">
        {error !== "" ? (
          <span className="terminal-inline-error" role="alert">
            <IconAlertTriangle size={14} stroke={1.8} aria-hidden="true" />
            {error}
          </span>
        ) : null}
        <button
          className="button secondary compact"
          type="button"
          disabled={sessionId === null || exited}
          onClick={() => {
            const sessionId = sessionIdRef.current;
            if (sessionId !== null) {
              void signalTerminal(sessionId, "interrupt");
            }
          }}
        >
          Interrupt
        </button>
        <button
          className="button danger compact"
          type="button"
          disabled={sessionId === null || exited}
          onClick={() => {
            const sessionId = sessionIdRef.current;
            if (sessionId !== null) {
              void signalTerminal(sessionId, "terminate");
            }
          }}
        >
          <IconPlayerStop size={14} stroke={1.8} aria-hidden="true" />
          Terminate
        </button>
      </div>
    </section>
  );
}

export default function TerminalWindow() {
  const [workspaceId, setWorkspaceId] = useState<string | null>(null);
  const [contextGeneration, setContextGeneration] = useState<number | null>(
    null,
  );
  const [workspaceName, setWorkspaceName] = useState("Local workspace");
  const [tabs, setTabs] = useState<LocalTerminalTab[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [contextWasRefreshed, setContextWasRefreshed] = useState(false);

  const addTab = useCallback((kind: TerminalKind) => {
    setTabs((current) => {
      if (current.length >= MAX_TERMINAL_TABS) {
        return current;
      }
      const ordinal = current.filter((tab) => tab.kind === kind).length + 1;
      const tab = {
        id: crypto.randomUUID(),
        kind,
        title: `Colossus TUI ${ordinal}`,
      };
      setActiveTabId(tab.id);
      return [...current, tab];
    });
  }, []);

  useEffect(() => {
    let disposed = false;
    let inFlight = false;
    let currentContext: TerminalContext | null = null;

    const refreshContext = async () => {
      if (inFlight) {
        return;
      }
      inFlight = true;
      try {
        const context = await terminalContext();
        const authorityChanged = terminalContextChanged(
          currentContext,
          context,
        );
        const launchRequested = terminalLaunchRequested(
          currentContext,
          context,
        );
        if (disposed || (!authorityChanged && !launchRequested)) {
          return;
        }
        const previousContext = currentContext;
        currentContext = context;
        if (authorityChanged) {
          setTabs([]);
          setActiveTabId(null);
          if (!context.enabled || context.workspaceId === null) {
            setWorkspaceId(null);
            setContextGeneration(null);
            setWorkspaceName("Local workspace");
            setContextWasRefreshed(previousContext !== null);
            setError(
              "Local terminal access is disabled or has no active workspace.",
            );
            return;
          }
          setWorkspaceId(context.workspaceId);
          setContextGeneration(context.contextGeneration);
          setWorkspaceName(context.workspaceName ?? "Local workspace");
          setContextWasRefreshed(previousContext !== null);
          setError("");
        }
        if (
          launchRequested &&
          context.enabled &&
          context.workspaceId !== null &&
          context.requestedKind !== null
        ) {
          addTab(context.requestedKind);
        }
      } catch (reason: unknown) {
        if (disposed) {
          return;
        }
        setError(
          reason instanceof Error
            ? reason.message
            : "Terminal context is unavailable.",
        );
      } finally {
        inFlight = false;
      }
    };

    const handleFocus = () => void refreshContext();
    void refreshContext();
    const interval = window.setInterval(
      () => void refreshContext(),
      TERMINAL_CONTEXT_REFRESH_MS,
    );
    window.addEventListener("focus", handleFocus);
    return () => {
      disposed = true;
      window.clearInterval(interval);
      window.removeEventListener("focus", handleFocus);
    };
  }, [addTab]);

  const closeTab = useCallback((id: string) => {
    setTabs((current) => {
      const index = current.findIndex((tab) => tab.id === id);
      const next = current.filter((tab) => tab.id !== id);
      setActiveTabId((active) => {
        if (active !== id) {
          return active;
        }
        return next[Math.min(index, next.length - 1)]?.id ?? null;
      });
      return next;
    });
  }, []);

  return (
    <main className="terminal-window-shell">
      <header className="terminal-window-header">
        <div>
          <span className="terminal-window-icon" aria-hidden="true">
            <IconTerminal2 size={21} stroke={1.7} />
          </span>
          <div>
            <strong>Colossus TUI</strong>
            <span>{workspaceName} · authenticated Managed Local session</span>
          </div>
        </div>
        <div className="terminal-window-actions">
          <button
            className="button primary compact"
            type="button"
            disabled={workspaceId === null || tabs.length >= MAX_TERMINAL_TABS}
            onClick={() => addTab("colossus_tui")}
          >
            <IconPlus size={15} stroke={1.8} aria-hidden="true" />
            Colossus TUI
          </button>
        </div>
      </header>

      <nav className="terminal-tabs" aria-label="Terminal sessions">
        {tabs.map((tab) => (
          <div className="terminal-tab" key={tab.id}>
            <button
              type="button"
              aria-current={activeTabId === tab.id ? "page" : undefined}
              onClick={() => setActiveTabId(tab.id)}
            >
              {tab.title}
            </button>
            <button
              className="terminal-tab-close"
              type="button"
              aria-label={`Close ${tab.title}`}
              onClick={() => closeTab(tab.id)}
            >
              <IconX size={14} stroke={1.8} aria-hidden="true" />
            </button>
          </div>
        ))}
      </nav>

      {error !== "" ? (
        <section className="terminal-window-error" role="alert">
          <IconAlertTriangle size={24} stroke={1.6} aria-hidden="true" />
          <div>
            <strong>Colossus TUI unavailable</strong>
            <p>{error}</p>
          </div>
          <button
            className="button secondary"
            type="button"
            onClick={() => window.location.reload()}
          >
            <IconRefresh size={16} stroke={1.8} aria-hidden="true" />
            Retry
          </button>
        </section>
      ) : null}

      <div className="terminal-panes">
        {workspaceId !== null && contextGeneration !== null
          ? tabs.map((tab) => (
              <TerminalPane
                key={tab.id}
                tab={tab}
                workspaceId={workspaceId}
                contextGeneration={contextGeneration}
                active={tab.id === activeTabId}
              />
            ))
          : null}
        {workspaceId !== null && tabs.length === 0 ? (
          <section className="terminal-empty">
            <IconTerminal2 size={30} stroke={1.4} aria-hidden="true" />
            <strong>No TUI sessions</strong>
            <p>
              {contextWasRefreshed
                ? "The Managed Local context changed, so prior sessions were closed. Open a new TUI when ready."
                : "Open the authenticated Colossus TUI from the control above."}
            </p>
          </section>
        ) : null}
      </div>
    </main>
  );
}
