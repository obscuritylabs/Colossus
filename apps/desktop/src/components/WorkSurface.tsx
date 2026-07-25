import {
  IconClock,
  IconFiles,
  IconFolderOpen,
  IconMenu2,
  IconPlayerStop,
  IconPlugConnected,
  IconRefresh,
  IconShieldLock,
  IconSparkles,
  IconX,
} from "@tabler/icons-react";
import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";

import colossusMark from "../assets/colossus-mark.svg";
import { presentRunStatus, shortDateLabel } from "../presenters";
import type { RunView } from "../state";
import type {
  CommandError,
  ConnectionStatus,
  Interaction,
  InteractionAnswer,
} from "../types";
import type { AgentParticipant } from "./AgentFlow";
import { AgentFlow } from "./AgentFlow";
import type { ArtifactViewItem } from "./ArtifactWorkspace";
import { ArtifactWorkspace } from "./ArtifactWorkspace";
import { InteractionCard } from "./InteractionCard";
import { RunTimeline } from "./RunTimeline";

interface WorkSurfaceProps {
  title: string;
  view: RunView | undefined;
  conversationViews: readonly RunView[];
  connection: ConnectionStatus;
  connecting: boolean;
  cancelling: boolean;
  runLoadError: string;
  actionError: CommandError | null;
  participants: readonly AgentParticipant[];
  artifacts: readonly ArtifactViewItem[];
  composer: ReactNode;
  filesPanel: ReactNode;
  filesAvailable: boolean;
  artifactsAvailable: boolean;
  workNavigationOpen: boolean;
  onConnect: () => void;
  onCancel: () => void;
  onRespond: (
    interaction: Interaction,
    response: InteractionAnswer,
  ) => Promise<void>;
  onResume: () => void;
  onSuggestion: (suggestion: string) => void;
  onOpenWorkNavigation: () => void;
  onCloseWorkNavigation: () => void;
}

const STARTERS = [
  "Review this workspace and identify the safest high-impact next task",
  "Plan a secure migration without making external changes",
  "Coordinate an implementation and security review",
];

