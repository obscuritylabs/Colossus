import {
  IconBook2,
  IconExternalLink,
  IconFileText,
  IconFlask,
} from "@tabler/icons-react";
import { useContext } from "react";
import { BrowserLink, BrowserLinkContext } from "./browser/BrowserLink";

export type { ResearchSource } from "@colossus/ui/session/sources";
import {
  researchSources,
  isWebUri,
  workspaceSourcePath,
} from "@colossus/ui/session/sources";
export {
  researchSources,
  isWebUri,
  workspaceSourcePath,
} from "@colossus/ui/session/sources";
interface ResearchSourcesPanelProps {
  output: string;
  running: boolean;
  onOpenWorkspaceFile: (path: string) => void;
}

export function ResearchSourcesPanel({
  output,
  running,
  onOpenWorkspaceFile,
}: ResearchSourcesPanelProps) {
  const sources = researchSources(output);
  const openBrowser = useContext(BrowserLinkContext);

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
                    openBrowser ? (
                      <BrowserLink href={source.uri}>{source.uri}</BrowserLink>
                    ) : (
                      <a href={source.uri} target="_blank" rel="noreferrer">
                        {source.uri}
                        <IconExternalLink
                          size={14}
                          stroke={1.7}
                          aria-hidden="true"
                        />
                      </a>
                    )
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
