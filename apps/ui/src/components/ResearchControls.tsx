import {
  IconCheck,
  IconFolder,
  IconPlugConnected,
  IconWorld,
  IconX,
} from "@tabler/icons-react";
import { useId, useLayoutEffect, useRef } from "react";
import type { ResearchDepth, ResearchSourceKind } from "../session/types";

const RESEARCH_DEPTH_OPTIONS = [
  { value: "quick", label: "Quick" },
  { value: "standard", label: "Standard" },
  { value: "deep", label: "Deep" },
] as const;

export const RESEARCH_SOURCE_OPTIONS = [
  {
    value: "repo",
    label: "This Workspace",
    description: "Search across your workspace",
    Icon: IconFolder,
  },
  {
    value: "web",
    label: "Web",
    description: "Search the public web",
    Icon: IconWorld,
  },
  {
    value: "mcp",
    label: "MCP connections",
    description: "Search enabled MCP tools or research projections",
    Icon: IconPlugConnected,
  },
] as const;

/** Controlled presentation; hosts decide which evidence lanes are available. */
export function ResearchControls({
  researchDepth,
  researchSources,
  submitting = false,
  onResearchDepthChange,
  onResearchSourcesChange,
  availableSources = ["repo", "web", "mcp"],
}: {
  researchDepth: ResearchDepth;
  researchSources: readonly ResearchSourceKind[];
  submitting?: boolean;
  availableSources?: readonly ResearchSourceKind[];
  onResearchDepthChange: (depth: ResearchDepth) => void;
  onResearchSourcesChange: (sources: ResearchSourceKind[]) => void;
}) {
  const name = `research-depth-${useId()}`;
  const header = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    const popup = header.current?.closest<HTMLElement>(".run-controls-popover");
    const details = popup?.closest("details");
    if (!popup || !details) return;
    const previous = {
      position: popup.style.position,
      inset: popup.style.inset,
      maxHeight: popup.style.maxHeight,
    };
    const place = () => {
      popup.style.position = previous.position;
      popup.style.inset = previous.inset;
      if (!details.open) return;
      popup.style.maxHeight = `${Math.min(480, window.innerHeight - 24)}px`;
      const rect = popup.getBoundingClientRect();
      popup.style.position = "fixed";
      popup.style.inset = `${Math.max(12, rect.top)}px auto auto ${Math.max(12, Math.min(rect.left, window.innerWidth - 12 - rect.width))}px`;
    };
    const observer = new ResizeObserver(place);
    observer.observe(popup);
    details.addEventListener("toggle", place);
    window.addEventListener("resize", place);
    document.addEventListener("scroll", place, true);
    place();
    return () => {
      observer.disconnect();
      details.removeEventListener("toggle", place);
      window.removeEventListener("resize", place);
      document.removeEventListener("scroll", place, true);
      popup.style.position = previous.position;
      popup.style.inset = previous.inset;
      popup.style.maxHeight = previous.maxHeight;
    };
  }, []);
  return (
    <>
      <header ref={header} className="research-settings-header">
        <h3>Research settings</h3>
        <button
          className="research-settings-close"
          type="button"
          aria-label="Close research settings"
          onClick={(event) => {
            const controls = event.currentTarget.closest("details");
            controls?.removeAttribute("open");
            controls?.querySelector("summary")?.focus();
          }}
        >
          <IconX size={17} stroke={1.8} aria-hidden="true" />
        </button>
      </header>
      <fieldset className="research-depth-controls">
        <legend>Research depth</legend>
        <div className="research-depth-options">
          {RESEARCH_DEPTH_OPTIONS.map((option) => (
            <label className="research-depth-option" key={option.value}>
              <input
                type="radio"
                name={name}
                value={option.value}
                checked={researchDepth === option.value}
                disabled={submitting}
                onChange={() => onResearchDepthChange(option.value)}
              />
              <span>{option.label}</span>
            </label>
          ))}
        </div>
      </fieldset>
      <fieldset className="research-source-controls">
        <legend>Evidence sources</legend>
        <div className="research-source-options">
          {RESEARCH_SOURCE_OPTIONS.map((option) => {
            const selected = researchSources.includes(option.value);
            return (
              <label
                className={`research-source-option${selected ? " is-selected" : ""}`}
                key={option.value}
              >
                <input
                  type="checkbox"
                  checked={selected}
                  disabled={
                    submitting || !availableSources.includes(option.value)
                  }
                  onChange={(event) => {
                    const next = event.target.checked
                      ? [...researchSources, option.value]
                      : researchSources.filter((item) => item !== option.value);
                    onResearchSourcesChange(next);
                  }}
                />
                <span className="research-source-icon" aria-hidden="true">
                  <option.Icon size={19} stroke={1.7} />
                </span>
                <span className="research-source-copy">
                  <strong>{option.label}</strong>
                  <small>{option.description}</small>
                </span>
                <span className="research-source-checkbox" aria-hidden="true">
                  {selected ? <IconCheck size={14} stroke={2.4} /> : null}
                </span>
              </label>
            );
          })}
        </div>
      </fieldset>
    </>
  );
}
