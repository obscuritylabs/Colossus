const STORAGE_KEY = "colossus.thread-pins:v1";
const MAX_STORED_BYTES = 65_536;
const MAX_PINNED_SPACES = 64;
export const MAX_PINNED_THREADS_PER_SPACE = 32;

export interface StoredThreadPins {
  spaceId: string;
  sessionIds: readonly string[];
}

function isBoundedIdentifier(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= 256 &&
    !/[\u0000-\u001f\u007f]/u.test(value)
  );
}

export function parseStoredThreadPins(
  serialized: string | null,
): readonly StoredThreadPins[] {
  if (serialized === null || serialized.length > MAX_STORED_BYTES) {
    return [];
  }
  try {
    const parsed: unknown = JSON.parse(serialized);
    if (!Array.isArray(parsed)) {
      return [];
    }
    const spaces: StoredThreadPins[] = [];
    const seenSpaces = new Set<string>();
    for (const candidate of parsed.slice(0, MAX_PINNED_SPACES)) {
      if (
        typeof candidate !== "object" ||
        candidate === null ||
        !("spaceId" in candidate) ||
        !("sessionIds" in candidate) ||
        !isBoundedIdentifier(candidate.spaceId) ||
        !Array.isArray(candidate.sessionIds) ||
        seenSpaces.has(candidate.spaceId)
      ) {
        continue;
      }
      const boundedSessionIds: string[] = candidate.sessionIds.filter(
        (value: unknown): value is string => isBoundedIdentifier(value),
      );
      const sessionIds = boundedSessionIds
        .filter(
          (sessionId, index, values) => values.indexOf(sessionId) === index,
        )
        .slice(0, MAX_PINNED_THREADS_PER_SPACE);
      if (sessionIds.length === 0) {
        continue;
      }
      seenSpaces.add(candidate.spaceId);
      spaces.push({ spaceId: candidate.spaceId, sessionIds });
    }
    return spaces;
  } catch {
    return [];
  }
}

export function pinnedThreadIdsForSpace(
  stored: readonly StoredThreadPins[],
  spaceId: string | null,
): readonly string[] {
  if (spaceId === null) {
    return [];
  }
  return stored.find((entry) => entry.spaceId === spaceId)?.sessionIds ?? [];
}

export function setThreadPinned(
  stored: readonly StoredThreadPins[],
  spaceId: string,
  sessionId: string,
  pinned: boolean,
): readonly StoredThreadPins[] {
  if (!isBoundedIdentifier(spaceId) || !isBoundedIdentifier(sessionId)) {
    return stored;
  }
  const current = pinnedThreadIdsForSpace(stored, spaceId);
  const sessionIds = pinned
    ? [
        sessionId,
        ...current.filter((candidate) => candidate !== sessionId),
      ].slice(0, MAX_PINNED_THREADS_PER_SPACE)
    : current.filter((candidate) => candidate !== sessionId);
  const otherSpaces = stored
    .filter((entry) => entry.spaceId !== spaceId)
    .slice(0, MAX_PINNED_SPACES - 1);
  return sessionIds.length === 0
    ? otherSpaces
    : [{ spaceId, sessionIds }, ...otherSpaces];
}

export function readStoredThreadPins(
  absentFallback: readonly StoredThreadPins[] = [],
): readonly StoredThreadPins[] {
  if (typeof window === "undefined") {
    return absentFallback;
  }
  try {
    const serialized = window.localStorage.getItem(STORAGE_KEY);
    return serialized === null
      ? absentFallback
      : parseStoredThreadPins(serialized);
  } catch {
    return absentFallback;
  }
}

export function storeThreadPins(stored: readonly StoredThreadPins[]): void {
  try {
    const serialized = JSON.stringify(stored);
    if (serialized.length <= MAX_STORED_BYTES) {
      window.localStorage.setItem(STORAGE_KEY, serialized);
    }
  } catch {
    // Pinning remains available for this app session when storage is unavailable.
  }
}
