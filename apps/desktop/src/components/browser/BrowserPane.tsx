/** @jsxRuntime classic */
/** @jsx element */
/** @jsxFrag Fragment */
// The classic JSX transform keeps this bounded browser surface compact.
import { element, Fragment } from "./browser-jsx";
import { useEffect, useRef, useState } from "react";
import type { TablerIcon } from "@tabler/icons-react";
import {
  IconArrowLeft,
  IconArrowRight,
  IconArrowsMaximize,
  IconArrowsMinimize,
  IconExternalLink,
  IconWorld,
  IconLoader2,
  IconPlus,
  IconRefresh,
  IconX,
} from "@tabler/icons-react";
import type { BrowserController } from "./useBrowser";
import { BrowserCertificates } from "./BrowserCertificates";
import { useDesktopPreferences } from "../../DesktopPreferencesProvider";
import "./browser.css";

function IconButton({
  label,
  icon: Icon,
  disabled,
  onClick,
}: {
  label: string;
  icon: TablerIcon;
  disabled?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      className="icon-button"
      type="button"
      aria-label={label}
      disabled={disabled}
      onClick={onClick}
    >
      <Icon size={17} />
    </button>
  );
}

export function BrowserPane({
  controller,
  run,
  onUseConversation,
  docked = false,
  expanded,
  onExpand,
  onClose,
}: {
  controller: BrowserController;
  run?: { runId: string; sessionId: string } | undefined;
  onUseConversation?: ((sessionId: string) => void) | undefined;
  docked?: boolean;
  expanded: boolean;
  onExpand: () => void;
  onClose: () => void;
}) {
  const { browserNewTabUrl } = useDesktopPreferences();
  const { snapshot, command, error, busy, fixture, viewport } = controller;
  const active = snapshot.tabs.find((tab) => tab.id === snapshot.selectedTabId);
  const [address, setAddress] = useState(active?.url ?? "");
  const [showCertificates, setShowCertificates] = useState(false);
  const addressRef = useRef<HTMLInputElement>(null);
  const selectedTabRef = useRef<HTMLDivElement>(null);
  const viewportRef = useRef<HTMLDivElement>(null);
  const ready =
    snapshot.available &&
    snapshot.engine?.ready !== false &&
    snapshot.engine?.kind !== "unavailable";
  const engineLabel = !ready
    ? "Browser unavailable"
    : !snapshot.engine
      ? "Browser"
      : {
          embedded_chromium: snapshot.engine.preview
            ? "Chromium preview"
            : "Chromium",
          webview2: "WebView2",
          webkit: "WebKit",
          unavailable: "Browser unavailable",
        }[snapshot.engine?.kind ?? "unavailable"];
  const controlLabel = {
    human: "Human control",
    agent: "Agent control",
    paused: "Paused",
    unavailable: "Control unavailable",
  }[ready ? (active?.control ?? "human") : "unavailable"];
  const readOnly = active?.control === "agent" || active?.control === "paused";

  useEffect(() => {
    setAddress(active?.url ?? "");
  }, [active?.id, active?.url]);

  useEffect(() => {
    if (!snapshot.available) setShowCertificates(false);
  }, [snapshot.available]);

  useEffect(() => {
    selectedTabRef.current?.scrollIntoView({
      block: "nearest",
      inline: "nearest",
    });
  }, [active?.id]);

  useEffect(() => {
    const element = viewportRef.current;
    if (!element || !active || !active.url || active.error !== null) return;
    let disposed = false;
    let sending = false;
    const generation = snapshot.generation;
    const tabId = active.id;
    const hide = () => viewport({ generation, tabId: null, rect: null });
    async function update() {
      if (disposed || sending) return;
      const overlays = [
        ...document.querySelectorAll<HTMLElement>(
          '[role="dialog"], [role="alertdialog"], [role="menu"], [role="listbox"], [data-browser-occluded]',
        ),
      ];
      const occluded =
        document.hidden ||
        overlays.some(
          (overlay) =>
            !overlay.contains(element) && overlay.getClientRects().length > 0,
        );
      const bounds = element!.getBoundingClientRect();
      sending = true;
      await viewport({
        generation,
        tabId,
        rect:
          occluded || bounds.width < 16 || bounds.height < 16
            ? null
            : {
                x: bounds.x,
                y: bounds.y,
                width: bounds.width,
                height: bounds.height,
              },
      });
      sending = false;
      if (disposed) await hide();
    }
    const observer = new ResizeObserver(() => void update());
    const mutations = new MutationObserver(() => void update());
    observer.observe(element);
    mutations.observe(document.body, {
      subtree: true,
      childList: true,
      attributes: true,
      attributeFilter: [
        "open",
        "hidden",
        "aria-hidden",
        "data-browser-occluded",
      ],
    });
    const timer = window.setInterval(() => void update(), 400);
    const listeners = [
      [window, "resize"],
      [document, "visibilitychange"],
    ] as const;
    for (const [target, event] of listeners)
      target.addEventListener(event, update);
    void update();
    return () => {
      disposed = true;
      observer.disconnect();
      mutations.disconnect();
      window.clearInterval(timer);
      for (const [target, event] of listeners)
        target.removeEventListener(event, update);
      void hide();
    };
  }, [active?.id, active?.url, active?.error, snapshot.generation, viewport]);

  useEffect(() => {
    function focusAddress(event: KeyboardEvent) {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "l") {
        event.preventDefault();
        addressRef.current?.focus();
        addressRef.current?.select();
      }
    }
    window.addEventListener("keydown", focusAddress);
    return () => window.removeEventListener("keydown", focusAddress);
  }, []);

  return (
    <section className="browser-pane" aria-label="Browser">
      <header className="browser-toolbar">
        <div className="browser-tabs" role="group" aria-label="Browser tabs">
          {snapshot.tabs.length === 0 ? (
            <span className="browser-tab-placeholder">
              <IconWorld size={15} aria-hidden="true" />
              Browser
            </span>
          ) : null}
          {snapshot.tabs.map((tab) => (
            <div
              className={`browser-tab${tab.id === active?.id ? " is-active" : ""}`}
              key={tab.id}
              ref={tab.id === active?.id ? selectedTabRef : null}
            >
              <button
                type="button"
                aria-pressed={tab.id === active?.id}
                onClick={() => void command({ type: "select", tabId: tab.id })}
                title={tab.url || "New tab"}
              >
                {tab.loading ? (
                  <IconLoader2
                    className="browser-spinner"
                    size={14}
                    aria-hidden="true"
                  />
                ) : (
                  <IconWorld size={14} aria-hidden="true" />
                )}
                <span>{tab.title || "New tab"}</span>
              </button>
              <button
                type="button"
                aria-label={`Close tab: ${tab.title || "New tab"}`}
                onClick={() => void command({ type: "close", tabId: tab.id })}
              >
                <IconX size={13} />
              </button>
            </div>
          ))}
        </div>
        <IconButton
          label="New browser tab"
          icon={IconPlus}
          disabled={busy || !snapshot.available}
          onClick={() => void command({ type: "new", url: browserNewTabUrl })}
        />
        <div className="browser-pane-actions" hidden={docked}>
          <IconButton
            label={expanded ? "Restore browser pane" : "Expand browser pane"}
            icon={expanded ? IconArrowsMinimize : IconArrowsMaximize}
            onClick={onExpand}
          />
          <IconButton
            label="Close browser pane"
            icon={IconX}
            onClick={onClose}
          />
        </div>
      </header>
      <form
        className="browser-address-bar"
        onSubmit={(event) => {
          event.preventDefault();
          if (snapshot.available && address.trim() && !readOnly)
            void command(
              active?.url
                ? { type: "navigate", tabId: active.id, url: address }
                : { type: "new", url: address },
            );
        }}
      >
        {(
          [
            ["back", "Go back", IconArrowLeft, !active?.canGoBack],
            ["forward", "Go forward", IconArrowRight, !active?.canGoForward],
            [
              active?.loading ? "stop" : "reload",
              active?.loading ? "Stop loading" : "Reload page",
              active?.loading ? IconX : IconRefresh,
              !active?.url,
            ],
          ] as const
        ).map(([type, label, icon, disabled], index) => (
          <IconButton
            key={index}
            label={label}
            icon={icon}
            disabled={disabled || readOnly}
            onClick={() => active && void command({ type, tabId: active.id })}
          />
        ))}
        <input
          ref={addressRef}
          type="text"
          aria-label="Web address"
          autoComplete="off"
          spellCheck={false}
          value={address}
          readOnly={readOnly}
          placeholder="Enter a URL or localhost:port"
          onChange={(event) => setAddress(event.target.value)}
        />
        <IconButton
          label="Open in system browser"
          icon={IconExternalLink}
          disabled={!active?.url}
          onClick={() =>
            active && void command({ type: "open_external", tabId: active.id })
          }
        />
      </form>
      {active?.url.startsWith("http:") ? (
        <p className="browser-http-note">HTTP connection · Not encrypted</p>
      ) : null}
      {error ? (
        <div className="browser-notice" role="alert">
          {error}
        </div>
      ) : null}
      {snapshot.engine?.message ? (
        <div className="browser-notice" role="status">
          {snapshot.engine.message}
        </div>
      ) : null}
      {active?.notice ? (
        <div className="browser-notice" role="status">
          <span>{active.notice}</span>
          {active.popupUrl ? (
            <>
              <span className="browser-popup-url">{active.popupUrl}</span>
              <button
                className="button secondary compact"
                type="button"
                onClick={() =>
                  void command({ type: "open_popup", tabId: active.id })
                }
              >
                Open new tab
              </button>
            </>
          ) : null}
          <IconButton
            label="Dismiss browser notice"
            icon={IconX}
            onClick={() =>
              void command({ type: "dismiss_notice", tabId: active.id })
            }
          />
        </div>
      ) : null}
      <div className="browser-viewport" ref={viewportRef} aria-label="Web page">
        {showCertificates ? (
          <BrowserCertificates
            controller={controller}
            onClose={() => setShowCertificates(false)}
          />
        ) : null}
        {active?.error || !active?.url || (import.meta.env.DEV && fixture) ? (
          <div
            className={`browser-empty${import.meta.env.DEV && fixture && active?.url && !active.error ? " browser-fixture-page" : ""}`}
            role={active?.error ? "alert" : undefined}
          >
            <IconWorld size={32} />
            <h3>
              {active?.error
                ? "Unable to display this page"
                : !active?.url
                  ? "Browse beside your work"
                  : import.meta.env.DEV
                    ? active.title
                    : ""}
            </h3>
            <p>
              {active?.error ||
                (!active?.url
                  ? "Open a website or local preview in the address bar."
                  : import.meta.env.DEV
                    ? active.url
                    : "")}
            </p>
            {active?.error ? (
              <button
                className="button secondary"
                type="button"
                onClick={() =>
                  void command({ type: "reload", tabId: active.id })
                }
              >
                Retry
              </button>
            ) : (
              <span>
                {!active?.url
                  ? "This session ends when you close its last tab or exit Colossus."
                  : import.meta.env.DEV
                    ? "Native page content appears here in the Desktop app."
                    : ""}
              </span>
            )}
          </div>
        ) : null}
      </div>
      <footer className="browser-footer">
        <span role="status">
          {ready && active?.loading ? (
            <>
              <IconLoader2
                size={13}
                className="browser-spinner"
                aria-hidden="true"
              />{" "}
              Loading page…
            </>
          ) : (
            <>
              {engineLabel} · {controlLabel} · Temporary session
            </>
          )}
        </span>
        {snapshot.engine?.agentControlAvailable &&
        !snapshot.engine.preview &&
        active?.control === "human" &&
        run &&
        active.conversationId === run.sessionId ? (
          <button
            type="button"
            disabled={busy}
            onClick={() =>
              void command({
                type: "handoff",
                tabId: active.id,
                runId: run.runId,
              })
            }
          >
            Give agent control
          </button>
        ) : null}
        {snapshot.engine?.agentControlAvailable &&
        !snapshot.engine.preview &&
        active?.control === "human" &&
        active.conversationId &&
        active.conversationId !== run?.sessionId &&
        onUseConversation ? (
          <button
            type="button"
            disabled={busy}
            onClick={() => onUseConversation(active.conversationId!)}
          >
            Use in a new conversation
          </button>
        ) : null}
        <button
          type="button"
          disabled={!ready}
          onClick={() => setShowCertificates((visible) => !visible)}
          aria-expanded={showCertificates}
        >
          Certificates
        </button>
        <button
          type="button"
          disabled={snapshot.tabs.length === 0 || busy}
          onClick={() => void command({ type: "clear" })}
        >
          Clear session
        </button>
      </footer>
    </section>
  );
}
