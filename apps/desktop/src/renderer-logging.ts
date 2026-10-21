import { invoke, isTauri } from "@tauri-apps/api/core";

export type RendererFailure = "render" | "script" | "unhandled_promise";

const lastReported = new Map<RendererFailure, number>();

/** Report only a fixed failure category; error objects can contain private text. */
export function reportRendererFailure(failure: RendererFailure): void {
  if (
    !isTauri() ||
    new URLSearchParams(window.location.search).has("surface")
  ) {
    return;
  }
  const now = Date.now();
  const previous = lastReported.get(failure);
  if (previous !== undefined && now - previous < 10_000) return;
  lastReported.set(failure, now);
  void invoke("report_renderer_failure", { failure }).catch(() => {});
}

export function installRendererFailureHandlers(): void {
  window.addEventListener("error", () => reportRendererFailure("script"));
  window.addEventListener("unhandledrejection", () =>
    reportRendererFailure("unhandled_promise"),
  );
}
