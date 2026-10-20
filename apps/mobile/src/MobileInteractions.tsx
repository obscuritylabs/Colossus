import { useEffect, useRef, useState } from "react";
import { Button, TextInput } from "@colossus/ui";
import type {
  CommandReceipt,
  Interaction,
  Permission,
} from "@colossus/control-plane-client";
import {
  ClientFailure,
  messageOf,
  type InteractionResponseRequest,
  type MobileClient,
  type Operation,
} from "./client";

type Attempt = {
  operation: Extract<Operation, { kind: "respond" }>;
  command?: CommandReceipt;
  uncertain: boolean;
  summary: string;
  interaction: Interaction;
};
/** Responses preserve the reviewed operation and both replay identities until receipt reconciliation. */
export function MobileInteractions({
  client,
  project,
  task,
  interactions,
  permissions,
  onUpdate,
  onError,
  onLocked,
}: {
  client: MobileClient;
  project: string;
  task: string;
  interactions: Interaction[];
  permissions: Permission[];
  onUpdate: () => void | Promise<void>;
  onError: (message: string) => void;
  onLocked?: (locked: boolean, task: string) => void;
}) {
  const [busy, setBusy] = useState(false),
    [notice, setNotice] = useState("");
  const [answers, setAnswers] = useState<Record<string, string>>({});
  const [confirmed, setConfirmed] = useState<Set<string>>(() => new Set());

  const [retained, setRetained] = useState<Attempt | null>(null);
  const attempt = useRef<Attempt | null>(null);
  useEffect(() => {
    onLocked?.(busy || retained !== null, task);
    return () => onLocked?.(false, task);
  }, [busy, retained, onLocked, task]);
  async function submit(
    interaction?: Interaction,
    response?: InteractionResponseRequest["response"],
  ) {
    if (busy) return;
    if (!attempt.current) {
      if (!interaction || !response) return;
      attempt.current = {
        operation: {
          kind: "respond",
          project,
          task,
          mutation_id: crypto.randomUUID(),
          request: {
            run_id: interaction.run_id,
            interaction_id: interaction.interaction_id,
            etag: interaction.etag,
            idempotency_key: crypto.randomUUID(),
            response,
          },
        },
        uncertain: false,
        interaction: structuredClone(interaction),
        summary:
          "approval" in response
            ? `${response.approval.approved ? "Approve" : "Deny"} ${interaction.content.approval?.action ?? "action"} on ${interaction.content.approval?.resource ?? "the requested resource"}`
            : "choice" in response.prompt
              ? response.prompt.choice.label
              : response.prompt.free_form,
      };
    }
    const current = attempt.current;
    setBusy(true);
    setNotice("");
    onError("");
    try {
      if (!current.command)
        current.command = (
          await client.request<{ command: CommandReceipt }>(current.operation)
        ).command;
      else
        current.command = (
          await client.request<{ command: CommandReceipt }>({
            kind: "command",
            project: current.operation.project,
            task: current.operation.task,
            command: current.command.command_id,
          })
        ).command;
      const deadline = Date.now() + 5000;
      while (!current.command.reply && Date.now() < deadline) {
        await new Promise<void>((resolve) => setTimeout(resolve, 300));
        current.command = (
          await client.request<{ command: CommandReceipt }>({
            kind: "command",
            project: current.operation.project,
            task: current.operation.task,
            command: current.command.command_id,
          })
        ).command;
      }
      if (!current.command.reply) {
        setNotice(
          "Response queued. Check its receipt when the runtime reconnects.",
        );
        setRetained({ ...current });
      } else {
        if (current.command.reply.kind === "failed")
          throw new Error(
            current.command.reply.error?.message ??
              "The runtime rejected this response.",
          );
        setNotice("Response confirmed by the runtime.");
        const request = current.operation.request;
        setConfirmed((identities) =>
          new Set(identities).add(
            JSON.stringify([
              request.run_id,
              request.interaction_id,
              request.etag,
            ]),
          ),
        );

        attempt.current = null;
        setRetained(null);
      }
      await onUpdate();
    } catch (failure) {
      const unknown =
        !current.command?.reply &&
        (current.uncertain ||
          Boolean(current.command) ||
          (failure instanceof ClientFailure &&
            failure.code === "outcome_unknown"));
      if (unknown) {
        current.uncertain = true;
        setRetained({ ...current });
      } else {
        attempt.current = null;
        setRetained(null);
      }
      onError(messageOf(failure));
    } finally {
      setBusy(false);
    }
  }
  return (
    <>
      {notice ? (
        <p role="status" className="mobile-notice">
          {notice}
        </p>
      ) : null}
      {retained ? (
        <section className="mobile-attention" aria-label="Retained response">
          <p>
            {retained.command
              ? "Your response is awaiting its runtime receipt."
              : "The response outcome is unknown. Reconcile the original response before sending another."}
          </p>
          <p>Original response: {retained.summary}</p>
          <Button disabled={busy} onClick={() => void submit()}>
            {retained.command
              ? "Check response receipt"
              : "Reconcile original response"}
          </Button>
        </section>
      ) : null}
      {(retained
        ? [retained.interaction]
        : interactions.filter((interaction) => interaction.status === "pending")
      ).map((interaction) => {
        const approval = interaction.content.approval,
          prompt = interaction.content.user_prompt;
        const allowed =
          interaction.respondable_by_caller &&
          permissions.includes(approval ? "approve" : "control");
        const responded = confirmed.has(
          JSON.stringify([
            interaction.run_id,
            interaction.interaction_id,
            interaction.etag,
          ]),
        );
        const disabled = !allowed || busy || retained !== null || responded;
        return (
          <section
            className="mobile-attention mobile-interaction"
            key={interaction.interaction_id}
            aria-label={approval ? "Approval required" : "Answer required"}
          >
            <h2>{approval ? "Approval required" : "Your input is needed"}</h2>
            {approval ? (
              <>
                <p>{approval.reason}</p>
                <dl className="mobile-task-facts">
                  <div>
                    <dt>Action</dt>
                    <dd>{approval.action}</dd>
                  </div>
                  <div>
                    <dt>Resource</dt>
                    <dd>{approval.resource}</dd>
                  </div>
                  <div>
                    <dt>Risk</dt>
                    <dd>{approval.risk ?? "Unspecified"}</dd>
                  </div>
                </dl>
                {approval.command_context ? (
                  <section aria-label="Prepared command">
                    <h3>Prepared command</h3>
                    <p>{approval.command_context.justification}</p>
                    <dl>
                      <dt>Executable</dt>
                      <dd>
                        <code>{approval.command_context.executable}</code>
                      </dd>
                      <dt>Arguments</dt>
                      <dd>
                        <ol>
                          {approval.command_context.arguments.map(
                            (argument, index) => (
                              <li key={index}>
                                <code>{JSON.stringify(argument)}</code>
                              </li>
                            ),
                          )}
                        </ol>
                      </dd>
                      <dt>Working directory</dt>
                      <dd>
                        <code>
                          {approval.command_context.working_directory}
                        </code>
                      </dd>
                    </dl>
                    {approval.command_context.redacted ? (
                      <p>Credentials are redacted.</p>
                    ) : null}
                  </section>
                ) : null}
                <div className="mobile-task-actions">
                  <Button
                    disabled={disabled}
                    onClick={() =>
                      void submit(interaction, {
                        approval: {
                          approved: true,
                          request_hash: approval.request_hash,
                        },
                      })
                    }
                  >
                    Approve this action
                  </Button>
                  <Button
                    variant="secondary"
                    disabled={disabled}
                    onClick={() =>
                      void submit(interaction, {
                        approval: {
                          approved: false,
                          request_hash: approval.request_hash,
                        },
                      })
                    }
                  >
                    Deny
                  </Button>
                </div>
              </>
            ) : prompt ? (
              <>
                <p>{prompt.question}</p>
                <div className="mobile-task-actions">
                  {prompt.choices.map((choice) => (
                    <Button
                      key={choice.choice_id}
                      variant="secondary"
                      disabled={disabled}
                      onClick={() =>
                        void submit(interaction, {
                          prompt: {
                            choice: {
                              choice_id: choice.choice_id,
                              label: choice.label,
                            },
                          },
                        })
                      }
                    >
                      {choice.label}
                    </Button>
                  ))}
                </div>
                {prompt.allow_free_form ? (
                  <form
                    onSubmit={(event) => {
                      event.preventDefault();
                      if (answers[interaction.interaction_id]?.trim())
                        void submit(interaction, {
                          prompt: {
                            free_form: answers[interaction.interaction_id]!,
                          },
                        });
                    }}
                  >
                    <label htmlFor={`answer-${interaction.interaction_id}`}>
                      Your answer
                    </label>
                    <TextInput
                      id={`answer-${interaction.interaction_id}`}
                      value={answers[interaction.interaction_id] ?? ""}
                      maxLength={65536}
                      disabled={disabled}
                      onChange={(event) =>
                        setAnswers((current) => ({
                          ...current,
                          [interaction.interaction_id]: event.target.value,
                        }))
                      }
                    />
                    <Button
                      type="submit"
                      disabled={
                        disabled || !answers[interaction.interaction_id]?.trim()
                      }
                    >
                      Send answer
                    </Button>
                  </form>
                ) : null}
              </>
            ) : null}
            {responded ? (
              <p className="mobile-caption">
                Response confirmed. Waiting for task status to refresh.
              </p>
            ) : null}
            {!allowed ? (
              <p className="mobile-caption">
                Your project permissions or runtime grant do not allow this
                response.
              </p>
            ) : null}
          </section>
        );
      })}
    </>
  );
}
