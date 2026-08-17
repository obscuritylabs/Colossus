export const MIN_WORK_SIDEBAR_WIDTH = 260;
export const MAX_WORK_SIDEBAR_WIDTH = 480;

const COMPACT_WORK_SIDEBAR_WIDTH = 288;
const DEFAULT_WORK_SIDEBAR_WIDTH = 360;
const MIN_PRIMARY_WORKSPACE_WIDTH = 560;
const STORAGE_KEY = "colossus.work-sidebar-width:v1";

export function defaultWorkSidebarWidth(viewportWidth?: number): number {
  const width =
    viewportWidth ??
    (typeof window === "undefined" ? undefined : window.innerWidth);
  return width !== undefined && width <= 1190
    ? COMPACT_WORK_SIDEBAR_WIDTH
    : DEFAULT_WORK_SIDEBAR_WIDTH;
}

export function clampWorkSidebarWidth(
  width: number,
  viewportWidth?: number,
): number {
  const availableWidth =
    viewportWidth ??
    (typeof window === "undefined" ? undefined : window.innerWidth);
  const responsiveMaximum =
    availableWidth === undefined
      ? MAX_WORK_SIDEBAR_WIDTH
      : Math.max(
          MIN_WORK_SIDEBAR_WIDTH,
          Math.min(
            MAX_WORK_SIDEBAR_WIDTH,
            availableWidth - MIN_PRIMARY_WORKSPACE_WIDTH,
          ),
        );
  return Math.round(
    Math.min(responsiveMaximum, Math.max(MIN_WORK_SIDEBAR_WIDTH, width)),
  );
}

export function readStoredWorkSidebarWidth(): number | null {
  if (typeof window === "undefined") {
    return null;
  }
  try {
    const stored = Number(window.localStorage.getItem(STORAGE_KEY));
    return Number.isFinite(stored) && stored > 0
      ? clampWorkSidebarWidth(stored)
      : null;
  } catch {
    return null;
  }
}

export function storeWorkSidebarWidth(width: number): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, String(Math.round(width)));
  } catch {
    // Resizing remains available when renderer storage is unavailable.
  }
}

export function clearStoredWorkSidebarWidth(): void {
  try {
    window.localStorage.removeItem(STORAGE_KEY);
  } catch {
    // Resetting the live layout still succeeds without renderer storage.
  }
}
