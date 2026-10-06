import { IconSparkles } from "@tabler/icons-react";
import colossusMark from "../../assets/colossus-mark.svg";
import { Button } from "./Controls.js";

export const DEFAULT_WORK_STARTERS = [
  "Orient yourself in this repo",
  "Plan a secure migration without making external changes",
  "Coordinate an implementation and security review",
] as const;

export interface WorkWelcomeProps {
  eyebrow?: string;
  title?: string;
  description?: string;
  suggestions?: readonly string[];
  onSuggestion: (suggestion: string) => void;
  disabled?: boolean;
}

/** Shared new-work presentation; the host owns drafts, submission and authority. */
export function WorkWelcome({
  eyebrow = "Local-first agent workspace",
  title = "Give Colossus a goal. Keep control of every effect.",
  description = "Start a task, switch to plan mode, or coordinate specialist work through one policy-bound local connection.",
  suggestions = DEFAULT_WORK_STARTERS,
  onSuggestion,
  disabled = false,
}: WorkWelcomeProps) {
  return (
    <section className="shared-work-welcome" aria-label="Start new work">
      <img src={colossusMark} alt="" width={56} height={56} />
      <p className="shared-work-welcome-eyebrow">{eyebrow}</p>
      <h3>{title}</h3>
      <p className="shared-work-welcome-description">{description}</p>
      <div className="shared-work-starters" aria-label="Example prompts">
        {suggestions.map((suggestion) => (
          <Button
            key={suggestion}
            type="button"
            variant="tertiary"
            disabled={disabled}
            onClick={() => onSuggestion(suggestion)}
          >
            <IconSparkles size={17} stroke={1.6} aria-hidden="true" />
            <span>{suggestion}</span>
          </Button>
        ))}
      </div>
    </section>
  );
}
