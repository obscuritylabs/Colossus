import { useEffect, useRef, useState } from "react";
import { Button, DropdownSelect } from "@colossus/ui";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import {
  FleetTerminalController,
  type FleetTerminalState,
} from "./fleet-terminal-controller";
import {
  MAX_TERMINAL_COPY,
  terminalPresentation,
} from "./fleet-terminal-model";
import "@xterm/xterm/css/xterm.css";
import "./fleet-terminal.css";

export default function FleetTerminal({
  project,
  node,
  workspace,
  scope,
  onDetach,
}: {
  project: string;
  node: string;
  workspace: string;
  scope: string;
  onDetach: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const container = useRef<HTMLDivElement>(null);
  const controller = useRef<FleetTerminalController | null>(null);
  const terminal = useRef<Terminal | null>(null);
  const detach = useRef(onDetach);
  detach.current = onDetach;
  const [state, setState] = useState<FleetTerminalState>({
    phase: "connecting",
    message: "Connecting to workspace…",
    controlling: false,
    session: null,
    sessions: [],
  });
  const [copyError, setCopyError] = useState("");
  const [closing, setClosing] = useState(false);
  const [announcements, setAnnouncements] = useState(false);
  const announcementMode = useRef(false);
  useEffect(() => {
    const element = container.current;
    if (!element) return;
    dialog.current?.showModal();
    const emulator = new Terminal({
      ...terminalPresentation(element),
      screenReaderMode: announcementMode.current,
      disableStdin: true,
    });
    const fit = new FitAddon();
    emulator.loadAddon(fit);
    emulator.open(element);
    emulator.textarea?.setAttribute(
      "aria-label",
      `Shell input for ${workspace}`,
    );
    emulator.textarea?.setAttribute("aria-describedby", "fleet-shell-help");
    terminal.current = emulator;
    let disposed = false;
    let wasLive = false;
    let resizeTimer: ReturnType<typeof setTimeout> | undefined;
    const fitted = () => {
      const dimensions = fit.proposeDimensions();
      if (!dimensions) return;
      const cols = Math.max(2, Math.min(512, dimensions.cols));
      const rows = Math.max(2, Math.min(512, dimensions.rows));
      emulator.resize(cols, rows);
      void host.resize(rows, cols);
    };
    const host = new FleetTerminalController(
      emulator,
      project,
      node,
      scope,
      (value) => {
        if (disposed) return;
        setState(value);
        emulator.options.disableStdin =
          !value.controlling || value.phase !== "live";
        if (value.phase === "live" && !wasLive) {
          wasLive = true;
          fitted();
          emulator.focus();
        } else if (value.phase !== "live") wasLive = false;
      },
    );
    controller.current = host;
    const copy = () => {
      const selection = emulator.getSelection();
      if (!selection) return;
      if (new TextEncoder().encode(selection).length > MAX_TERMINAL_COPY) {
        setCopyError("Select at most 256 KiB of terminal text to copy.");
        return;
      }
      void navigator.clipboard
        .writeText(selection)
        .then(() => setCopyError(""))
        .catch(() =>
          setCopyError("The selected terminal text could not be copied."),
        );
    };
    emulator.attachCustomKeyEventHandler((event) => {
      if (event.type !== "keydown") return true;
      if (event.shiftKey && event.key === "Escape") {
        event.preventDefault();
        detach.current();
        return false;
      }
      if (
        event.isTrusted &&
        !event.altKey &&
        (event.ctrlKey || event.metaKey) &&
        event.key.toLowerCase() === "c" &&
        emulator.hasSelection()
      ) {
        event.preventDefault();
        copy();
        return false;
      }
      return true;
    });
    // PTY output has no clipboard or navigation authority. No OSC clipboard addon.
    const ignoreClipboard = emulator.parser.registerOscHandler(52, () => true);
    const input = emulator.onData((data) =>
      host.input(new TextEncoder().encode(data)),
    );
    const binary = emulator.onBinary((data) =>
      host.input(
        Uint8Array.from(data, (character) => character.charCodeAt(0) & 255),
      ),
    );
    const observer = new ResizeObserver(() => {
      clearTimeout(resizeTimer);
      resizeTimer = setTimeout(fitted, 100);
    });
    observer.observe(element);
    const appearance = new MutationObserver(() => {
      Object.assign(emulator.options, {
        ...terminalPresentation(element),
        screenReaderMode: announcementMode.current,
      });
      fitted();
    });
    appearance.observe(document.documentElement, {
      attributes: true,
      attributeFilter: [
        "data-theme",
        "data-palette",
        "data-text-size",
        "style",
        "class",
      ],
    });
    fitted();
    void host.connect();
    return () => {
      disposed = true;
      clearTimeout(resizeTimer);
      observer.disconnect();
      appearance.disconnect();
      input.dispose();
      binary.dispose();
      ignoreClipboard.dispose();
      host.dispose();
      emulator.dispose();
      controller.current = null;
      terminal.current = null;
    };
  }, [project, node, scope, workspace]);
  async function close() {
    setClosing(true);
    if (await controller.current?.closeShell()) onDetach();
    else setClosing(false);
  }
  return (
    <dialog
      ref={dialog}
      className="fleet-terminal-dialog"
      aria-labelledby="fleet-shell-title"
      aria-describedby="fleet-shell-authority"
      onCancel={(event) => event.preventDefault()}
    >
      <header className="fleet-terminal-header">
        <div>
          <h2 id="fleet-shell-title">Shell · {workspace}</h2>
          <p id="fleet-shell-authority">
            Runs as the host OS user, outside agent tool policy. The host
            controls shell availability.
          </p>
        </div>
        <Button variant="tertiary" onClick={onDetach}>
          Detach
        </Button>
      </header>
      <div className="fleet-terminal-toolbar">
        <span role="status" aria-live="polite">
          {state.message}
        </span>
        {state.sessions.length > 1 ? (
          <label className="fleet-terminal-session">
            Shell session
            <DropdownSelect
              aria-label="Shell session"
              value={state.session?.terminal_id ?? ""}
              disabled={state.phase === "connecting" || closing}
              onChange={(event) =>
                void controller.current?.selectSession(event.target.value)
              }
            >
              {!state.session ? (
                <option value="" disabled>
                  Choose a shell
                </option>
              ) : null}
              {state.sessions.map((session, index) => (
                <option key={session.terminal_id} value={session.terminal_id}>
                  Shell {index + 1} · {session.status}
                </option>
              ))}
            </DropdownSelect>
          </label>
        ) : null}
        <div className="fleet-terminal-actions">
          {state.phase === "disconnected" || state.phase === "error" ? (
            <Button
              variant="secondary"
              onClick={() => void controller.current?.connect()}
            >
              Reconnect
            </Button>
          ) : null}
          {state.phase === "takeover" ||
          (state.phase === "live" && !state.controlling) ? (
            <Button
              variant="secondary"
              onClick={() => void controller.current?.connect(true)}
            >
              Take control
            </Button>
          ) : null}
          <Button
            variant="tertiary"
            disabled={!state.controlling || state.phase !== "live"}
            onClick={() => void controller.current?.interrupt()}
          >
            Interrupt
          </Button>
          <Button
            variant="tertiary"
            onClick={() => {
              const selection = terminal.current?.getSelection();
              if (!selection) {
                setCopyError("Select terminal text first.");
                return;
              }
              if (
                new TextEncoder().encode(selection).length > MAX_TERMINAL_COPY
              ) {
                setCopyError(
                  "Select at most 256 KiB of terminal text to copy.",
                );
                return;
              }
              void navigator.clipboard
                .writeText(selection)
                .then(() => setCopyError(""))
                .catch(() =>
                  setCopyError(
                    "The selected terminal text could not be copied.",
                  ),
                );
            }}
          >
            Copy selection
          </Button>
          <Button
            variant={announcements ? "secondary" : "tertiary"}
            aria-pressed={announcements}
            onClick={() => {
              const enabled = !announcements;
              announcementMode.current = enabled;
              setAnnouncements(enabled);
              if (terminal.current) {
                terminal.current.options.screenReaderMode = enabled;
                terminal.current.focus();
              }
            }}
          >
            Screen reader announcements
          </Button>
          <Button
            variant="secondary"
            disabled={
              closing ||
              !state.session ||
              (!state.controlling &&
                state.phase !== "history_gap" &&
                state.phase !== "exited")
            }
            onClick={() => void close()}
          >
            {closing ? "Closing…" : "Close Shell"}
          </Button>
          {state.phase !== "connecting" ? (
            <Button
              variant="secondary"
              disabled={closing}
              onClick={() => void controller.current?.startNew()}
            >
              New Shell
            </Button>
          ) : null}
        </div>
      </div>
      {copyError ? (
        <p className="fleet-terminal-error" role="alert">
          {copyError}
        </p>
      ) : null}
      <div
        ref={container}
        className="fleet-terminal-screen"
        role="region"
        aria-label={`Terminal for ${workspace}`}
      />
      <p id="fleet-shell-help" className="fleet-terminal-help">
        Ctrl-C interrupts. With text selected, Ctrl-C copies. Shift-Escape
        detaches; the shell keeps running. Close Shell ends it. Reconnect never
        replays typed commands.
      </p>
    </dialog>
  );
}
