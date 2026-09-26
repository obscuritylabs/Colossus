import type { BrowserApi, BrowserSnapshot, BrowserTab } from "../browser-api";

const sessions = new Map<string, BrowserSnapshot>();
let nextId = 0;

export function browserFixture(scope: string): BrowserApi {
  if (!sessions.has(scope))
    sessions.set(scope, {
      available: true,
      generation: ++nextId,
      tabs: [],
      selectedTabId: null,
    });
  const state = sessions.get(scope)!;
  const copy = () => structuredClone(state);
  function add(address: string) {
    if (state.tabs.length >= 8)
      throw new Error(
        "Eight browser tabs are already open. Close a tab before opening another.",
      );
    const tab: BrowserTab = {
      id: `fixture-browser-${++nextId}`,
      url: "",
      title: "New tab",
      canGoBack: false,
      canGoForward: false,
      loading: false,
      error: null,
      notice: null,
      popupUrl: null,
    };
    state.tabs.push(tab);
    state.selectedTabId = tab.id;
    if (address) navigate(tab, address);
  }
  function navigate(tab: BrowserTab, address: string) {
    const url = new URL(
      address.includes("://") ? address : `https://${address}`,
    );
    if (
      !["http:", "https:"].includes(url.protocol) ||
      url.username ||
      url.password
    )
      throw new Error(
        "Enter an HTTP or HTTPS address without embedded credentials.",
      );
    tab.canGoBack = tab.url !== "";
    tab.url = url.href;
    tab.title = url.hostname;
    tab.loading = true;
    tab.error = null;
    window.setTimeout(() => {
      tab.loading = false;
      if (url.hostname === "failure.test")
        tab.error =
          "This page could not load. Check its address and connection, then retry.";
    }, 1500);
  }
  return {
    context: async () => copy(),
    viewport: async () => {},
    command: async (generation, action) => {
      if (generation !== state.generation)
        throw new Error("The selected workspace changed.");
      if (action.type === "new") add(action.url);
      else if (action.type === "clear") {
        state.tabs = [];
        state.selectedTabId = null;
      } else {
        const tab = state.tabs.find((item) => item.id === action.tabId);
        if (!tab) throw new Error("This tab has closed.");
        switch (action.type) {
          case "navigate":
            navigate(tab, action.url);
            break;
          case "select":
            state.selectedTabId = tab.id;
            break;
          case "close":
            state.tabs = state.tabs.filter((item) => item !== tab);
            if (state.selectedTabId === tab.id)
              state.selectedTabId = state.tabs[0]?.id ?? null;
            break;
          case "stop":
            tab.loading = false;
            break;
          case "reload":
            navigate(tab, tab.url);
            break;
          case "back":
            tab.canGoBack = false;
            tab.canGoForward = true;
            break;
          case "forward":
            tab.canGoBack = true;
            tab.canGoForward = false;
            break;
          case "open_popup":
            if (tab.popupUrl) add(tab.popupUrl);
            tab.popupUrl = null;
            tab.notice = null;
            break;
          case "dismiss_notice":
            tab.notice = null;
            tab.popupUrl = null;
            break;
          case "open_external":
            break;
        }
      }
      return copy();
    },
  };
}
