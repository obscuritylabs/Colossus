import { isTerminalStatus } from "./types";
import type { CreateRunRequest, Run } from "./types";
import {
  MAX_THREAD_NAME_CHARACTERS,
  normalizeThreadName,
} from "./thread-names";

const STORAGE_KEY = "colossus.thread-forks:v1";
const DRAFT_PREFIX = "desktop-fork-draft-";
const MAX_DRAFTS = 64;

/** Local metadata only. The runtime resolves context when the operator sends. */
export interface ThreadForkDraft {
  id: string;
  spaceId: string;
  sourceRunId: string;
  sourceSessionId: string;
  sourceCreatedAt: string;
  role: string;
  title: string;
  createdAt: string;
  materializedSessionId?: string;
}

export function isThreadForkDraft(run: Run): boolean {
  return run.runId.startsWith(DRAFT_PREFIX);
}

export function canForkThread(run: Run): boolean {
  return (
    !isThreadForkDraft(run) && !run.archived && isTerminalStatus(run.status)
  );
}

export function defaultForkTitle(title: string): string {
  return `${[...title].slice(0, MAX_THREAD_NAME_CHARACTERS - 7).join("")} (fork)`;
}

export function createThreadForkDraft(
  source: Run,
  spaceId: string,
  title: string,
): ThreadForkDraft {
  const name = normalizeThreadName(title);
  if (!canForkThread(source) || name === null)
    throw new Error("Enter a valid title for this fork.");
  return {
    id: `${DRAFT_PREFIX}${crypto.randomUUID()}`,
    spaceId,
    sourceRunId: source.runId,
    sourceSessionId: source.sessionId,
    sourceCreatedAt: source.createdAt,
    role: source.role,
    title: name,
    createdAt: new Date().toISOString(),
  };
}

/** Sidebar presentation only; this identifier is never sent as a runtime session. */
export function threadForkDraftRun(draft: ThreadForkDraft): Run {
  return {
    runId: draft.id,
    sessionId: draft.id,
    title: draft.title,
    role: draft.role,
    mode: "execute",
    status: "completed",
    createdAt: draft.createdAt,
    updatedAt: draft.createdAt,
    startedAt: null,
    finishedAt: null,
    lastSequence: 0,
    pendingInteractionCount: 0,
    terminal: null,
    archived: false,
    etag: draft.id,
  };
}

/** Only a source ID crosses the native boundary; never a renderer transcript. */
export function threadForkBranch(
  draft: ThreadForkDraft,
): NonNullable<CreateRunRequest["branch"]> {
  return { sourceRunId: draft.sourceRunId, kind: "thread" };
}

export function parseThreadForkDrafts(
  serialized: string | null,
): readonly ThreadForkDraft[] {
  if (serialized === null || serialized.length > 65_536) return [];
  try {
    const value: unknown = JSON.parse(serialized);
    if (!Array.isArray(value)) return [];
    const seen = new Set<string>();
    return value.slice(0, MAX_DRAFTS).flatMap((entry: unknown) => {
      if (entry === null || typeof entry !== "object") return [];
      const item = entry as Record<string, unknown>;
      const fields = [
        "id",
        "spaceId",
        "sourceRunId",
        "sourceSessionId",
        "role",
        "sourceCreatedAt",
        "createdAt",
      ] as const;
      if (
        fields.some(
          (field) =>
            typeof item[field] !== "string" ||
            !item[field] ||
            item[field].length > 256 ||
            /[\u0000-\u001f\u007f]/u.test(item[field]),
        )
      )
        return [];
      const title = normalizeThreadName(item.title);
      const id = item.id as string;
      const sessionId = item.materializedSessionId;
      if (
        title === null ||
        !id.startsWith(DRAFT_PREFIX) ||
        seen.has(id) ||
        !Number.isFinite(Date.parse(item.createdAt as string)) ||
        !Number.isFinite(Date.parse(item.sourceCreatedAt as string)) ||
        (sessionId !== undefined &&
          (typeof sessionId !== "string" ||
            !sessionId ||
            sessionId.length > 256 ||
            /[\u0000-\u001f\u007f]/u.test(sessionId)))
      )
        return [];
      seen.add(id);
      return [
        {
          id,
          spaceId: item.spaceId as string,
          sourceRunId: item.sourceRunId as string,
          sourceSessionId: item.sourceSessionId as string,
          role: item.role as string,
          sourceCreatedAt: item.sourceCreatedAt as string,
          createdAt: item.createdAt as string,
          title,
          ...(sessionId === undefined
            ? {}
            : { materializedSessionId: sessionId as string }),
        },
      ];
    });
  } catch {
    return [];
  }
}

export function readThreadForkDrafts(): readonly ThreadForkDraft[] {
  try {
    return parseThreadForkDrafts(window.localStorage.getItem(STORAGE_KEY));
  } catch {
    return [];
  }
}

export function storeThreadForkDrafts(
  drafts: readonly ThreadForkDraft[],
): void {
  try {
    window.localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify(drafts.slice(0, MAX_DRAFTS)),
    );
  } catch {
    /* The draft remains usable in this app session when storage is unavailable. */
  }
}
