import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { CommandApprovalDetails } from "./components/CommandApprovalDetails";
import type { CommandApprovalContext } from "./types";

interface Review {
  reviewId: string;
  target: string;
  commandContext: CommandApprovalContext | null;
  action: string;
  resource: string;
  reason: string;
  canRemember: boolean;
}
type Choice = "deny" | "allow_once" | "always_allow";

export default function CommandReviewWindow() {
  const [review, setReview] = useState<Review | null>(null);
  const [error, setError] = useState("");
  const [submitting, setSubmitting] = useState(false);
  useEffect(() => {
    let active = true;
    void invoke<Review>("command_review_context")
      .then((value) => {
        if (active) setReview(value);
      })
      .catch(() => {
        if (active)
          setError(
            "This approval is no longer available. Close this window and refresh the run.",
          );
      });
    return () => {
      active = false;
    };
  }, []);
  async function respond(decision: Choice) {
    if (!review || submitting) return;
    setSubmitting(true);
    try {
      await invoke("finish_command_review", {
        reviewId: review.reviewId,
        decision,
      });
    } catch {
      setError(
        "This approval changed or expired. Close this window and refresh the run.",
      );
    }
  }
  return (
    <main className="command-review-window">
      <p className="eyebrow">Permission required</p>
      <h1>{review?.commandContext ? "Review command" : "Review action"}</h1>
      {error ? (
        <p role="alert">{error}</p>
      ) : review ? (
        <>
          <p className="command-review-target">{review.target}</p>
          {review.commandContext ? (
            <CommandApprovalDetails
              context={review.commandContext}
              initiallyExpanded
            />
          ) : (
            <dl className="approval-details">
              <div>
                <dt>Action</dt>
                <dd>{review.action}</dd>
              </div>
              <div>
                <dt>Resource</dt>
                <dd>{review.resource}</dd>
              </div>
              <div>
                <dt>Reason</dt>
                <dd>{review.reason}</dd>
              </div>
            </dl>
          )}
          <p className="command-review-scope" id="approval-scope">
            {review.canRemember
              ? "Always allow remembers this exact command and working directory in this workspace. Manage remembered commands in Settings → Workspace → Access."
              : "This action can be approved once. Remembered approvals are available for commands with complete, unredacted details in a local workspace."}
          </p>
          <div className="command-review-actions">
            <button
              className="button secondary"
              type="button"
              disabled={submitting}
              onClick={() => void respond("deny")}
            >
              Deny
            </button>
            {review.canRemember ? (
              <button
                className="button secondary"
                type="button"
                aria-describedby="approval-scope"
                disabled={submitting}
                onClick={() => void respond("always_allow")}
              >
                Always allow
              </button>
            ) : null}
            <button
              className="button primary"
              type="button"
              disabled={submitting}
              onClick={() => void respond("allow_once")}
            >
              {submitting ? "Applying decision…" : "Allow once"}
            </button>
          </div>
        </>
      ) : (
        <p role="status">Loading approval details…</p>
      )}
    </main>
  );
}
