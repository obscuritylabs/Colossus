import { IconTerminal2 } from "@tabler/icons-react";
import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import type { TerminalKind, TerminalPlanContext } from "../../types";

export interface TerminalDockRequest {
  scope: string | null;
  kind: TerminalKind;
  planContext?: TerminalPlanContext | undefined;
  sequence: number;
}

export function terminalRequestForScope(
  request: TerminalDockRequest | null,
  scope: string | null,
): TerminalDockRequest | null {
  return request?.scope === scope ? request : null;
}

export function TerminalDock({
  ready,
  fixture,
  scope,
  request,
  onSettings,
}: {
  ready: boolean;
  fixture: boolean;
  scope: string | null;
  request: TerminalDockRequest | null;
  onSettings: () => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const [error, setError] = useState("");
  const [retry, setRetry] = useState(0);
  const [loading, setLoading] = useState(true);
  const currentRequest = terminalRequestForScope(request, scope);
  useEffect(() => {
    if (fixture || !ready) return;
    let disposed = false;
    let epoch: number | null = null;
    let sending = false;
    setLoading(true);
    setError("");
    async function update() {
      if (epoch === null || sending || disposed || !host.current) return;
      const occluded =
        document.hidden ||
        Array.from(
          document.querySelectorAll<HTMLElement>(
            '[role="dialog"], [role="alertdialog"], [role="menu"], [role="listbox"], [data-browser-occluded]',
          ),
        ).some(
          (element) =>
            !element.contains(host.current) &&
            element.getClientRects().length > 0,
        );
      const bounds = host.current.getBoundingClientRect();
      sending = true;
      try {
        await invoke("terminal_pane_viewport", {
          request: {
            epoch,
            rect:
              occluded || bounds.width < 16 || bounds.height < 16
                ? null
                : {
                    x: bounds.x,
                    y: bounds.y,
                    width: bounds.width,
                    height: bounds.height,
                  },
          },
        });
      } catch (cause) {
        if (!disposed)
          setError(
            cause instanceof Error
              ? cause.message
              : "The terminal pane could not be displayed. Try again.",
          );
      } finally {
        sending = false;
        if (disposed)
          void invoke("terminal_pane_viewport", {
            request: { epoch, rect: null },
          }).catch(() => {});
      }
    }
    void invoke<number>("mount_terminal_pane", {
      expectedScope: scope,
      request: currentRequest
        ? { kind: currentRequest.kind, ...currentRequest.planContext }
        : null,
      requestSequence: currentRequest?.sequence ?? 0,
    })
      .then((value) => {
        epoch = value;
        if (disposed) {
          void invoke("terminal_pane_viewport", {
            request: { epoch, rect: null },
          }).catch(() => {});
          return;
        }
        setLoading(false);
        void update();
      })
      .catch((cause: unknown) => {
        if (!disposed) {
          setLoading(false);
          setError(
            typeof cause === "object" && cause !== null && "message" in cause
              ? String(cause.message)
              : "Unable to start the terminal. Try again.",
          );
        }
      });
    const observer = new ResizeObserver(() => void update());
    if (host.current) observer.observe(host.current);
    const mutations = new MutationObserver(() => void update());
    mutations.observe(document.body, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: [
        "hidden",
        "open",
        "aria-hidden",
        "data-browser-occluded",
      ],
    });
    const timer = window.setInterval(() => void update(), 400);
    document.addEventListener("visibilitychange", update);
    return () => {
      disposed = true;
      observer.disconnect();
      mutations.disconnect();
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", update);
      if (epoch !== null)
        void invoke("terminal_pane_viewport", {
          request: { epoch, rect: null },
        }).catch(() => {});
    };
  }, [ready, fixture, scope, currentRequest, retry]);
  return (
    <section className="terminal-dock" ref={host} aria-label="Terminal pane">
      {!ready || fixture || loading || error ? (
        <div
          className="terminal-dock-message"
          role={error ? "alert" : "status"}
        >
          <IconTerminal2 size={32} aria-hidden="true" />
          <h3>
            {error
              ? "Terminal unavailable"
              : fixture
                ? "Terminal beside your work"
                : !ready
                  ? "Set up your local terminal"
                  : "Starting terminal…"}
          </h3>
          <p>
            {error ||
              (fixture
                ? "Open the Colossus TUI or a system shell here in the desktop app. Keep both in separate tabs and switch tools without closing your sessions."
                : !ready
                  ? "Enable the local terminal in settings and connect a managed workspace to use the Colossus TUI."
                  : "Connecting to your workspace.")}
          </p>
          {!ready && !fixture ? (
            <button
              className="button secondary"
              type="button"
              onClick={onSettings}
            >
              Open terminal settings
            </button>
          ) : error ? (
            <button
              className="button secondary"
              type="button"
              onClick={() => setRetry((value) => value + 1)}
            >
              Retry
            </button>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}
