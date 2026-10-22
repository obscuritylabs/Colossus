import { MarkdownContent } from "@colossus/ui/conversation";
import { PlanInspection } from "@colossus/ui/session/PlanInspection";
import type { SessionPlanReference } from "@colossus/ui/session/selectors";
import type { ArtifactViewItem } from "@colossus/ui/session/types";
import { ThreadPlanDecision } from "./ThreadPlanDecision";
import type { PlanStrategy } from "./plan-continuation";
import { WebLink } from "./WebLink";

export type ThreadInspection =
  | { kind: "plan"; sourceRunId: string; planId: string; revision: number }
  | { kind: "artifact"; artifact: ArtifactViewItem }
  | { kind: "aside" };

export function inspectionKey(inspection: ThreadInspection): string {
  return inspection.kind === "aside"
    ? "aside"
    : inspection.kind === "artifact"
      ? JSON.stringify(["artifact", inspection.artifact.id])
      : JSON.stringify([
          "plan",
          inspection.sourceRunId,
          inspection.planId,
          inspection.revision,
        ]);
}

export function ThreadPlanInspection({
  plan,
  available,
  goalAvailable,
  busy,
  onRevise,
  onExecute,
}: {
  plan: SessionPlanReference;
  available: boolean;
  goalAvailable: boolean;
  busy: boolean;
  onRevise: (plan: SessionPlanReference) => void;
  onExecute: (plan: SessionPlanReference, strategy: PlanStrategy) => void;
}) {
  return (
    <PlanInspection
      plan={plan}
      document={
        <MarkdownContent
          className="plan-details-markdown"
          content={plan.output}
          linkComponent={WebLink}
        />
      }
      actions={
        <ThreadPlanDecision
          plan={plan}
          available={available}
          goalAvailable={goalAvailable}
          busy={busy}
          onRevise={onRevise}
          onExecute={onExecute}
          inline
        />
      }
    />
  );
}

export function ThreadArtifactInspection({
  artifact,
}: {
  artifact: ArtifactViewItem;
}) {
  return (
    <section
      className="shared-inspector-document"
      aria-label="Artifact metadata"
    >
      <h3>{artifact.fileName}</h3>
      <dl className="configuration-list">
        <div>
          <dt>Media type</dt>
          <dd>{artifact.mediaType}</dd>
        </div>
        <div>
          <dt>Size</dt>
          <dd>{artifact.sizeLabel}</dd>
        </div>
        <div>
          <dt>State</dt>
          <dd>{artifact.stateLabel}</dd>
        </div>
      </dl>
      <p>
        Released metadata is available here. Open local file contents in
        Desktop.
      </p>
    </section>
  );
}
