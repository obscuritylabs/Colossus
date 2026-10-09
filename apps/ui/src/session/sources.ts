export interface ResearchSource {
  label: string;
  title: string;
  uri: string;
}

const SOURCE_LINE = /^- \[([^\]]+)]\s+(.+?)\s+[—-]\s+(\S.*)$/;

export function researchSources(output: string): readonly ResearchSource[] {
  const section = output.split(/^## Sources\s*$/m)[1] ?? "";
  return section
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => SOURCE_LINE.exec(line))
    .filter((match): match is RegExpExecArray => match !== null)
    .slice(0, 100)
    .map((match) => ({
      label: match[1]?.slice(0, 32) ?? "Source",
      title: match[2]?.slice(0, 512) ?? "Released source",
      uri: match[3]?.slice(0, 2_048) ?? "",
    }));
}

export function isWebUri(uri: string): boolean {
  try {
    const parsed = new URL(uri);
    return parsed.protocol === "https:" || parsed.protocol === "http:";
  } catch {
    return false;
  }
}

export function workspaceSourcePath(uri: string): string | null {
  const candidate = uri.startsWith("repo://") ? uri.slice(7) : uri;
  if (
    candidate === "" ||
    candidate.startsWith("/") ||
    candidate.includes("\\") ||
    /^[a-z][a-z0-9+.-]*:/i.test(candidate)
  ) {
    return null;
  }
  const path = candidate.split(/[?#]/, 1)[0] ?? "";
  const components = path.split("/");
  return path.length <= 4_096 &&
    components.every(
      (component) =>
        component !== "" && component !== "." && component !== "..",
    )
    ? path
    : null;
}
