function identityLabel(value: unknown): string {
  if (typeof value !== "string") return "";
  const clean = value
    .replace(/[\p{Cc}\p{Cf}]/gu, " ")
    .replace(/\s+/gu, " ")
    .trim();
  const characters = Array.from(clean);
  return characters.length > 72
    ? `${characters.slice(0, 71).join("")}…`
    : clean;
}

/** Name the configured MCP server and tool from released input or result metadata. */
export function mcpCallTarget(
  releasedMetadata: string | null | undefined,
): string | null {
  if (!releasedMetadata) return null;
  try {
    const input: unknown = JSON.parse(releasedMetadata);
    if (typeof input !== "object" || input === null || Array.isArray(input))
      return null;
    const fields = input as Record<string, unknown>;
    const server = identityLabel(fields.server);
    if (!server) return null;
    const tool = identityLabel(fields.tool);
    return tool ? `${server} · ${tool}` : server;
  } catch {
    return null;
  }
}
