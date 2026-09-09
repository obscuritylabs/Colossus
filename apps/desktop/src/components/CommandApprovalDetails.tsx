import { useId, useState } from "react";

import type { CommandApprovalContext } from "../types";
import "./command-approval.css";

export function CommandApprovalDetails({
  context,
  initiallyExpanded = false,
}: {
  context: CommandApprovalContext;
  initiallyExpanded?: boolean;
}) {
  const [expanded, setExpanded] = useState(initiallyExpanded);
  const detailsId = useId();
  const argv = [context.executable, ...context.arguments];
  const preview = JSON.stringify(argv);
  return (
    <section className="command-approval-details" aria-label="Prepared command">
      <p className="eyebrow">Reason — agent-provided</p>
      <p className="command-approval-reason">{context.justification}</p>
      <dl className="approval-details">
        <div>
          <dt>Working directory</dt>
          <dd>{context.workingDirectory}</dd>
        </div>
      </dl>
      {context.redacted ? (
        <p role="note">
          Credential-bearing text is redacted. Execution input is unchanged.
        </p>
      ) : null}
      {!expanded ? (
        <pre className="command-approval-preview">
          <code>
            {preview.length > 240
              ? `${preview.slice(0, 240)} … (preview only)`
              : preview}
          </code>
        </pre>
      ) : null}
      <button
        type="button"
        className="button secondary"
        aria-expanded={expanded}
        aria-controls={detailsId}
        onClick={() => setExpanded(!expanded)}
      >
        {expanded ? "Hide full command" : "Show full command"}
      </button>
      {expanded ? (
        <div
          id={detailsId}
          className="command-approval-scroll"
          role="region"
          aria-label="Full prepared argument vector, executable first"
          tabIndex={0}
        >
          <p>
            JSON display strings; backslashes and control characters are visibly
            escaped.
          </p>
          <pre>
            <code>{JSON.stringify(argv, null, 2)}</code>
          </pre>
        </div>
      ) : null}
    </section>
  );
}
