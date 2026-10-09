import { AutomationExampleGallery } from "../automations";
import {
  IconCalendarTime,
  IconFileText,
  IconGitBranch,
  IconFolder,
  IconShieldCheck,
} from "@tabler/icons-react";
import { scheduleExamples, type ScheduleExample } from "./schedule-examples";

const exampleIcons = [
  IconShieldCheck,
  IconShieldCheck,
  IconFolder,
  IconGitBranch,
  IconFileText,
  IconCalendarTime,
];
export function ScheduleExamples({
  disabled,
  onCreateWithAgent,
}: {
  disabled: boolean;
  onCreateWithAgent: (example: ScheduleExample) => void;
}) {
  return (
    <AutomationExampleGallery
      title="Start with an example"
      description="Choose a task. An agent will tailor the instructions and timing with you in a new chat."
      disabled={disabled}
      examples={scheduleExamples.map((example, index) => {
        const Icon = exampleIcons[index] || IconCalendarTime;
        const [timing = "", description = ""] =
          example.description.split(" · ");
        return {
          id: example.id,
          name: example.name,
          description,
          timing,
          icon: <Icon size={22} stroke={1.6} aria-hidden="true" />,
        };
      })}
      onCreate={(id) => {
        const example = scheduleExamples.find((item) => item.id === id);
        if (example) onCreateWithAgent(example);
      }}
    />
  );
}
