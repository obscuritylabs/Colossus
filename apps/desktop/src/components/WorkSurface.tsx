import {
  IconArrowDown,
  IconClock,
  IconFiles,
  IconFolderOpen,
  IconLayoutSidebarRight,
  IconMenu2,
  IconMessageCirclePlus,
  IconBooks,
  IconPlayerStop,
  IconPlugConnected,
  IconRefresh,
  IconShieldLock,
  IconSparkles,
  IconX,
} from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties, ReactNode } from "react";

import colossusMark from "../assets/colossus-mark.svg";
import {
  MAX_ASIDE_PANE_WIDTH,
  MIN_ASIDE_PANE_WIDTH,
  clampAsidePaneWidth,
  clearStoredAsidePaneWidth,
  defaultAsidePaneWidth,
  readStoredAsidePaneWidth,
  storeAsidePaneWidth,
} from "../aside-pane-width";
import { isNearConversationLatest } from "../conversation-follow";
import { presentRunStatus, shortDateLabel } from "../presenters";
import {
  selectPlanForAutomaticDetails,
  selectSessionPlans,
  selectSessionSources,
} from "../session-resources";
import type { SessionPlanReference } from "../session-resources";
import type { RunView } from "../state";
import type {
  Aside,
  CommandError,
  ConnectionStatus,
  Interaction,
  InteractionAnswer,
  ListSessionActivityRequest,
  SessionMap,
  SessionMapResource,
  SessionActivityPage,
  ThreadDelegateInspection,
} from "../types";
import type { AsideDraft } from "./AsidePanel";
import { AsidePanel } from "./AsidePanel";
import type { AgentParticipant } from "./AgentFlow";
import type { ArtifactViewItem } from "./ArtifactWorkspace";
import { ArtifactWorkspace } from "./ArtifactWorkspace";
import { InteractionCard } from "./InteractionCard";
import { PlanDetailsPanel } from "./PlanDetailsPanel";
import { RunTimeline } from "./RunTimeline";
import { SessionActivityView } from "./SessionActivity";
import { ResearchSourcesPanel } from "./ResearchSourcesPanel";
import { SessionMapDetailsPanel } from "./SessionMapDetailsPanel";
import {
  SessionPlansView,
  SessionResourcesView,
  SessionSnapshotsView,
  SessionSourcesView,
  SessionTopology,
  SessionWorkspaceTabs,
} from "./SessionWorkspace";
import type { SessionWorkspaceView } from "./SessionWorkspace";
import { ThreadDetailsPanel } from "./ThreadDetailsPanel";

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
  sessionMap: SessionMap | null;
  sessionMapLoading: boolean;
  sessionMapError: string;
  selectedParticipantId: string | null;
  delegateView: RunView | undefined;
  delegateInspection: ThreadDelegateInspection | null;
  delegateLoading: boolean;
  delegateError: string;
  artifacts: readonly ArtifactViewItem[];
  selectedSpaceName: string;
  threadPinned: boolean;
  followRequestSequence: number;
  composer: ReactNode;
  filesPanel: ReactNode;
  filesAvailable: boolean;
  onOpenWorkspaceFile: (path: string) => void;
  artifactsAvailable: boolean;
  asideView: RunView | undefined;
  asideConversationViews: readonly RunView[];
  asideHistory: readonly Aside[];
  asideBusy: boolean;
  asideError: CommandError | null;
  asideReadOnly: boolean;
  planContinuationAvailable: boolean;
  planWorkflowAvailable: boolean;
  activityComparisonEnabled?: boolean;
  initialSessionWorkspaceView?: SessionWorkspaceView;
  onSessionWorkspaceViewChange?: (view: SessionWorkspaceView) => void;
  sessionActivityAvailable?: boolean;
  loadSessionActivity?: (
    request: ListSessionActivityRequest,
  ) => Promise<SessionActivityPage>;
  workNavigationOpen: boolean;
  onConnect: () => void;
  onCancel: () => void;
  onRespond: (
    interaction: Interaction,
    response: InteractionAnswer,
  ) => Promise<void>;
  onResume: () => void;
  onSuggestion: (suggestion: string) => void;
  onSelectParticipant: (participant: AgentParticipant) => void;
  onBackToThreadDetails: () => void;
  onSelectArtifact: (artifactId: string) => void;
  onOpenPlanWorkflow: (sessionId: string, planId: string) => void;
  onRevisePlan: (sourceRunId: string, planId: string, revision: number) => void;
  onExecutePlan: (
    sourceRunId: string,
    planId: string,
    revision: number,
    strategy: { type: "direct" } | { type: "goal"; maxIterations: number },
  ) => Promise<void>;
  onOpenWorkNavigation: () => void;
  onCloseWorkNavigation: () => void;
  onLoadAsides: (parentSessionId: string) => Promise<void>;
  onCreateAside: (prompt: string, draft: AsideDraft) => Promise<boolean>;
  onContinueAside: (prompt: string, view: RunView) => Promise<boolean>;
  onOpenAside: (aside: Aside) => Promise<void>;
  onNewAside: () => void;
  onRespondAside: (
    interaction: Interaction,
    response: InteractionAnswer,
  ) => Promise<void>;
  onCloseAside: (view: RunView | undefined) => Promise<boolean>;
}

