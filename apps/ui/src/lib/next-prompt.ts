/** Bounded local extraction; parity with colossus-presentation::local_next_prompt. */
export type NextPromptMode = "off" | "local" | "model";
export interface NextPromptSource {
  key: string;
  output: string;
}
export interface NextPromptPrediction {
  source_run_id: string;
  text: string | null;
  usage: { total_tokens: number } | null;
}
export function normalizeNextPrompt(input: string): string | null {
  const value = input
    .trim()
    .replace(/[?.!]+$/u, "")
    .trim();
  if (
    !value ||
    [...value].length > 240 ||
    /[\p{Cc}\u202a-\u202e\u2066-\u2069]/u.test(value) ||
    /^[\/`!]/u.test(value)
  )
    return null;
  const [first, ...rest] = [...value];
  return first ? first.toLocaleUpperCase() + rest.join("") : null;
}
export function localNextPrompt(output: string): string | null {
  let code = false,
    next = false;
  for (const raw of [...output.slice(0, 24000)]
    .slice(0, 12000)
    .join("")
    .split(/\r?\n/u)) {
    const line = raw.trim();
    if (/^(?:```|~~~)/u.test(line)) {
      code = !code;
      continue;
    }
    if (code || !line) continue;
    const heading = line
      .replace(/^#+/u, "")
      .trim()
      .replace(/^\*+|\*+$/gu, "")
      .replace(/:$/u, "")
      .toLowerCase();
    if (["next steps", "next step", "suggested next steps"].includes(heading)) {
      next = true;
      continue;
    }
    if (line.startsWith("#")) next = false;
    const offer =
      /^(?:would you like me to |do you want me to |i can also |i can )(.+)$/iu.exec(
        line,
      );
    if (offer?.[1]) return normalizeNextPrompt(offer[1]);
    const action = next
      ? /^(?:[-*] (?:\[ \] )?|\d+\. )(.+)$/u.exec(line)
      : null;
    if (action?.[1]) return normalizeNextPrompt(action[1]);
  }
  return null;
}
