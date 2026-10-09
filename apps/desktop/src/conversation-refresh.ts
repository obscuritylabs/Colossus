import type { Run, RunDetails, RunPage } from "./types";

/** Refresh reads only; route/session fences prevent late replies from changing another view. */
export async function refreshConversation({
  sessionId,
  isCurrent,
  list,
  hydrate,
  changed,
  onRecent,
  onRun,
}: {
  sessionId: string | null;
  isCurrent: () => boolean;
  list: (sessionId?: string) => Promise<RunPage>;
  hydrate: (runId: string) => Promise<RunDetails>;
  changed: (run: Run) => boolean;
  onRecent: (page: RunPage) => void;
  onRun: (details: RunDetails) => void;
}) {
  const [recent, session] = await Promise.all([
    list(),
    sessionId ? list(sessionId) : Promise.resolve(null),
  ]);
  if (!isCurrent()) return;
  onRecent(recent);
  if (!session) return;
  for (const run of session.runs.slice(0, 32).reverse()) {
    if (!isCurrent()) return;
    if (run.sessionId !== sessionId || !changed(run)) continue;
    const details = await hydrate(run.runId);
    if (!isCurrent()) return;
    if (details.run.runId !== run.runId || details.run.sessionId !== sessionId)
      continue;
    onRun(details);
  }
}