const STARTERS = [
  "Orient yourself in this repo",
  "Plan a secure migration without making external changes",
  "Coordinate an implementation and security review",
];

const IGNORE_SESSION_WORKSPACE_VIEW = () => undefined;

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
  sessionMap,
  sessionMapLoading,
  sessionMapError,
  selectedParticipantId,
  delegateView,
  delegateInspection,
  delegateLoading,
  delegateError,
  artifacts,
  selectedSpaceName,
  threadPinned,
  followRequestSequence,
  composer,
  filesPanel,
  filesAvailable,
  onOpenWorkspaceFile,
  artifactsAvailable,
  asideView,
  asideConversationViews,
  asideHistory,
  asideBusy,
  asideError,
  asideReadOnly,
  planContinuationAvailable,
  planWorkflowAvailable,
  activityComparisonEnabled = false,
  initialSessionWorkspaceView = "conversation",
  onSessionWorkspaceViewChange = IGNORE_SESSION_WORKSPACE_VIEW,
  sessionActivityAvailable = false,
  loadSessionActivity = async () => ({
    activities: [],
    nextPageToken: "",
    headSequence: 0,
    projectedThroughSequence: 0,
    caughtUp: true,
  }),
  workNavigationOpen,
  onConnect,
  onCancel,
  onRespond,
  onResume,
  onSuggestion,
  onSelectParticipant,
  onBackToThreadDetails,
  onSelectArtifact,
  onOpenPlanWorkflow,
  onRevisePlan,
  onExecutePlan,
  onOpenWorkNavigation,
  onCloseWorkNavigation,
  onLoadAsides,
  onCreateAside,
  onContinueAside,
  onOpenAside,
  onNewAside,
  onRespondAside,
  onCloseAside,
}: WorkSurfaceProps) {
  const [sessionWorkspaceView, setSessionWorkspaceView] =
    useState<SessionWorkspaceView>(initialSessionWorkspaceView);
  const changeSessionWorkspaceView = useCallback(
    (next: SessionWorkspaceView) => {
      setSessionWorkspaceView(next);
      onSessionWorkspaceViewChange(next);
    },
    [onSessionWorkspaceViewChange],
  );
  const [selectedPlanId, setSelectedPlanId] = useState<string | null>(null);
  const [selectedSessionResource, setSelectedSessionResource] =
    useState<SessionMapResource | null>(null);
  const [followingLatest, setFollowingLatest] = useState(true);
  const followingLatestRef = useRef(true);
  const observedAutomaticPlanKeysRef = useRef<ReadonlySet<string>>(new Set());
  const [compactLayout, setCompactLayout] = useState(
    () => window.matchMedia("(max-width: 980px)").matches,
  );
  const [activeDrawer, setActiveDrawer] = useState<
    "files" | "artifacts" | "aside" | "research" | "details" | null
  >(() =>
    window.matchMedia("(min-width: 1200px)").matches ? "details" : null,
  );
  const [asideDraft, setAsideDraft] = useState<AsideDraft | null>(null);
  const [selectionLauncher, setSelectionLauncher] = useState<
    (AsideDraft & { left: number; top: number }) | null
  >(null);
  const [filesDrawerMounted, setFilesDrawerMounted] = useState(false);
  const [asidePaneWidth, setAsidePaneWidth] = useState<number | null>(
    readStoredAsidePaneWidth,
  );
  const workLayoutRef = useRef<HTMLDivElement>(null);
  const feedScrollRef = useRef<HTMLDivElement>(null);
  const stableFeedPositionRef = useRef({ top: 0, left: 0 });
  const stableFeedPositionTimerRef = useRef<number | null>(null);
  const drawerRef = useRef<HTMLDivElement>(null);
  const drawerCloseRef = useRef<HTMLButtonElement>(null);
  const filesTriggerRef = useRef<HTMLButtonElement>(null);
  const artifactTriggerRef = useRef<HTMLButtonElement>(null);
  const asideTriggerRef = useRef<HTMLButtonElement>(null);
  const researchTriggerRef = useRef<HTMLButtonElement>(null);
  const detailsTriggerRef = useRef<HTMLButtonElement>(null);
  const lastDrawerTriggerRef = useRef<HTMLButtonElement | null>(null);
  const workNavigationTriggerRef = useRef<HTMLButtonElement>(null);
  const previousWorkNavigationOpen = useRef(workNavigationOpen);
  const asideResizeRef = useRef<{
    pointerId: number;
    startX: number;
    startWidth: number;
    width: number;
  } | null>(null);
  const run = view?.run;
  const status = run === undefined ? null : presentRunStatus(run.status);
  const startedAt = run?.startedAt ?? run?.createdAt;
  const startedLabel =
    startedAt === undefined ? null : shortDateLabel(startedAt);
  const parentSessionId = run?.sessionId ?? null;
  const researchDrawerAvailable = run?.mode === "research";
  const researchOutput = view?.output ?? "";
  const resizableDrawer =
    activeDrawer === "aside" ||
    activeDrawer === "research" ||
    activeDrawer === "details";
  const selectedSessionId = run?.sessionId ?? null;

  function updateFollowingLatest(next: boolean) {
    followingLatestRef.current = next;
    setFollowingLatest(next);
  }

  function scrollToLatest(behavior: ScrollBehavior) {
    const feed = feedScrollRef.current;
    if (feed === null) {
      return;
    }
    updateFollowingLatest(true);
    feed.scrollTo({ top: feed.scrollHeight, behavior });
  }

  function scheduleStableFeedPosition(feed: HTMLDivElement) {
    if (stableFeedPositionTimerRef.current !== null) {
      window.clearTimeout(stableFeedPositionTimerRef.current);
    }
    stableFeedPositionTimerRef.current = window.setTimeout(() => {
      stableFeedPositionRef.current = {
        top: feed.scrollTop,
        left: feed.scrollLeft,
      };
      stableFeedPositionTimerRef.current = null;
    }, 100);
  }

  function restoreFeedPosition(position: { top: number; left: number }) {
    const restore = () => {
      feedScrollRef.current?.scrollTo(position);
      stableFeedPositionRef.current = position;
    };
    requestAnimationFrame(() => {
      restore();
      window.setTimeout(restore, 200);
    });
  }

  useEffect(() => {
    changeSessionWorkspaceView(initialSessionWorkspaceView);
    setSelectedPlanId(null);
    updateFollowingLatest(true);
  }, [
    changeSessionWorkspaceView,
    initialSessionWorkspaceView,
    selectedSessionId,
  ]);

  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      const feed = feedScrollRef.current;
      if (feed === null) {
        return;
      }
      if (sessionWorkspaceView === "conversation") {
        if (followingLatestRef.current) {
          feed.scrollTo({ top: feed.scrollHeight, behavior: "auto" });
        }
        return;
      }
      const position = { top: 0, left: 0 };
      feed.scrollTo({ ...position, behavior: "auto" });
      stableFeedPositionRef.current = position;
    });
    return () => cancelAnimationFrame(frame);
  }, [sessionWorkspaceView]);

  useEffect(() => {
    const frame = requestAnimationFrame(() => scrollToLatest("auto"));
    return () => cancelAnimationFrame(frame);
  }, [followRequestSequence, selectedSessionId]);

  useEffect(() => {
    if (
      sessionWorkspaceView !== "conversation" ||
      !followingLatestRef.current
    ) {
      return;
    }
    const frame = requestAnimationFrame(() => {
      if (followingLatestRef.current) {
        scrollToLatest("auto");
      }
    });
    return () => cancelAnimationFrame(frame);
  }, [conversationViews, sessionWorkspaceView]);

  useEffect(
    () => () => {
      if (stableFeedPositionTimerRef.current !== null) {
        window.clearTimeout(stableFeedPositionTimerRef.current);
      }
    },
    [],
  );

  const sessionPlans = useMemo(
    () => selectSessionPlans(conversationViews),
    [conversationViews],
  );
  const selectedPlan = useMemo(
    () => sessionPlans.find((plan) => plan.planId === selectedPlanId) ?? null,
    [selectedPlanId, sessionPlans],
  );
  const sessionResourceCounts = useMemo(() => {
    const sourceCount = selectSessionSources(conversationViews).length;
    const durableMapCount =
      sessionMap === null
        ? 0
        : sessionMap.delegates.length +
          sessionMap.goals.length +
          sessionMap.tasks.length +
          sessionMap.decisions.length +
          sessionMap.memories.length +
          sessionMap.contextSnapshots.length +
          sessionMap.researchRuns.length;
    return {
      planCount: sessionPlans.length,
      resourceCount:
        sessionPlans.length + sourceCount + artifacts.length + durableMapCount,
      snapshotCount: sessionMap?.contextSnapshots.length ?? 0,
      sourceCount,
    };
  }, [artifacts.length, conversationViews, sessionMap, sessionPlans.length]);

  useEffect(() => {
    if (selectedSessionId === null) {
      return;
    }
    const selection = selectPlanForAutomaticDetails(
      selectedSessionId,
      sessionPlans,
      observedAutomaticPlanKeysRef.current,
    );
    observedAutomaticPlanKeysRef.current = selection.observedKeys;
    if (selection.plan === null) {
      return;
    }
    onCloseWorkNavigation();
    lastDrawerTriggerRef.current = detailsTriggerRef.current;
    onBackToThreadDetails();
    setSelectedPlanId(selection.plan.planId);
    setActiveDrawer("details");
  }, [
    onBackToThreadDetails,
    onCloseWorkNavigation,
    selectedSessionId,
    sessionPlans,
  ]);

  useEffect(() => {
    const media = window.matchMedia("(max-width: 980px)");
    const onChange = (event: MediaQueryListEvent) => {
      setCompactLayout(event.matches);
      if (!event.matches && !resizableDrawer) {
        setActiveDrawer(null);
        onCloseWorkNavigation();
      }
    };
    setCompactLayout(media.matches);
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, [activeDrawer, onCloseWorkNavigation, resizableDrawer]);

  useEffect(() => {
    if (previousWorkNavigationOpen.current && !workNavigationOpen) {
      requestAnimationFrame(() => workNavigationTriggerRef.current?.focus());
    }
    previousWorkNavigationOpen.current = workNavigationOpen;
    if (workNavigationOpen) {
      if (resizableDrawer) {
        onCloseWorkNavigation();
        return;
      }
      setActiveDrawer(null);
    }
  }, [
    activeDrawer,
    onCloseWorkNavigation,
    resizableDrawer,
    workNavigationOpen,
  ]);

  useEffect(() => {
    if (!resizableDrawer || compactLayout) {
      return;
    }
    const layout = workLayoutRef.current;
    if (layout === null) {
      return;
    }
    const observedLayout = layout;
    function fitAsideToLayout() {
      const layoutWidth = observedLayout.getBoundingClientRect().width;
      if (layoutWidth <= 0) {
        return;
      }
      setAsidePaneWidth((current) =>
        clampAsidePaneWidth(
          current ??
            (activeDrawer === "details"
              ? 320
              : defaultAsidePaneWidth(layoutWidth)),
          layoutWidth,
        ),
      );
    }
    fitAsideToLayout();
    if (typeof ResizeObserver === "undefined") {
      return;
    }
    const observer = new ResizeObserver(fitAsideToLayout);
    observer.observe(observedLayout);
    return () => observer.disconnect();
  }, [activeDrawer, compactLayout, resizableDrawer]);

  useEffect(() => {
    if (
      (activeDrawer === "files" && !filesAvailable) ||
      (activeDrawer === "artifacts" && !artifactsAvailable) ||
      (activeDrawer === "research" && !researchDrawerAvailable) ||
      (activeDrawer === "details" && run === undefined)
    ) {
      setActiveDrawer(null);
    }
  }, [
    activeDrawer,
    artifactsAvailable,
    filesAvailable,
    researchDrawerAvailable,
    run,
  ]);

  useEffect(() => {
    if (activeDrawer === null || !compactLayout) {
      return;
    }
    const obscured = [
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
        if (activeDrawer === "aside") {
          return;
        }
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

  useEffect(() => {
    function captureSelection() {
      const selection = window.getSelection();
      const raw = selection?.toString().trim() ?? "";
      if (selection === null || selection.rangeCount === 0 || raw === "") {
        setSelectionLauncher(null);
        return;
      }
      const node = selection.anchorNode;
      const element =
        node instanceof Element ? node : (node?.parentElement ?? null);
      const selectable = element?.closest<HTMLElement>(
        "[data-aside-selectable='true']",
      );
      const focusNode = selection.focusNode;
      const focusElement =
        focusNode instanceof Element
          ? focusNode
          : (focusNode?.parentElement ?? null);
      const focusSelectable = focusElement?.closest<HTMLElement>(
        "[data-aside-selectable='true']",
      );
      const context = selectable?.closest<HTMLElement>("[data-aside-context]");
      const sourceRunId = context?.dataset.asideSourceRunId;
      if (
        selectable === null ||
        focusSelectable !== selectable ||
        sourceRunId === undefined
      ) {
        setSelectionLauncher(null);
        return;
      }
      const quote = new TextEncoder().encode(raw).slice(0, 4096);
      const boundedQuote = new TextDecoder().decode(quote).trim();
      const rect = selection.getRangeAt(0).getBoundingClientRect();
      if (boundedQuote === "" || rect.width === 0 || rect.height === 0) {
        setSelectionLauncher(null);
        return;
      }
      setSelectionLauncher({
        sourceRunId,
        quote: boundedQuote,
        left: Math.min(
          window.innerWidth - 150,
          Math.max(12, rect.left + rect.width / 2 - 62),
        ),
        top: Math.max(12, rect.top - 42),
      });
    }
    document.addEventListener("mouseup", captureSelection);
    document.addEventListener("keyup", captureSelection);
    return () => {
      document.removeEventListener("mouseup", captureSelection);
      document.removeEventListener("keyup", captureSelection);
    };
  }, []);

  function closeDrawer() {
    const trigger = lastDrawerTriggerRef.current;
    if (activeDrawer === "details") {
      setSelectedPlanId(null);
      setSelectedSessionResource(null);
      onBackToThreadDetails();
    }
    setActiveDrawer(null);
    requestAnimationFrame(() => trigger?.focus());
  }

  function openSessionViewFromDetails(view: SessionWorkspaceView) {
    setSessionWorkspaceView(view);
    closeDrawer();
  }

  function toggleDrawer(
    drawer: "files" | "artifacts" | "aside" | "research" | "details",
  ) {
    const trigger =
      drawer === "files"
        ? filesTriggerRef.current
        : drawer === "artifacts"
          ? artifactTriggerRef.current
          : drawer === "aside"
            ? asideTriggerRef.current
            : drawer === "research"
              ? researchTriggerRef.current
              : detailsTriggerRef.current;
    lastDrawerTriggerRef.current = trigger;
    if (activeDrawer === drawer) {
      if (drawer === "aside") {
        return;
      }
      closeDrawer();
      return;
    }
    if (activeDrawer === "details") {
      setSelectedPlanId(null);
      setSelectedSessionResource(null);
      onBackToThreadDetails();
    }
    onCloseWorkNavigation();
    if (drawer === "files") {
      setFilesDrawerMounted(true);
    } else if (drawer === "artifacts" && artifacts[0] !== undefined) {
      onSelectArtifact(artifacts[0].id);
    }
    if (drawer === "aside" && run !== undefined) {
      setAsideDraft({
        sourceRunId: run.runId,
        quote: "",
      });
      void onLoadAsides(run.sessionId);
    }
    setActiveDrawer(drawer);
  }

  function openWorkspaceSource(path: string) {
    onCloseWorkNavigation();
    lastDrawerTriggerRef.current = filesTriggerRef.current;
    setFilesDrawerMounted(true);
    setActiveDrawer("files");
    onOpenWorkspaceFile(path);
  }

  function openResearchDrawer() {
    onCloseWorkNavigation();
    lastDrawerTriggerRef.current = researchTriggerRef.current;
    setActiveDrawer("research");
  }

  function openParticipantInDetails(participant: AgentParticipant) {
    const feedPosition = stableFeedPositionRef.current;
    onCloseWorkNavigation();
    lastDrawerTriggerRef.current = detailsTriggerRef.current;
    setSelectedPlanId(null);
    setSelectedSessionResource(null);
    setActiveDrawer("details");
    onSelectParticipant(participant);
    restoreFeedPosition(feedPosition);
  }

  function openPlanInDetails(plan: SessionPlanReference) {
    onCloseWorkNavigation();
    lastDrawerTriggerRef.current = detailsTriggerRef.current;
    onBackToThreadDetails();
    setSelectedSessionResource(null);
    setSelectedPlanId(plan.planId);
    setActiveDrawer("details");
  }

  function openConversationPlanInDetails(_sourceRunId: string, planId: string) {
    const plan = sessionPlans.find((candidate) => candidate.planId === planId);
    if (plan !== undefined) {
      openPlanInDetails(plan);
    }
  }

  function openArtifactFromResources(artifactId: string) {
    onCloseWorkNavigation();
    lastDrawerTriggerRef.current = artifactTriggerRef.current;
    onSelectArtifact(artifactId);
    setActiveDrawer("artifacts");
  }

  function openSessionResource(resource: SessionMapResource) {
    if (resource.family === "delegates") {
      const delegate = resource.value;
      const state: AgentParticipant["state"] =
        delegate.status === "running"
          ? "working"
          : delegate.status === "queued"
            ? "waiting"
            : delegate.status === "interrupted"
              ? "failed"
              : delegate.status;
      const participant: AgentParticipant = {
        id: delegate.jobId,
        name: "Delegated agent",
        role: delegate.task,
        state,
        icon: "builder",
        kind: "delegate",
        parentRunId: delegate.parentRunId,
        childSessionId: delegate.childSessionId,
        modelRole: delegate.role,
        task: delegate.task,
        finalOutput: delegate.finalOutput,
        error: delegate.error,
        createdAt: delegate.createdAt,
        updatedAt: delegate.updatedAt,
        ...(delegate.childRunId === undefined
          ? {}
          : { childRunId: delegate.childRunId }),
        ...(delegate.startedAt === undefined
          ? {}
          : { startedAt: delegate.startedAt }),
        ...(delegate.completedAt === undefined
          ? {}
          : { completedAt: delegate.completedAt }),
      };
      openParticipantInDetails(participant);
      return;
    }
    const feedPosition = stableFeedPositionRef.current;
    onCloseWorkNavigation();
    onBackToThreadDetails();
    lastDrawerTriggerRef.current = detailsTriggerRef.current;
    setSelectedPlanId(null);
    setSelectedSessionResource(resource);
    setActiveDrawer("details");
    restoreFeedPosition(feedPosition);
  }

  function previewAsideWidth(width: number): number {
    const layoutWidth = workLayoutRef.current?.getBoundingClientRect().width;
    if (layoutWidth === undefined || layoutWidth <= 0) {
      return Math.round(width);
    }
    const nextWidth = clampAsidePaneWidth(width, layoutWidth);
    setAsidePaneWidth(nextWidth);
    return nextWidth;
  }

  function commitAsideWidth(width: number) {
    const nextWidth = previewAsideWidth(width);
    storeAsidePaneWidth(nextWidth);
  }

  function finishAsideResize(pointerId: number, handle: HTMLElement) {
    const resize = asideResizeRef.current;
    if (resize === null || resize.pointerId !== pointerId) {
      return;
    }
    asideResizeRef.current = null;
    if (handle.hasPointerCapture(pointerId)) {
      handle.releasePointerCapture(pointerId);
    }
    commitAsideWidth(resize.width);
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
              <span>
                {run.mode === "plan"
                  ? "Plan mode"
                  : run.mode === "research"
                    ? "Research mode"
                    : "Execute mode"}
              </span>
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
          {run !== undefined ? (
            <button
              ref={detailsTriggerRef}
              className="button secondary compact thread-details-open-button"
              type="button"
              aria-label={`${activeDrawer === "details" ? "Close" : "Open"} thread details`}
              aria-controls="work-side-drawer"
              aria-expanded={activeDrawer === "details"}
              onClick={() => toggleDrawer("details")}
            >
              <IconLayoutSidebarRight
                size={15}
                stroke={1.7}
                aria-hidden="true"
              />
              <span className="compact-action-copy">Details</span>
            </button>
          ) : null}
          {researchDrawerAvailable ? (
            <button
              ref={researchTriggerRef}
              className="button secondary compact research-open-button"
              type="button"
              aria-label="Open Research sources"
              aria-controls="work-side-drawer"
              aria-expanded={activeDrawer === "research"}
              onClick={() => toggleDrawer("research")}
            >
              <IconBooks size={15} stroke={1.7} aria-hidden="true" />
              <span className="compact-action-copy">Sources</span>
            </button>
          ) : null}
          {run !== undefined ? (
            <button
              ref={asideTriggerRef}
              className="button secondary compact aside-open-button"
              type="button"
              aria-label="Open Aside"
              aria-controls="work-side-drawer"
              aria-expanded={activeDrawer === "aside"}
              onClick={() => toggleDrawer("aside")}
            >
              <IconMessageCirclePlus
                size={15}
                stroke={1.7}
                aria-hidden="true"
              />
              <span className="compact-action-copy">Aside</span>
            </button>
          ) : null}
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

      {view === undefined ? null : (
        <SessionWorkspaceTabs
          active={sessionWorkspaceView}
          onChange={changeSessionWorkspaceView}
        />
      )}

      {selectionLauncher !== null ? (
        <button
          className="aside-selection-launcher"
          type="button"
          style={{ left: selectionLauncher.left, top: selectionLauncher.top }}
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => {
            const { left: _left, top: _top, ...draft } = selectionLauncher;
            setAsideDraft(draft);
            setSelectionLauncher(null);
            window.getSelection()?.removeAllRanges();
            onCloseWorkNavigation();
            if (parentSessionId !== null) {
              void onLoadAsides(parentSessionId);
            }
            setActiveDrawer("aside");
          }}
        >
          Ask in Aside
        </button>
      ) : null}

      <div
        ref={workLayoutRef}
        className={`work-layout${activeDrawer !== null ? " is-work-drawer-open" : ""}${resizableDrawer ? " is-aside-open" : ""}`}
        style={
          asidePaneWidth === null
            ? undefined
            : ({
                "--aside-pane-width": `${asidePaneWidth}px`,
              } as CSSProperties)
        }
      >
        <section
          className={`work-thread${sessionWorkspaceView === "topology" ? " is-topology-view" : ""}${sessionWorkspaceView === "activity" ? " is-activity-view" : ""}`}
          aria-label="Work conversation"
        >
          <div className="work-feed-frame">
            <div
              ref={feedScrollRef}
              className="work-feed-scroll"
              onScroll={(event) => {
                updateFollowingLatest(
                  isNearConversationLatest(event.currentTarget),
                );
                scheduleStableFeedPosition(event.currentTarget);
              }}
            >
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
                        <IconSparkles
                          size={17}
                          stroke={1.6}
                          aria-hidden="true"
                        />
                        <span>{suggestion}</span>
                      </button>
                    ))}
                  </div>
                </section>
              ) : sessionWorkspaceView === "topology" ? (
                <SessionTopology
                  views={conversationViews}
                  participants={participants}
                  sessionMap={sessionMap}
                  loading={sessionMapLoading}
                  error={sessionMapError}
                  artifacts={artifacts}
                  onSelectResource={openSessionResource}
                  onSelectArtifact={openArtifactFromResources}
                />
              ) : sessionWorkspaceView === "activity" ? (
                <SessionActivityView
                  sourceRunId={view.run.runId}
                  available={sessionActivityAvailable}
                  loadPage={loadSessionActivity}
                />
              ) : sessionWorkspaceView === "plans" ? (
                <SessionPlansView
                  views={conversationViews}
                  workflowAvailable={planWorkflowAvailable}
                  onInspectPlan={openPlanInDetails}
                  onOpenPlanWorkflow={onOpenPlanWorkflow}
                  onRevisePlan={onRevisePlan}
                />
              ) : sessionWorkspaceView === "snapshots" ? (
                <SessionSnapshotsView
                  sessionMap={sessionMap}
                  loading={sessionMapLoading}
                  error={sessionMapError}
                  onSelectResource={openSessionResource}
                />
              ) : sessionWorkspaceView === "sources" ? (
                <SessionSourcesView
                  views={conversationViews}
                  onOpenWorkspaceFile={openWorkspaceSource}
                />
              ) : sessionWorkspaceView === "resources" ? (
                <SessionResourcesView
                  views={conversationViews}
                  artifacts={artifacts}
                  sessionMap={sessionMap}
                  loading={sessionMapLoading}
                  error={sessionMapError}
                  onChangeView={changeSessionWorkspaceView}
                  onSelectArtifact={openArtifactFromResources}
                  onSelectResource={openSessionResource}
                />
              ) : (
                <>
                  <div className="conversation-timeline" id="work-activity">
                    {conversationViews.map((conversationView) => (
                      <div
                        data-aside-context="true"
                        data-aside-source-run-id={conversationView.run.runId}
                        key={conversationView.run.runId}
                      >
                        <RunTimeline
                          view={conversationView}
                          activityComparison={activityComparisonEnabled}
                          planContinuationAvailable={planContinuationAvailable}
                          planWorkflowAvailable={planWorkflowAvailable}
                          onInspectPlan={openConversationPlanInDetails}
                          onOpenPlanWorkflow={onOpenPlanWorkflow}
                          onRevisePlan={onRevisePlan}
                          onExecutePlan={onExecutePlan}
                          onOpenResearchSources={openResearchDrawer}
                        />
                      </div>
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
                        <IconRefresh
                          size={15}
                          stroke={1.8}
                          aria-hidden="true"
                        />
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
                    <span>
                      Verify the external outcome before trying again.
                    </span>
                  ) : null}
                </section>
              ) : null}
            </div>
            {!followingLatest &&
            view !== undefined &&
            sessionWorkspaceView === "conversation" ? (
              <button
                className="jump-to-latest"
                type="button"
                onClick={() => scrollToLatest("auto")}
              >
                <IconArrowDown size={15} stroke={2} aria-hidden="true" />
                Jump to latest
              </button>
            ) : null}
          </div>
          {sessionWorkspaceView === "topology" ||
          sessionWorkspaceView === "activity" ? null : (
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
          )}
        </section>

        {resizableDrawer && !compactLayout ? (
          <div
            className="aside-resize-handle"
            role="separator"
            aria-label={
              activeDrawer === "aside"
                ? "Resize Aside conversation"
                : activeDrawer === "research"
                  ? "Resize Research sources"
                  : "Resize Thread details"
            }
            aria-orientation="vertical"
            aria-valuemin={MIN_ASIDE_PANE_WIDTH}
            aria-valuemax={MAX_ASIDE_PANE_WIDTH}
            aria-valuenow={asidePaneWidth ?? MIN_ASIDE_PANE_WIDTH}
            title="Drag to resize. Double-click to reset."
            tabIndex={0}
            onPointerDown={(event) => {
              if (event.button !== 0 || drawerRef.current === null) {
                return;
              }
              event.preventDefault();
              const startWidth =
                drawerRef.current.getBoundingClientRect().width;
              asideResizeRef.current = {
                pointerId: event.pointerId,
                startX: event.clientX,
                startWidth,
                width: startWidth,
              };
              event.currentTarget.setPointerCapture(event.pointerId);
            }}
            onPointerMove={(event) => {
              const resize = asideResizeRef.current;
              if (resize === null || resize.pointerId !== event.pointerId) {
                return;
              }
              resize.width = previewAsideWidth(
                resize.startWidth - (event.clientX - resize.startX),
              );
            }}
            onPointerUp={(event) =>
              finishAsideResize(event.pointerId, event.currentTarget)
            }
            onPointerCancel={(event) =>
              finishAsideResize(event.pointerId, event.currentTarget)
            }
            onKeyDown={(event) => {
              const layoutWidth =
                workLayoutRef.current?.getBoundingClientRect().width;
              if (layoutWidth === undefined || layoutWidth <= 0) {
                return;
              }
              const currentWidth =
                drawerRef.current?.getBoundingClientRect().width ??
                asidePaneWidth ??
                defaultAsidePaneWidth(layoutWidth);
              const increment = event.shiftKey ? 24 : 8;
              let nextWidth: number | null = null;
              if (event.key === "ArrowLeft") {
                nextWidth = currentWidth + increment;
              } else if (event.key === "ArrowRight") {
                nextWidth = currentWidth - increment;
              } else if (event.key === "Home") {
                nextWidth = MIN_ASIDE_PANE_WIDTH;
              } else if (event.key === "End") {
                nextWidth = MAX_ASIDE_PANE_WIDTH;
              }
              if (nextWidth !== null) {
                event.preventDefault();
                commitAsideWidth(nextWidth);
              }
            }}
            onDoubleClick={() => {
              const layoutWidth =
                workLayoutRef.current?.getBoundingClientRect().width;
              if (layoutWidth === undefined || layoutWidth <= 0) {
                return;
              }
              clearStoredAsidePaneWidth();
              setAsidePaneWidth(
                activeDrawer === "details"
                  ? clampAsidePaneWidth(320, layoutWidth)
                  : defaultAsidePaneWidth(layoutWidth),
              );
            }}
          />
        ) : null}

        {activeDrawer !== null && activeDrawer !== "aside" && compactLayout ? (
          <button
            className="workspace-drawer-backdrop artifact-drawer-backdrop"
            type="button"
            aria-label={`Close ${activeDrawer} drawer`}
            aria-hidden="true"
            tabIndex={-1}
            onClick={closeDrawer}
          />
        ) : null}
        {filesAvailable ||
        artifactsAvailable ||
        run !== undefined ||
        researchDrawerAvailable ? (
          <div
            ref={drawerRef}
            className={`artifact-drawer work-side-drawer${activeDrawer !== null ? " is-drawer-open" : ""}${resizableDrawer ? " is-aside-open" : ""}`}
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
                  : activeDrawer === "aside"
                    ? "Aside conversation"
                    : activeDrawer === "research"
                      ? "Research sources"
                      : activeDrawer === "details"
                        ? "Thread details"
                        : undefined
            }
          >
            {activeDrawer !== "aside" ? (
              <button
                ref={drawerCloseRef}
                className="icon-button compact-drawer-close artifact-drawer-close"
                type="button"
                aria-label={`Close ${activeDrawer ?? "side"} drawer`}
                onClick={closeDrawer}
              >
                <IconX size={19} stroke={1.8} aria-hidden="true" />
              </button>
            ) : null}
            <div
              className="work-drawer-panel"
              hidden={activeDrawer !== "artifacts"}
            >
              <ArtifactWorkspace
                artifacts={artifacts}
                onSelect={onSelectArtifact}
              />
            </div>
            {filesAvailable && filesDrawerMounted ? (
              <div
                className="work-drawer-panel"
                hidden={activeDrawer !== "files"}
              >
                {filesPanel}
              </div>
            ) : null}
            <div
              className="work-drawer-panel"
              hidden={activeDrawer !== "research"}
            >
              <ResearchSourcesPanel
                output={researchOutput}
                onOpenWorkspaceFile={openWorkspaceSource}
                running={
                  run?.status === "queued" ||
                  run?.status === "running" ||
                  run?.status === "waiting" ||
                  run?.status === "cancelling"
                }
              />
            </div>
            {run !== undefined ? (
              <div
                className="work-drawer-panel"
                hidden={activeDrawer !== "details"}
              >
                {selectedSessionResource !== null ? (
                  <SessionMapDetailsPanel
                    resource={selectedSessionResource}
                    spaceName={selectedSpaceName}
                    onBack={() => setSelectedSessionResource(null)}
                  />
                ) : selectedPlan === null ? (
                  <ThreadDetailsPanel
                    run={run}
                    spaceName={selectedSpaceName}
                    pinned={threadPinned}
                    participants={participants}
                    files={artifacts}
                    selectedParticipantId={selectedParticipantId}
                    delegateView={delegateView}
                    delegateInspection={delegateInspection}
                    delegateLoading={delegateLoading}
                    delegateError={delegateError}
                    sessionRunCount={conversationViews.length}
                    sessionPlanCount={sessionResourceCounts.planCount}
                    sessionResourceCount={sessionResourceCounts.resourceCount}
                    sessionSnapshotCount={sessionResourceCounts.snapshotCount}
                    sessionSourceCount={sessionResourceCounts.sourceCount}
                    onSelectParticipant={onSelectParticipant}
                    onBackToThread={onBackToThreadDetails}
                    onOpenSessionView={openSessionViewFromDetails}
                  />
                ) : (
                  <PlanDetailsPanel
                    plan={selectedPlan}
                    sessionId={run.sessionId}
                    workflowAvailable={planWorkflowAvailable}
                    onBack={() => setSelectedPlanId(null)}
                    onRevise={onRevisePlan}
                    onOpenWorkflow={onOpenPlanWorkflow}
                  />
                )}
              </div>
            ) : null}
            <div
              className="work-drawer-panel"
              hidden={activeDrawer !== "aside"}
            >
              <AsidePanel
                draft={asideDraft}
                view={asideView}
                conversationViews={asideConversationViews}
                history={asideHistory}
                busy={asideBusy}
                error={asideError}
                readOnly={asideReadOnly}
                onCreate={onCreateAside}
                onContinue={onContinueAside}
                onOpen={onOpenAside}
                onNew={() => {
                  if (run !== undefined) {
                    setAsideDraft({
                      sourceRunId: run.runId,
                      quote: "",
                    });
                  }
                  onNewAside();
                }}
                onRespond={onRespondAside}
                onClose={onCloseAside}
                onDismiss={closeDrawer}
              />
            </div>
          </div>
        ) : null}
      </div>
    </main>
  );
}
