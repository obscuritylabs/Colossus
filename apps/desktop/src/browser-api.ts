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
  sessionId?: string;
  conversationId?: string | null;
  control?:
    "human" | "agent" | "paused" | "completed" | "expired" | "unavailable";
  /** Desktop retains a read-only native frame after agent control ends. */
  completedSnapshot?: boolean;
}

export interface BrowserEngine {
  kind:
    | "embedded_chromium"
    | "contained_chromium"
    | "owned_chromium"
    | "owned_chromium_functional_preview"
    | "webview2"
    | "webkit"
    | "unavailable";
  preview: boolean;
  ready: boolean;
  message: string | null;
  agentControlAvailable: boolean;
}

export interface BrowserCertificateStatus {
  scope: "operating_system_user" | "unsupported";
  caImportAvailable: boolean;
  pfxImportAvailable: boolean;
  clientIdentityReviewAvailable?: boolean;
  clientIdentitySelectionReady: boolean;
  acceptancePending: boolean;
  message: string;
  fingerprintsSha256: string[];
}

export type BrowserCertificateAction =
  "status" | "import_ca" | "import_client_identity" | "review_client_identity";

export interface HumanBrowserPreferences {
  rememberSignIns: boolean;
  userAgent: string;
}

export interface BrowserPreferencesStatus {
  generation: number;
  available: boolean;
  preferences: HumanBrowserPreferences;
  message: string;
}

export type BrowserPreferencesAction =
  | { type: "status" | "clear_data" | "reset" }
  | { type: "save"; preferences: HumanBrowserPreferences };

export interface BrowserDiagnostics {
  origin: string | null;
  httpStatus: number | null;
  redirects: number;
  nativeError: number | null;
  clientCertificateRequested: boolean;
  tlsFailed: boolean;
  persistent: boolean;
  customUserAgent: boolean;
  systemEngine: boolean;
}

export interface BrowserSnapshot {
  available: boolean;
  generation: number;
  tabs: BrowserTab[];
  selectedTabId: string | null;
  engine?: BrowserEngine;
  diagnostics?: Record<string, BrowserDiagnostics>;
}

export type BrowserAction =
  | { type: "new"; url: string; conversationId?: string }
  | { type: "handoff"; tabId: string; runId: string }
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
  context(runId?: string, conversationId?: string): Promise<BrowserSnapshot>;
  command(generation: number, action: BrowserAction): Promise<BrowserSnapshot>;
  viewport(request: BrowserViewport): Promise<void>;
  preferences?(
    generation: number,
    action: BrowserPreferencesAction,
  ): Promise<BrowserPreferencesStatus>;
  certificates?(
    generation: number,
    action: BrowserCertificateAction,
    appearance: {
      colorScheme: "system" | "dark" | "light";
      textSize: "compact" | "comfortable" | "large";
    },
    tabId?: string,
  ): Promise<BrowserCertificateStatus>;
}

export const nativeBrowserApi: BrowserApi = {
  context: (runId, conversationId) =>
    invoke("browser_context", { runId, conversationId }),
  command: (generation, action) =>
    invoke("browser_command", { request: { generation, action } }),
  viewport: (request) => invoke("browser_viewport", { request }),
  preferences: (generation, action) =>
    invoke("browser_preferences", { request: { generation, action } }),
  certificates: (generation, action, appearance, tabId) =>
    invoke("browser_certificates", {
      request: { generation, action, appearance, tabId },
    }),
};

export function browserErrorMessage(error: unknown): string {
  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof error.message === "string"
  )
    return error.message;
  return "The browser action failed. Please try again.";
}
