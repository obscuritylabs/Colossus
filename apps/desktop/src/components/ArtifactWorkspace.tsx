import {
  IconCheck,
  IconCode,
  IconFileText,
  IconFolderOpen,
} from "@tabler/icons-react";
import { useEffect, useState } from "react";
import type { KeyboardEvent } from "react";

export interface ArtifactPreviewLine {
  number: number;
  kind: "context" | "addition" | "deletion";
  text: string;
}

export interface ArtifactViewItem {
  id: string;
  fileName: string;
  mediaType: string;
  sizeLabel: string;
  stateLabel: string;
  createdLabel: string;
  previewLines?: readonly ArtifactPreviewLine[];
  previewStatus?: "idle" | "loading" | "error";
  previewError?: string;
}

interface ArtifactWorkspaceProps {
  artifacts: readonly ArtifactViewItem[];
  selectedId?: string;
  onSelect?: (artifactId: string) => void;
}

function isCodeArtifact(item: ArtifactViewItem): boolean {
  return (
    item.mediaType.includes("text/") ||
    item.mediaType.includes("json") ||
    /\.(rs|ts|tsx|js|jsx|go|py|json|ya?ml|toml|md)$/i.test(item.fileName)
  );
}

export function ArtifactWorkspace({
  artifacts,
  selectedId,
  onSelect,
}: ArtifactWorkspaceProps) {
  const [localSelection, setLocalSelection] = useState(
    selectedId ?? artifacts[0]?.id ?? "",
  );

  useEffect(() => {
    if (selectedId !== undefined) {
      setLocalSelection(selectedId);
      return;
    }
    if (!artifacts.some((item) => item.id === localSelection)) {
      setLocalSelection(artifacts[0]?.id ?? "");
    }
  }, [artifacts, localSelection, selectedId]);

  const activeId = selectedId ?? localSelection;
  const active =
    artifacts.find((artifact) => artifact.id === activeId) ?? artifacts[0];

  function select(artifactId: string) {
    setLocalSelection(artifactId);
    onSelect?.(artifactId);
  }

  function moveTabFocus(event: KeyboardEvent<HTMLButtonElement>) {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
      return;
    }
    const tabs = Array.from(
      event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>(
        '[role="tab"]',
      ) ?? [],
    );
    const currentIndex = tabs.indexOf(event.currentTarget);
    if (currentIndex < 0 || tabs.length === 0) {
      return;
    }
    event.preventDefault();
    const nextIndex =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? tabs.length - 1
          : (currentIndex +
              (event.key === "ArrowRight" ? 1 : -1) +
              tabs.length) %
            tabs.length;
    const next = tabs[nextIndex];
    if (next !== undefined) {
      next.focus();
      select(next.dataset.artifactId ?? "");
    }
  }

  return (
    <aside className="artifact-workspace" aria-label="Work artifacts">
      {artifacts.length === 0 || active === undefined ? (
        <div className="artifact-empty">
          <span className="empty-icon" aria-hidden="true">
            <IconFolderOpen size={24} stroke={1.5} />
          </span>
          <div>
            <strong>No released artifacts yet</strong>
            <p>
              Files and outputs released by this work will appear here without
              exposing private runtime paths.
            </p>
          </div>
        </div>
      ) : (
        <>
          <div className="artifact-tabs" role="tablist" aria-label="Artifacts">
            {artifacts.map((artifact) => {
              const selected = artifact.id === active.id;
              const Icon = isCodeArtifact(artifact) ? IconCode : IconFileText;
              return (
                <button
                  key={artifact.id}
                  type="button"
                  role="tab"
                  aria-selected={selected}
                  aria-controls="artifact-preview"
                  className="artifact-tab"
                  data-artifact-id={artifact.id}
                  tabIndex={selected ? 0 : -1}
                  onClick={() => select(artifact.id)}
                  onKeyDown={moveTabFocus}
                >
                  <Icon size={15} stroke={1.6} aria-hidden="true" />
                  <span>{artifact.fileName}</span>
                </button>
              );
            })}
          </div>
          <div className="artifact-toolbar">
            <span className="artifact-saved">
              <IconCheck size={15} stroke={2} aria-hidden="true" />
              {active.stateLabel}
            </span>
            <span>
              {active.sizeLabel} · {active.createdLabel}
            </span>
          </div>
          <div
            className="artifact-preview"
            id="artifact-preview"
            role="tabpanel"
            aria-label={active.fileName}
            tabIndex={0}
          >
            {active.previewLines !== undefined ? (
              <pre aria-label={`${active.fileName} preview`}>
                <code>
                  {active.previewLines.map((line) => (
                    <span
                      className={`code-line code-line-${line.kind}`}
                      key={`${line.number}-${line.text}`}
                    >
                      <span className="code-number" aria-hidden="true">
                        {line.number}
                      </span>
                      <span className="code-sign" aria-hidden="true">
                        {line.kind === "addition"
                          ? "+"
                          : line.kind === "deletion"
                            ? "−"
                            : " "}
                      </span>
                      <span>{line.text}</span>
                    </span>
                  ))}
                </code>
              </pre>
            ) : (
              <div className="artifact-locked-preview">
                <IconFileText size={26} stroke={1.4} aria-hidden="true" />
                <strong>{active.fileName}</strong>
                <p>
                  {active.previewStatus === "loading"
                    ? "Loading the authorized preview…"
                    : active.previewStatus === "error"
                      ? (active.previewError ??
                        "The artifact preview could not be loaded.")
                      : "Select this artifact to load its authorized preview."}
                </p>
              </div>
            )}
          </div>
        </>
      )}
    </aside>
  );
}
