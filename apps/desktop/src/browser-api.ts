import { invoke } from "@tauri-apps/api/core";

export interface BrowserTab {
  id: string;
  url: string;
  title: string;
  canGoBack: boolean;
  canGoForward: boolean;
  loading: boolean;
  error: string | null;
  notice: string | null;
  popupUrl: string | null;
}

export interface BrowserSnapshot {
  available: boolean;
  generation: number;
  tabs: BrowserTab[];
  selectedTabId: string | null;
}

export type BrowserAction =
  | { type: "new"; url: string }
  | { type: "navigate"; tabId: string; url: string }
  | {
      type:
        | "select"
        | "close"
        | "back"
        | "forward"
        | "reload"
        | "stop"
        | "open_external"
        | "open_popup"
        | "dismiss_notice";
      tabId: string;
    }
  | { type: "clear" };

export interface BrowserViewport {
  generation: number;
  tabId: string | null;
  rect: { x: number; y: number; width: number; height: number } | null;
}

export interface BrowserApi {
  context(): Promise<BrowserSnapshot>;
  command(generation: number, action: BrowserAction): Promise<BrowserSnapshot>;
  viewport(request: BrowserViewport): Promise<void>;
}

export const nativeBrowserApi: BrowserApi = {
  context: () => invoke("browser_context"),
  command: (generation, action) =>
    invoke("browser_command", { request: { generation, action } }),
  viewport: (request) => invoke("browser_viewport", { request }),
};

export function browserErrorMessage(error: unknown): string {
  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof error.message === "string"
  )
    return error.message;
  return "The browser could not complete that action. Please try again.";
}
