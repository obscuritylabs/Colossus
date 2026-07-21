import {
  IconArrowRight,
  IconPencil,
  IconShieldCheck,
  IconSparkles,
  IconTool,
} from "@tabler/icons-react";
import type { ComponentType } from "react";

export type AgentIcon = "lead" | "builder" | "security" | "writer";
export type AgentWorkState =
  "coordinating" | "working" | "reviewing" | "waiting" | "idle";

export interface AgentParticipant {
  id: string;
  name: string;
  role: string;
  state: AgentWorkState;
  icon: AgentIcon;
}

interface AgentFlowProps {
  participants: readonly AgentParticipant[];
}

const ICONS: Record<
  AgentIcon,
  ComponentType<{ size?: number; stroke?: number }>
> = {
  lead: IconSparkles,
  builder: IconTool,
  security: IconShieldCheck,
  writer: IconPencil,
};

function readableState(state: AgentWorkState): string {
  return state.charAt(0).toUpperCase() + state.slice(1);
}

export function AgentFlow({ participants }: AgentFlowProps) {
  return (
    <section className="agent-flow" aria-labelledby="agent-flow-title">
      <h2 className="sr-only" id="agent-flow-title">
        Participating agents
      </h2>
      <ol>
        {participants.map((participant, index) => {
          const Icon = ICONS[participant.icon];
          return (
            <li key={participant.id}>
              <div className="agent-flow-card">
                <span className="agent-flow-avatar" aria-hidden="true">
                  <Icon size={19} stroke={1.7} />
                </span>
                <span className="agent-flow-copy">
                  <strong>{participant.name}</strong>
                  <span>{participant.role}</span>
                </span>
                <span
                  className={`agent-state agent-state-${participant.state}`}
                >
                  <i aria-hidden="true" />
                  {readableState(participant.state)}
                </span>
              </div>
              {index < participants.length - 1 ? (
                <IconArrowRight
                  className="agent-flow-arrow"
                  size={22}
                  stroke={1.4}
                  aria-hidden="true"
                />
              ) : null}
            </li>
          );
        })}
      </ol>
    </section>
  );
}
