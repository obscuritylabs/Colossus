const MAX_LABEL_CHARACTERS = 320;
const LEGACY_SUMMARY =
  /^(?:validated tool call requested|tool (?:execution )?(?:requested|started|completed|cancelled|failed|was cancelled)(?: before start)?(?: at turn \d+)?)[.!]?$/iu;

function singleLine(value: string | null | undefined, limit: number): string {
  const clean = (value ?? "")
    .replace(/[\p{Cc}\p{Cf}]/gu, " ")
    .replace(/\s+/gu, " ")
    .trim();
  const characters = Array.from(clean);
  return characters.length > limit
    ? `${characters.slice(0, limit - 1).join("")}…`
    : clean;
}

/** Display the runtime's released summary without interpreting tool input or output. */
export function toolActivityLabel(
  summary: string | null | undefined,
  toolName: string | null | undefined,
): string {
  const label = singleLine(summary, MAX_LABEL_CHARACTERS);
  if (label && !LEGACY_SUMMARY.test(label)) return label;
  const prefix = "Tool activity · ";
  const identity = singleLine(toolName, MAX_LABEL_CHARACTERS - prefix.length);
  return identity ? `${prefix}${identity}` : "Tool activity";
}
