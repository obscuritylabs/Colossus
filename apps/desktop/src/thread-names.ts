const STORAGE_KEY = "colossus.thread-names:v1";
const MAX_STORED_BYTES = 65_536;
const MAX_NAMED_WORKSPACES = 64;
export const MAX_NAMED_THREADS_PER_WORKSPACE = 128;
export const MAX_THREAD_NAME_CHARACTERS = 80;

export interface StoredThreadName {
  sessionId: string;
  name: string;
}

export interface StoredWorkspaceThreadNames {
  spaceId: string;
  threads: readonly StoredThreadName[];
}

const UNSAFE_DISPLAY_CHARACTERS =
  /[\u0000-\u001f\u007f-\u009f\u200b-\u200f\u202a-\u202e\u2060-\u206f]/u;

function isBoundedIdentifier(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.length <= 256 &&
    !UNSAFE_DISPLAY_CHARACTERS.test(value)
  );
}

export function normalizeThreadName(value: unknown): string | null {
  if (typeof value !== "string" || UNSAFE_DISPLAY_CHARACTERS.test(value)) {
    return null;
  }
  const normalized = value.normalize("NFC").trim();
  return normalized.length > 0 &&
    [...normalized].length <= MAX_THREAD_NAME_CHARACTERS &&
    !UNSAFE_DISPLAY_CHARACTERS.test(normalized)
    ? normalized
    : null;
}

export function parseStoredThreadNames(
  serialized: string | null,
): readonly StoredWorkspaceThreadNames[] {
  if (serialized === null || serialized.length > MAX_STORED_BYTES) {
    return [];
  }
  try {
    const parsed: unknown = JSON.parse(serialized);
    if (!Array.isArray(parsed)) {
      return [];
    }
    const workspaces: StoredWorkspaceThreadNames[] = [];
    const seenWorkspaces = new Set<string>();
    for (const candidate of parsed.slice(0, MAX_NAMED_WORKSPACES)) {
      if (
        typeof candidate !== "object" ||
        candidate === null ||
        !("spaceId" in candidate) ||
        !("threads" in candidate) ||
        !isBoundedIdentifier(candidate.spaceId) ||
        !Array.isArray(candidate.threads) ||
        seenWorkspaces.has(candidate.spaceId)
      ) {
        continue;
      }
      const threads: StoredThreadName[] = [];
      const seenSessions = new Set<string>();
      for (const thread of candidate.threads) {
        if (
          threads.length >= MAX_NAMED_THREADS_PER_WORKSPACE ||
          typeof thread !== "object" ||
          thread === null ||
          !("sessionId" in thread) ||
          !("name" in thread) ||
          !isBoundedIdentifier(thread.sessionId) ||
          seenSessions.has(thread.sessionId)
        ) {
          continue;
        }
        const name = normalizeThreadName(thread.name);
        if (name === null) {
          continue;
        }
        seenSessions.add(thread.sessionId);
        threads.push({ sessionId: thread.sessionId, name });
      }
      if (threads.length === 0) {
        continue;
      }
      seenWorkspaces.add(candidate.spaceId);
      workspaces.push({ spaceId: candidate.spaceId, threads });
    }
    return workspaces;
  } catch {
    return [];
  }
}

export function threadNameForWorkspace(
  stored: readonly StoredWorkspaceThreadNames[],
  spaceId: string | null,
  sessionId: string,
): string | null {
  if (spaceId === null) {
    return null;
  }
  return (
    stored
      .find((entry) => entry.spaceId === spaceId)
      ?.threads.find((entry) => entry.sessionId === sessionId)?.name ?? null
  );
}

export function setThreadName(
  stored: readonly StoredWorkspaceThreadNames[],
  spaceId: string,
  sessionId: string,
  value: string,
): readonly StoredWorkspaceThreadNames[] {
  const name = normalizeThreadName(value);
  if (
    !isBoundedIdentifier(spaceId) ||
    !isBoundedIdentifier(sessionId) ||
    name === null
  ) {
    return stored;
  }
  const current =
    stored.find((entry) => entry.spaceId === spaceId)?.threads ?? [];
  const threads = [
    { sessionId, name },
    ...current.filter((entry) => entry.sessionId !== sessionId),
  ].slice(0, MAX_NAMED_THREADS_PER_WORKSPACE);
  const otherWorkspaces = stored
    .filter((entry) => entry.spaceId !== spaceId)
    .slice(0, MAX_NAMED_WORKSPACES - 1);
  return [{ spaceId, threads }, ...otherWorkspaces];
}

export function readStoredThreadNames(): readonly StoredWorkspaceThreadNames[] {
  if (typeof window === "undefined") {
    return [];
  }
  try {
    return parseStoredThreadNames(window.localStorage.getItem(STORAGE_KEY));
  } catch {
    return [];
  }
}

export function storeThreadNames(
  stored: readonly StoredWorkspaceThreadNames[],
): void {
  try {
    const serialized = JSON.stringify(stored);
    if (serialized.length <= MAX_STORED_BYTES) {
      window.localStorage.setItem(STORAGE_KEY, serialized);
    }
  } catch {
    // Renaming remains available for this app session when storage is unavailable.
  }
}
