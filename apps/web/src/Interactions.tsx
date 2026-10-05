import { useRef, useState } from "react";
import { IconShieldCheck, IconMessageQuestion } from "@tabler/icons-react";
import {
  request,
  projectPath,
  awaitReceipt,
  type CommandReceipt,
  type Interaction,
  type Permission,
} from "./api";
export function Interactions({
  interactions,
  project,
  task,
  permissions,
  onUpdate,
  onError,
}: {
  interactions: Interaction[];
  project: string;
  task: string;
  permissions: Permission[];
  onUpdate: () => void;
  onError: (message: string) => void;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const [notice, setNotice] = useState("");
  const attempts = useRef(
    new Map<string, { mutation: string; idempotency: string }>(),
  );
  const [answers, setAnswers] = useState<Record<string, string>>({});
  async function respond(interaction: Interaction, response: unknown) {
    const signature = JSON.stringify([
      interaction.interaction_id,
      interaction.etag,
      response,
    ]);
    let attempt = attempts.current.get(signature);
    if (!attempt) {
      attempt = {
        mutation: crypto.randomUUID(),
        idempotency: crypto.randomUUID(),
      };
      attempts.current.set(signature, attempt);
    }
    setBusy(interaction.interaction_id);
    setNotice("");
    try {
      const path = `${projectPath(project)}/tasks/${task}`;
      const { command } = await request<{ command: CommandReceipt }>(
        `${path}/respond`,
        {
          mutation_id: attempt.mutation,
          request: {
            run_id: interaction.run_id,
            interaction_id: interaction.interaction_id,
            etag: interaction.etag,
            idempotency_key: attempt.idempotency,
            response,
          },
        },
      );
      const settled = await awaitReceipt(path, command);
      setNotice(
        settled
          ? "Response confirmed by the runtime."
          : "Response queued. It will be delivered when the runtime reconnects; retrying uses this same response.",
      );
      onUpdate();
    } catch (error) {
      onError(error instanceof Error ? error.message : "Response failed.");
    } finally {
      setBusy(null);
    }
  }
  return (
    <>
      {notice && (
        <p className="muted" role="status">
          {notice}
        </p>
      )}
      {interactions
        .filter((interaction) => interaction.status === "pending")
        .map((interaction) => {
          const approval = interaction.content.approval,
            prompt = interaction.content.user_prompt;
          const allowed =
            interaction.respondable_by_caller &&
            permissions.includes(approval ? "approve" : "control");
          return (
            <section
              className="interaction"
              key={interaction.interaction_id}
              aria-label={approval ? "Approval required" : "Answer required"}
            >
              <div className="interaction-heading">
                {approval ? (
                  <IconShieldCheck size={19} />
                ) : (
                  <IconMessageQuestion size={19} />
                )}
                <h3>
                  {approval ? "Approval required" : "Your input is needed"}
                </h3>
              </div>
              {approval ? (
                <>
                  <p>{approval.reason}</p>
                  {approval.command_context && (
                    <div className="command-review">
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
                      {approval.command_context.redacted && (
                        <p className="muted">
                          Credentials are redacted from these prepared command
                          details.
                        </p>
                      )}
                    </div>
                  )}
                  <dl>
                    <dt>Action</dt>
                    <dd>{approval.action}</dd>
                    <dt>Resource</dt>
                    <dd>{approval.resource}</dd>
                    <dt>Risk</dt>
                    <dd>{approval.risk ?? "Unspecified"}</dd>
                  </dl>
                  <div className="actions">
                    <button
                      disabled={!allowed || busy !== null}
                      onClick={() =>
                        void respond(interaction, {
                          approval: {
                            approved: true,
                            request_hash: approval.request_hash,
                          },
                        })
                      }
                    >
                      Approve this action
                    </button>
                    <button
                      className="secondary"
                      disabled={!allowed || busy !== null}
                      onClick={() =>
                        void respond(interaction, {
                          approval: {
                            approved: false,
                            request_hash: approval.request_hash,
                          },
                        })
                      }
                    >
                      Deny
                    </button>
                  </div>
                </>
              ) : prompt ? (
                <>
                  <p>{prompt.question}</p>
                  <div className="choice-actions">
                    {prompt.choices.map((choice) => (
                      <button
                        key={choice.choice_id}
                        className="secondary"
                        disabled={!allowed || busy !== null}
                        onClick={() =>
                          void respond(interaction, {
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
                      </button>
                    ))}
                  </div>
                  {prompt.allow_free_form && (
                    <form
                      onSubmit={(event) => {
                        event.preventDefault();
                        void respond(interaction, {
                          prompt: {
                            free_form:
                              answers[interaction.interaction_id] ?? "",
                          },
                        });
                      }}
                    >
                      <label
                        className="sr-only"
                        htmlFor={`answer-${interaction.interaction_id}`}
                      >
                        Your answer
                      </label>
                      <input
                        id={`answer-${interaction.interaction_id}`}
                        value={answers[interaction.interaction_id] ?? ""}
                        onChange={(event) =>
                          setAnswers({
                            ...answers,
                            [interaction.interaction_id]: event.target.value,
                          })
                        }
                        maxLength={65536}
                        disabled={!allowed || busy !== null}
                        placeholder="Write your answer…"
                      />
                      <button
                        disabled={
                          !allowed ||
                          busy !== null ||
                          !answers[interaction.interaction_id]?.trim()
                        }
                      >
                        Send answer
                      </button>
                    </form>
                  )}
                </>
              ) : null}
              {!allowed && (
                <p className="muted">
                  Your project permissions or local runtime grant do not allow
                  this response.
                </p>
              )}
            </section>
          );
        })}
    </>
  );
}
