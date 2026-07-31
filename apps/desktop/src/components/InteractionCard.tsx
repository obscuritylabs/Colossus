import { useMemo, useState } from "react";

import { CommandFailure } from "../api";
import type { Interaction, InteractionAnswer } from "../types";

interface InteractionCardProps {
  interaction: Interaction;
  onRespond: (
    interaction: Interaction,
    response: InteractionAnswer,
  ) => Promise<void>;
}

function safeErrorMessage(error: unknown): string {
  return error instanceof CommandFailure
    ? error.detail.message
    : "The response could not be sent. Please retry.";
}

function expiryLabel(expiresAt: string): string {
  const date = new Date(expiresAt);
  return Number.isNaN(date.getTime())
    ? "Response window is limited"
    : `Respond by ${date.toLocaleTimeString([], {
        hour: "numeric",
        minute: "2-digit",
      })}`;
}

export function InteractionCard({
  interaction,
  onRespond,
}: InteractionCardProps) {
  const [choiceId, setChoiceId] = useState("");
  const [text, setText] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState("");
  const content = interaction.content;
  const selectedChoice = useMemo(
    () =>
      content.type === "user_prompt"
        ? content.choices.find((choice) => choice.choiceId === choiceId)
        : undefined,
    [choiceId, content],
  );

  async function submit(response: InteractionAnswer) {
    setSubmitting(true);
    setError("");
    try {
      await onRespond(interaction, response);
    } catch (cause: unknown) {
      setError(safeErrorMessage(cause));
    } finally {
      setSubmitting(false);
    }
  }

  if (content.type === "approval") {
    const unavailable = !interaction.respondableByCaller || submitting;
    return (
      <section
        className="interaction-card approval-card"
        aria-labelledby={`${interaction.interactionId}-title`}
      >
        <div className="interaction-heading">
          <div>
            <p className="eyebrow">Approval required</p>
            <h3 id={`${interaction.interactionId}-title`}>{content.action}</h3>
          </div>
          {content.risk !== null && (
            <span className={`risk-badge risk-${content.risk}`}>
              {content.risk} risk
            </span>
          )}
        </div>
        <p>{content.reason}</p>
        <dl className="approval-details">
          <div>
            <dt>Resource</dt>
            <dd>{content.resource}</dd>
          </div>
          <div>
            <dt>Expires</dt>
            <dd>{expiryLabel(interaction.expiresAt)}</dd>
          </div>
        </dl>
        {error !== "" && (
          <p className="inline-error" role="alert">
            {error}
          </p>
        )}
        <div className="interaction-actions">
          <button
            className="button secondary"
            type="button"
            disabled={unavailable}
            onClick={() =>
              void submit({
                type: "approval",
                approved: false,
                requestHash: content.requestHash,
              })
            }
          >
            Deny
          </button>
          <button
            className={`button ${content.risk === "high" ? "danger" : "primary"}`}
            type="button"
            disabled={unavailable}
            onClick={() =>
              void submit({
                type: "approval",
                approved: true,
                requestHash: content.requestHash,
              })
            }
          >
            {submitting ? "Sending…" : "Allow once"}
          </button>
        </div>
      </section>
    );
  }

  const promptContent = content;
  const trimmedText = text.trim();
  const unavailable = !interaction.respondableByCaller || submitting;
  const canSubmit =
    !unavailable &&
    (selectedChoice !== undefined ||
      (promptContent.allowFreeForm && trimmedText.length > 0));
  const responseGuidance = !interaction.respondableByCaller
    ? "Response unavailable"
    : canSubmit
      ? "Ready to send"
      : promptContent.choices.length > 0
        ? "Select one response"
        : "Enter a response";

  function submitPrompt() {
    if (selectedChoice !== undefined) {
      void submit({
        type: "prompt_choice",
        choiceId: selectedChoice.choiceId,
        label: selectedChoice.label,
      });
      return;
    }
    if (promptContent.allowFreeForm && trimmedText.length > 0) {
      void submit({ type: "prompt_text", text: trimmedText });
    }
  }

  return (
    <section
      className="interaction-card prompt-card"
      aria-labelledby={`${interaction.interactionId}-title`}
    >
      <div className="interaction-heading prompt-heading">
        <div>
          <p className="eyebrow">Colossus needs your input</p>
          <h3 id={`${interaction.interactionId}-title`}>
            {promptContent.question}
          </h3>
        </div>
        <span className="expiry">{expiryLabel(interaction.expiresAt)}</span>
      </div>
      <div className="interaction-body">
        {promptContent.choices.length > 0 && (
          <fieldset className="choice-list">
            <legend className="sr-only">Choose a response</legend>
            {promptContent.choices.map((choice) => (
              <label
                className={`choice${choiceId === choice.choiceId ? " is-selected" : ""}`}
                key={choice.choiceId}
              >
                <input
                  type="radio"
                  name={`choice-${interaction.interactionId}`}
                  value={choice.choiceId}
                  checked={choiceId === choice.choiceId}
                  disabled={unavailable}
                  onChange={() => {
                    setChoiceId(choice.choiceId);
                    setText("");
                  }}
                />
                <span>{choice.label}</span>
              </label>
            ))}
          </fieldset>
        )}
        {promptContent.allowFreeForm && (
          <label className="field interaction-text">
            <span>
              {promptContent.choices.length > 0
                ? "Or enter a response"
                : "Response"}
            </span>
            <textarea
              value={text}
              maxLength={16_384}
              rows={2}
              disabled={unavailable}
              onChange={(event) => {
                setText(event.target.value);
                setChoiceId("");
              }}
            />
          </label>
        )}
        {error !== "" && (
          <p className="inline-error" role="alert">
            {error}
          </p>
        )}
      </div>
      <div className="interaction-actions prompt-actions">
        <span className="interaction-guidance" aria-live="polite">
          {responseGuidance}
        </span>
        <button
          className="button primary"
          type="button"
          disabled={!canSubmit}
          onClick={submitPrompt}
        >
          {submitting ? "Sending…" : "Send response"}
        </button>
      </div>
    </section>
  );
}
