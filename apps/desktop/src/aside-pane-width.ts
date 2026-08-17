export const MIN_ASIDE_PANE_WIDTH = 280;
export const MAX_ASIDE_PANE_WIDTH = 760;

const MIN_WORK_THREAD_WIDTH = 390;
const ASIDE_RESIZE_HANDLE_WIDTH = 8;
const DEFAULT_ASIDE_PANE_RATIO = 0.53;
const STORAGE_KEY = "colossus.aside-pane-width:v1";

export function defaultAsidePaneWidth(layoutWidth: number): number {
  return clampAsidePaneWidth(
    layoutWidth * DEFAULT_ASIDE_PANE_RATIO,
    layoutWidth,
  );
}

export function clampAsidePaneWidth(
  width: number,
  layoutWidth: number,
): number {
  const responsiveMaximum = Math.max(
    MIN_ASIDE_PANE_WIDTH,
    Math.min(
      MAX_ASIDE_PANE_WIDTH,
      layoutWidth - MIN_WORK_THREAD_WIDTH - ASIDE_RESIZE_HANDLE_WIDTH,
    ),
  );
  return Math.round(
    Math.min(responsiveMaximum, Math.max(MIN_ASIDE_PANE_WIDTH, width)),
  );
}

export function readStoredAsidePaneWidth(): number | null {
  if (typeof window === "undefined") {
    return null;
  }
  try {
    const stored = Number(window.localStorage.getItem(STORAGE_KEY));
    return Number.isFinite(stored) && stored > 0 ? Math.round(stored) : null;
  } catch {
    return null;
  }
}

export function storeAsidePaneWidth(width: number): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, String(Math.round(width)));
  } catch {
    // The live split remains adjustable when renderer storage is unavailable.
  }
}

export function clearStoredAsidePaneWidth(): void {
  try {
    window.localStorage.removeItem(STORAGE_KEY);
  } catch {
    // Resetting the live split still succeeds without renderer storage.
  }
}
