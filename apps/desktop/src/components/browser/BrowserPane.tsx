import { useEffect, useRef, useState } from "react";
import {
  IconArrowLeft,
  IconArrowRight,
  IconArrowsMaximize,
  IconArrowsMinimize,
  IconExternalLink,
  IconGlobe,
  IconLoader2,
  IconPlus,
  IconRefresh,
  IconX,
} from "@tabler/icons-react";
import type { BrowserController } from "./useBrowser";
import "./browser.css";

export function BrowserPane({
  controller,
  expanded,
  onExpand,
  onClose,
}: {
  controller: BrowserController;
  expanded: boolean;
  onExpand: () => void;
  onClose: () => void;
}) {
  const { snapshot, command, error, busy, fixture, viewport } = controller;
  const active = snapshot.tabs.find((tab) => tab.id === snapshot.selectedTabId);
  const [address, setAddress] = useState(active?.url ?? "");
  const addressRef = useRef<HTMLInputElement>(null);
  const viewportRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setAddress(active?.url ?? "");
  }, [active?.id, active?.url]);

  useEffect(() => {
    const element = viewportRef.current;
    if (
      element === null ||
      active === undefined ||
      active.url === "" ||
      active.error !== null
    )
      return;
    let disposed = false;
    let sending = false;
    const generation = snapshot.generation;
    const tabId = active.id;
    async function update() {
      if (disposed || sending || element === null) return;
      const overlays = Array.from(
        document.querySelectorAll<HTMLElement>(
          '[role="dialog"], [role="alertdialog"], [role="menu"], [role="listbox"], [data-browser-occluded]',
        ),
      );
      const occluded =
        document.hidden ||
        overlays.some(
          (overlay) =>
            !overlay.contains(element) && overlay.getClientRects().length > 0,
        );
      const bounds = element.getBoundingClientRect();
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
      if (disposed) await viewport({ generation, tabId: null, rect: null });
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
    window.addEventListener("resize", update);
    document.addEventListener("visibilitychange", update);
    void update();
    return () => {
      disposed = true;
      observer.disconnect();
      mutations.disconnect();
      window.clearInterval(timer);
      window.removeEventListener("resize", update);
      document.removeEventListener("visibilitychange", update);
      void viewport({ generation, tabId: null, rect: null });
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
      <header className="browser-heading">
        <IconGlobe size={17} aria-hidden="true" />
        <strong>Browser</strong>
        <span className="browser-session-label">Temporary session</span>
        <button
          className="icon-button"
          type="button"
          aria-label={expanded ? "Restore browser pane" : "Expand browser pane"}
          onClick={onExpand}
        >
          {expanded ? (
            <IconArrowsMinimize size={16} />
          ) : (
            <IconArrowsMaximize size={16} />
          )}
        </button>
        <button
          className="icon-button"
          type="button"
          aria-label="Close browser pane"
          onClick={onClose}
        >
          <IconX size={17} />
        </button>
      </header>
      <div className="browser-tabs" aria-label="Browser tabs">
        {snapshot.tabs.map((tab) => (
          <div
            className={`browser-tab${tab.id === active?.id ? " is-active" : ""}`}
            key={tab.id}
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
                <IconGlobe size={14} aria-hidden="true" />
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
        <button
          className="icon-button"
          type="button"
          aria-label="New browser tab"
          disabled={busy}
          onClick={() => void command({ type: "new", url: "" })}
        >
          <IconPlus size={17} />
        </button>
      </div>
      <form
        className="browser-address-bar"
        onSubmit={(event) => {
          event.preventDefault();
          if (address.trim())
            void command(
              active
                ? { type: "navigate", tabId: active.id, url: address }
                : { type: "new", url: address },
            );
        }}
      >
        <button
          className="icon-button"
          type="button"
          aria-label="Go back"
          disabled={!active?.canGoBack}
          onClick={() =>
            active && void command({ type: "back", tabId: active.id })
          }
        >
          <IconArrowLeft size={17} />
        </button>
        <button
          className="icon-button"
          type="button"
          aria-label="Go forward"
          disabled={!active?.canGoForward}
          onClick={() =>
            active && void command({ type: "forward", tabId: active.id })
          }
        >
          <IconArrowRight size={17} />
        </button>
        <button
          className="icon-button"
          type="button"
          aria-label={active?.loading ? "Stop loading" : "Reload page"}
          disabled={!active?.url}
          onClick={() =>
            active &&
            void command({
              type: active.loading ? "stop" : "reload",
              tabId: active.id,
            })
          }
        >
          {active?.loading ? <IconX size={17} /> : <IconRefresh size={17} />}
        </button>
        <input
          ref={addressRef}
          type="text"
          aria-label="Web address"
          autoComplete="off"
          spellCheck={false}
          value={address}
          placeholder="Enter a URL or localhost:port"
          onChange={(event) => setAddress(event.target.value)}
        />
        <button
          className="icon-button"
          type="button"
          aria-label="Open in system browser"
          disabled={!active?.url}
          onClick={() =>
            active && void command({ type: "open_external", tabId: active.id })
          }
        >
          <IconExternalLink size={17} />
        </button>
      </form>
      {active?.url.startsWith("http:") ? (
        <p className="browser-http-note">HTTP connection · Not encrypted</p>
      ) : null}
      {error ? (
        <div className="browser-notice" role="alert">
          {error}
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
          <button
            className="icon-button"
            type="button"
            aria-label="Dismiss browser notice"
            onClick={() =>
              void command({ type: "dismiss_notice", tabId: active.id })
            }
          >
            <IconX size={14} />
          </button>
        </div>
      ) : null}
      <div className="browser-viewport" ref={viewportRef} aria-label="Web page">
        {active?.error ? (
          <div className="browser-empty" role="alert">
            <IconGlobe size={32} />
            <h3>Unable to display this page</h3>
            <p>{active.error}</p>
            <button
              className="button secondary"
              type="button"
              onClick={() => void command({ type: "reload", tabId: active.id })}
            >
              Retry
            </button>
          </div>
        ) : !active?.url ? (
          <div className="browser-empty">
            <IconGlobe size={34} />
            <h3>Browse beside your work</h3>
            <p>Open a website or local preview using the address bar.</p>
            <span>
              Your browsing session ends when you close its last tab or exit
              Colossus.
            </span>
          </div>
        ) : fixture ? (
          <div className="browser-empty browser-fixture-page">
            <IconGlobe size={32} />
            <h3>{active.title}</h3>
            <p>{active.url}</p>
            <span>Native page content appears here in the Desktop app.</span>
          </div>
        ) : null}
      </div>
      <footer className="browser-footer">
        <span role="status">
          {active?.loading ? (
            <>
              <IconLoader2
                size={13}
                className="browser-spinner"
                aria-hidden="true"
              />{" "}
              Loading page…
            </>
          ) : (
            "Browsing stays separate from your conversation"
          )}
        </span>
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
