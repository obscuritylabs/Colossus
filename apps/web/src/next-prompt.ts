import { request } from "./api";
import type { NextPromptPrediction } from "@colossus/ui/lib/next-prompt";

interface PredictionCommand { command_id: string; reply: { kind: string; prediction?: NextPromptPrediction } | null; }

/** Queue once, then observe the same durable command; never retry a generation. */
export async function loadNextPrompt(taskPath: string, sourceRunId: string, signal: AbortSignal): Promise<NextPromptPrediction | null> {
  let { command } = await request<{ command: PredictionCommand }>(`${taskPath}/next-prompt`, {}, signal);
  const deadline = Date.now() + 18_000;
  while (!command.reply && Date.now() < deadline && !signal.aborted) {
    await new Promise<void>((resolve) => {
      const finish = () => { clearTimeout(timer); signal.removeEventListener("abort", finish); resolve(); };
      const timer = setTimeout(finish, 350);
      signal.addEventListener("abort", finish, { once: true });
    });
    if (signal.aborted) return null;
    ({ command } = await request<{ command: PredictionCommand }>(`${taskPath}/commands/${encodeURIComponent(command.command_id)}`, undefined, signal));
  }
  const prediction = command.reply?.kind === "next_prompt_prediction" ? command.reply.prediction : null;
  return prediction?.source_run_id === sourceRunId ? prediction : null;
}
