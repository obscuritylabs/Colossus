import { useState } from "react";
import { IconCalendarTime, IconCopy, IconPlus } from "@tabler/icons-react";
import { WorkflowDialog } from "./WorkflowDialog";
import {
  scheduleExamples,
  scheduleExamplePrompt,
  type ScheduleExample,
} from "./schedule-examples";
import "./catalog-inventory.css";
import "./schedule-examples.css";

export function ScheduleExamples({
  disabled,
  onUse,
}: {
  disabled: boolean;
  onUse: (example: ScheduleExample) => void;
}) {
  const [promptExample, setPromptExample] = useState<ScheduleExample | null>(
    null,
  );
  const [copyState, setCopyState] = useState("");
  const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  const prompt = promptExample
    ? scheduleExamplePrompt(promptExample, timezone)
    : "";
  const openPrompt = (example: ScheduleExample) => {
    setCopyState("");
    setPromptExample(example);
  };
  async function copy() {
    try {
      await navigator.clipboard.writeText(prompt);
      setCopyState(
        "Prompt copied. Paste it into an agent chat in this workspace.",
      );
    } catch {
      setCopyState(
        "Clipboard is unavailable. Select and copy the prompt below.",
      );
    }
  }
  return (
    <section
      className="schedule-examples"
      aria-labelledby="schedule-examples-heading"
    >
      <header className="workflow-detail-header">
        <div>
          <h3 id="schedule-examples-heading">Start with an example</h3>
          <p>
            Use a starting point, then review the instructions, timing, and
            tools.
          </p>
        </div>
        <button
          className="button secondary"
          onClick={() => openPrompt(scheduleExamples[0]!)}
        >
          Create with an agent
        </button>
      </header>
      <div className="schedule-example-grid">
        {scheduleExamples.map((example) => (
          <article
            className="schedule-example"
            key={example.id}
            aria-label={example.name}
          >
            <div className="schedule-example-heading">
              <span className="resource-icon">
                <IconCalendarTime size={16} aria-hidden="true" />
              </span>
              <h4>{example.name}</h4>
            </div>
            <p>{example.description}</p>
            <div className="workflow-actions">
              <button
                className="button secondary"
                disabled={disabled}
                onClick={() => onUse(example)}
                aria-label={`Use ${example.name} example`}
              >
                <IconPlus size={14} aria-hidden="true" /> Use example
              </button>
              <button
                className="catalog-text-button"
                onClick={() => openPrompt(example)}
                aria-label={`Agent prompt for ${example.name}`}
              >
                Agent prompt
              </button>
            </div>
          </article>
        ))}
      </div>
      <p className="workflow-help">
        Examples never save automatically. Research examples need a configured
        search service; every task follows current workspace permissions.
      </p>
      {promptExample && (
        <WorkflowDialog
          title="Create a schedule with an agent"
          busy={false}
          onClose={() => setPromptExample(null)}
        >
          <p>
            Paste this prompt into an agent chat. The bundled{" "}
            <strong>colossus/schedule-task</strong> skill guides the agent
            through timing, tools, review, and confirmation.
          </p>
          <label className="schedule-agent-prompt">
            Agent prompt
            <textarea readOnly value={prompt} rows={14} />
          </label>
          {copyState && <p role="status">{copyState}</p>}
          <footer className="workflow-actions">
            <button
              className="button secondary"
              onClick={() => setPromptExample(null)}
            >
              Close
            </button>
            <button className="button primary" onClick={() => void copy()}>
              <IconCopy size={16} aria-hidden="true" /> Copy agent prompt
            </button>
          </footer>
        </WorkflowDialog>
      )}
    </section>
  );
}
