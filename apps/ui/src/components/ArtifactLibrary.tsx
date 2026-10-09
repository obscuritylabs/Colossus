import { IconArchive, IconFileText } from "@tabler/icons-react";
import type { ReactNode } from "react";
import { WorkspaceSurfaceHeader } from "./WorkspaceSurfaceHeader";
export interface LibraryArtifact {
  key: string;
  fileName: string;
  typeLabel: string;
  sizeLabel: string;
  purposeLabel: string;
  stateLabel: string;
  canOpen: boolean;
}
export function ArtifactLibrary({
  artifacts,
  coverage,
  actions,
}: {
  artifacts: readonly LibraryArtifact[];
  coverage?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <>
      <WorkspaceSurfaceHeader
        eyebrow="Library / Released artifacts"
        title="Artifact library"
        description="Safe metadata for files and outputs released through run messages."
        actions={actions}
      />
      <div className="shared-artifact-library">
        <header>
          <h3>{artifacts.length} released artifacts</h3>
          {coverage}
        </header>
        <div className="artifact-library-list">
          {artifacts.map((artifact) => (
            <article key={artifact.key}>
              <span className="library-file-icon" aria-hidden="true">
                <IconFileText size={20} stroke={1.6} />
              </span>
              <div>
                <strong>{artifact.fileName}</strong>
                <span>
                  {artifact.typeLabel} · {artifact.sizeLabel} ·{" "}
                  {artifact.purposeLabel}
                </span>
              </div>
              <span
                className={
                  "status-chip tone-" +
                  (artifact.canOpen ? "success" : "attention")
                }
              >
                {artifact.stateLabel}
              </span>
            </article>
          ))}
          {artifacts.length === 0 ? (
            <div className="honest-empty compact-empty">
              <IconArchive size={25} stroke={1.4} aria-hidden="true" />
              <div>
                <strong>No released artifacts yet</strong>
                <p>
                  Run outputs will appear after they cross the public release
                  boundary.
                </p>
              </div>
            </div>
          ) : null}
        </div>
      </div>
    </>
  );
}