export function WorkSurface({
  title,
  view,
  conversationViews,
  connection,
  connecting,
  cancelling,
  runLoadError,
  actionError,
  participants,
  artifacts,
  composer,
  filesPanel,
  filesAvailable,
  artifactsAvailable,
  workNavigationOpen,
  onConnect,
  onCancel,
  onRespond,
  onResume,
  onSuggestion,
  onOpenWorkNavigation,
  onCloseWorkNavigation,
}: WorkSurfaceProps) {
  const [compactLayout, setCompactLayout] = useState(
    () => window.matchMedia("(max-width: 980px)").matches,
  );
  const [activeDrawer, setActiveDrawer] = useState<
    "files" | "artifacts" | null
  >(null);
  const [filesDrawerMounted, setFilesDrawerMounted] = useState(false);
  const drawerRef = useRef<HTMLDivElement>(null);
  const drawerCloseRef = useRef<HTMLButtonElement>(null);
  const filesTriggerRef = useRef<HTMLButtonElement>(null);
  const artifactTriggerRef = useRef<HTMLButtonElement>(null);
  const lastDrawerTriggerRef = useRef<HTMLButtonElement | null>(null);
  const workNavigationTriggerRef = useRef<HTMLButtonElement>(null);
  const previousWorkNavigationOpen = useRef(workNavigationOpen);
  const run = view?.run;
  const status = run === undefined ? null : presentRunStatus(run.status);
  const startedAt = run?.startedAt ?? run?.createdAt;
  const startedLabel =
    startedAt === undefined ? null : shortDateLabel(startedAt);

  useEffect(() => {
    const media = window.matchMedia("(max-width: 980px)");
    const onChange = (event: MediaQueryListEvent) => {
      setCompactLayout(event.matches);
      if (!event.matches) {
        setActiveDrawer(null);
        onCloseWorkNavigation();
      }
    };
    setCompactLayout(media.matches);
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, [onCloseWorkNavigation]);

  useEffect(() => {
    if (previousWorkNavigationOpen.current && !workNavigationOpen) {
      requestAnimationFrame(() => workNavigationTriggerRef.current?.focus());
    }
    previousWorkNavigationOpen.current = workNavigationOpen;
    if (workNavigationOpen) {
      setActiveDrawer(null);
    }
  }, [workNavigationOpen]);

  useEffect(() => {
    if (
      (activeDrawer === "files" && !filesAvailable) ||
      (activeDrawer === "artifacts" && !artifactsAvailable)
    ) {
      setActiveDrawer(null);
    }
  }, [activeDrawer, artifactsAvailable, filesAvailable]);

  useEffect(() => {
    if (activeDrawer === null || !compactLayout) {
      return;
    }
    const obscured = [
      document.querySelector<HTMLElement>(".product-rail"),
      document.querySelector<HTMLElement>("#work-navigation"),
      document.querySelector<HTMLElement>(".work-surface-header"),
      document.querySelector<HTMLElement>(".agent-flow"),
      document.querySelector<HTMLElement>(".work-thread"),
    ].flatMap((element) =>
      element === null
        ? []
        : [{ element, wasInert: element.hasAttribute("inert") }],
    );
    for (const { element } of obscured) {
      element.setAttribute("inert", "");
    }
    const focusTimer = window.setTimeout(
      () => drawerCloseRef.current?.focus(),
      180,
    );
    function onDrawerKeyDown(event: globalThis.KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        const trigger = lastDrawerTriggerRef.current;
        setActiveDrawer(null);
        requestAnimationFrame(() => trigger?.focus());
        return;
      }
      if (event.key !== "Tab") {
        return;
      }
      const focusable = Array.from(
        drawerRef.current?.querySelectorAll<HTMLElement>(
          'button:not(:disabled):not([tabindex="-1"]), [tabindex]:not([tabindex="-1"])',
        ) ?? [],
      ).filter(
        (element) => !element.hidden && element.closest("[hidden]") === null,
      );
      const first = focusable[0];
      const last = focusable.at(-1);
      if (first === undefined || last === undefined) {
        return;
      }
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }
    window.addEventListener("keydown", onDrawerKeyDown);
    return () => {
      window.clearTimeout(focusTimer);
      window.removeEventListener("keydown", onDrawerKeyDown);
      for (const { element, wasInert } of obscured) {
        if (!wasInert) {
          element.removeAttribute("inert");
        }
      }
    };
  }, [activeDrawer, compactLayout]);

  function closeDrawer() {
    const trigger = lastDrawerTriggerRef.current;
    setActiveDrawer(null);
    requestAnimationFrame(() => trigger?.focus());
  }

  function toggleDrawer(drawer: "files" | "artifacts") {
    const trigger =
      drawer === "files" ? filesTriggerRef.current : artifactTriggerRef.current;
    lastDrawerTriggerRef.current = trigger;
    if (activeDrawer === drawer) {
      closeDrawer();
      return;
    }
    onCloseWorkNavigation();
    if (drawer === "files") {
      setFilesDrawerMounted(true);
    }
    setActiveDrawer(drawer);
  }

  return (
    <main
      className={`work-surface${view === undefined ? " is-new-work" : ""}`}
      id="primary-workspace"
      tabIndex={-1}
    >
      <header className="surface-header work-surface-header">
        <div className="surface-title-copy">
          <p className="surface-breadcrumb">
            <span>Work</span>
            <span aria-hidden="true">/</span>
            <span>{status?.label ?? "New work"}</span>
          </p>
          <h2>{title}</h2>
          {run !== undefined ? (
            <p className="surface-run-meta">
              <span className={`tone-${status?.tone ?? "neutral"}`}>
                {status?.copy}
              </span>
              {startedLabel !== null ? (
                <span>
                  <IconClock size={12} stroke={1.7} aria-hidden="true" />
                  Started {startedLabel}
                </span>
              ) : null}
              <span>{run.mode === "plan" ? "Plan mode" : "Execute mode"}</span>
            </p>
          ) : null}
        </div>
        <div className="surface-header-actions">
          <button
            ref={workNavigationTriggerRef}
            className="button secondary compact work-navigation-button"
            type="button"
            aria-label="Open work navigation"
            aria-controls="work-navigation"
            aria-expanded={workNavigationOpen}
            onClick={onOpenWorkNavigation}
          >
            <IconMenu2 size={16} stroke={1.8} aria-hidden="true" />
            <span className="compact-action-copy">Work</span>
          </button>
          <span
            className={`connection-badge connection-${connection.state}`}
            title={connection.message}
          >
            <IconPlugConnected size={15} stroke={1.8} aria-hidden="true" />
            {connection.state === "connected" ? "Agent online" : "Disconnected"}
          </span>
          {filesAvailable ? (
            <button
              ref={filesTriggerRef}
              className="button secondary compact files-open-button"
              type="button"
              aria-label={`${activeDrawer === "files" ? "Close" : "Open"} files panel`}
              aria-controls="work-side-drawer"
              aria-expanded={activeDrawer === "files"}
              onClick={() => toggleDrawer("files")}
            >
              <IconFiles size={15} stroke={1.7} aria-hidden="true" />
              <span className="compact-action-copy">Files</span>
            </button>
          ) : null}
          {artifactsAvailable ? (
            <button
              ref={artifactTriggerRef}
              className="button secondary compact artifact-open-button"
              type="button"
              aria-label={`${activeDrawer === "artifacts" ? "Close" : "Open"} artifacts panel, ${artifacts.length} ${
                artifacts.length === 1 ? "artifact" : "artifacts"
              }`}
              aria-controls="work-side-drawer"
              aria-expanded={activeDrawer === "artifacts"}
              onClick={() => toggleDrawer("artifacts")}
            >
              <IconFolderOpen size={15} stroke={1.7} aria-hidden="true" />
              <span className="compact-action-copy">Artifacts</span>
              <span className="artifact-count" aria-hidden="true">
                {artifacts.length}
              </span>
            </button>
          ) : null}
          {run !== undefined &&
          (run.status === "queued" ||
            run.status === "running" ||
            run.status === "waiting") ? (
            <button
              className="button secondary compact"
              type="button"
              disabled={cancelling}
              onClick={onCancel}
            >
              <IconPlayerStop size={15} stroke={1.8} aria-hidden="true" />
              {cancelling ? "Stopping…" : "Stop"}
            </button>
          ) : null}
        </div>
      </header>

      {view !== undefined ? <AgentFlow participants={participants} /> : null}

      <div
        className={`work-layout${activeDrawer !== null ? " is-work-drawer-open" : ""}`}
      >
        <section className="work-thread" aria-label="Work conversation">
          <div className="work-feed-scroll">
            {connection.state !== "connected" ? (
              <section className="connection-panel" aria-live="polite">
                <span className="panel-icon" aria-hidden="true">
                  <IconShieldLock size={25} stroke={1.5} />
                </span>
                <div>
                  <p className="eyebrow">
                    {connection.state === "not_configured"
                      ? "One-time setup"
                      : "Connection needed"}
                  </p>
                  <h3>
                    {connection.state === "not_configured"
                      ? "Connect this desktop to an enrolled agent"
                      : "Reconnect the local Colossus agent"}
                  </h3>
                  <p>{connection.message}</p>
                  <p className="secure-note">
                    Credentials and privileged connection details stay in the
                    native process and are never exposed to this webview.
                  </p>
                  <button
                    className="button primary"
                    type="button"
                    disabled={connecting}
                    onClick={onConnect}
                  >
                    <IconRefresh size={16} stroke={1.8} aria-hidden="true" />
                    {connecting ? "Connecting…" : "Retry connection"}
                  </button>
                </div>
              </section>
            ) : view === undefined ? (
              <section className="work-welcome">
                <img src={colossusMark} alt="" />
                <p className="eyebrow">Local-first agent workspace</p>
                <h3>Give Colossus a goal. Keep control of every effect.</h3>
                <p>
                  Start a task, switch to plan mode, or coordinate specialist
                  work through one policy-bound local connection.
                </p>
                <div className="starter-list" aria-label="Example prompts">
                  {STARTERS.map((suggestion) => (
                    <button
                      type="button"
                      key={suggestion}
                      onClick={() => onSuggestion(suggestion)}
                    >
                      <IconSparkles size={17} stroke={1.6} aria-hidden="true" />
                      <span>{suggestion}</span>
                    </button>
                  ))}
                </div>
              </section>
            ) : (
              <>
                <div className="conversation-timeline" id="work-activity">
                  {conversationViews.map((conversationView) => (
                    <RunTimeline
                      view={conversationView}
                      key={conversationView.run.runId}
                    />
                  ))}
                </div>
                {view.streamState === "error" && view.streamError !== null ? (
                  <section className="stream-error" role="alert">
                    <div>
                      <strong>Live updates paused</strong>
                      <p>{view.streamError.message}</p>
                    </div>
                    <button
                      className="button secondary compact"
                      type="button"
                      onClick={onResume}
                    >
                      <IconRefresh size={15} stroke={1.8} aria-hidden="true" />
                      Resume
                    </button>
                  </section>
                ) : null}
              </>
            )}

            {runLoadError !== "" ? (
              <p className="page-error" role="alert">
                {runLoadError}
              </p>
            ) : null}
            {actionError !== null ? (
              <section className="page-error" role="alert">
                <strong>{actionError.message}</strong>
                {actionError.outcomeUnknown ? (
                  <span>Verify the external outcome before trying again.</span>
                ) : null}
              </section>
            ) : null}
          </div>
          <div className="work-composer-dock">
            {view !== undefined && view.pendingInteractions.length > 0 ? (
              <div
                className="pending-interaction-dock"
                aria-label="Required response"
              >
                {view.pendingInteractions.map((interaction) => (
                  <InteractionCard
                    key={interaction.interactionId}
                    interaction={interaction}
                    onRespond={onRespond}
                  />
                ))}
              </div>
            ) : null}
            {composer}
          </div>
        </section>

        {activeDrawer !== null && compactLayout ? (
          <button
            className="workspace-drawer-backdrop artifact-drawer-backdrop"
            type="button"
            aria-label={`Close ${activeDrawer} drawer`}
            aria-hidden="true"
            tabIndex={-1}
            onClick={closeDrawer}
          />
        ) : null}
        {filesAvailable || artifactsAvailable ? (
          <div
            ref={drawerRef}
            className={`artifact-drawer work-side-drawer${activeDrawer !== null ? " is-drawer-open" : ""}`}
            id="work-side-drawer"
            role={activeDrawer !== null && compactLayout ? "dialog" : undefined}
            aria-modal={
              activeDrawer !== null && compactLayout ? true : undefined
            }
            aria-label={
              activeDrawer === "files"
                ? "Workspace files"
                : activeDrawer === "artifacts"
                  ? "Artifact preview"
                  : undefined
            }
          >
            <button
              ref={drawerCloseRef}
              className="icon-button compact-drawer-close artifact-drawer-close"
              type="button"
              aria-label={`Close ${activeDrawer ?? "side"} drawer`}
              onClick={closeDrawer}
            >
              <IconX size={19} stroke={1.8} aria-hidden="true" />
            </button>
            <div
              className="work-drawer-panel"
              hidden={activeDrawer !== "artifacts"}
            >
              <ArtifactWorkspace artifacts={artifacts} />
            </div>
            {filesAvailable && filesDrawerMounted ? (
              <div
                className="work-drawer-panel"
                hidden={activeDrawer !== "files"}
              >
                {filesPanel}
              </div>
            ) : null}
          </div>
        ) : null}
      </div>
    </main>
  );
}
