import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

import { CommandApprovalDetails } from "./components/CommandApprovalDetails";
import type { CommandApprovalContext } from "./types";

interface Review {
  reviewId: string;
  target: string;
  commandContext: CommandApprovalContext;
}

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
            "This command review is no longer available. Close this window and refresh the run.",
          );
      });
    return () => {
      active = false;
    };
  }, []);
  async function respond(approved: boolean) {
    if (!review || submitting) return;
    setSubmitting(true);
    try {
      await invoke("finish_command_review", {
        reviewId: review.reviewId,
        approved,
      });
    } catch {
      setError(
        "This command review changed or expired. Close this window and refresh the run.",
      );
    }
  }
  return (
    <main className="command-review-window">
      <h1>Review command</h1>
      <p>
        The agent's explanation is not an authorization or safety assessment.
      </p>
      {error ? (
        <p role="alert">{error}</p>
      ) : review ? (
        <>
          <p>{review.target}</p>
          <CommandApprovalDetails
            context={review.commandContext}
            initiallyExpanded
          />
          <div className="command-review-actions">
            <button
              className="button secondary"
              type="button"
              disabled={submitting}
              onClick={() => void respond(false)}
            >
              Deny
            </button>
            <button
              className="button primary"
              type="button"
              disabled={submitting}
              onClick={() => void respond(true)}
            >
              {submitting
                ? "Awaiting native confirmation…"
                : "Continue to native confirmation"}
            </button>
          </div>
        </>
      ) : (
        <p role="status">Loading authoritative command details…</p>
      )}
    </main>
  );
}
