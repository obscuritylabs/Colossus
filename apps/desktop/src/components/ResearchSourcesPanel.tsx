import {
  IconBook2,
  IconExternalLink,
  IconFileText,
  IconFlask,
} from "@tabler/icons-react";

export interface ResearchSource {
  label: string;
  title: string;
  uri: string;
}

interface ResearchSourcesPanelProps {
  output: string;
  running: boolean;
  onOpenWorkspaceFile: (path: string) => void;
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

export function ResearchSourcesPanel({
  output,
  running,
  onOpenWorkspaceFile,
}: ResearchSourcesPanelProps) {
  const sources = researchSources(output);

  return (
    <section className="research-sources-panel" aria-label="Research sources">
      <header>
        <span className="eyebrow">Research evidence</span>
        <h2>Sources</h2>
        <p>
          Released citations from this Research report. Raw tool traffic remains
          outside the renderer.
        </p>
      </header>
      {sources.length > 0 ? (
        <ol className="research-source-list">
          {sources.map((source) => {
            const workspacePath = workspaceSourcePath(source.uri);
            return (
              <li key={`${source.label}:${source.uri}`}>
                <span className="research-source-label">{source.label}</span>
                <div>
                  <strong>{source.title}</strong>
                  {isWebUri(source.uri) ? (
                    <a href={source.uri} target="_blank" rel="noreferrer">
                      {source.uri}
                      <IconExternalLink
                        size={14}
                        stroke={1.7}
                        aria-hidden="true"
                      />
                    </a>
                  ) : workspacePath !== null ? (
                    <button
                      className="research-source-file"
                      type="button"
                      onClick={() => onOpenWorkspaceFile(workspacePath)}
                    >
                      <IconFileText size={14} stroke={1.7} aria-hidden="true" />
                      <span>{workspacePath}</span>
                      <span>Open file</span>
                    </button>
                  ) : (
                    <code>{source.uri}</code>
                  )}
                </div>
              </li>
            );
          })}
        </ol>
      ) : (
        <div className="research-sources-empty">
          {running ? (
            <IconFlask size={24} stroke={1.6} aria-hidden="true" />
          ) : (
            <IconBook2 size={24} stroke={1.6} aria-hidden="true" />
          )}
          <strong>
            {running ? "Gathering evidence…" : "No released sources"}
          </strong>
          <span>
            {running
              ? "Sources appear here after cited synthesis completes."
              : "The report did not include a released Sources section."}
          </span>
        </div>
      )}
    </section>
  );
}
