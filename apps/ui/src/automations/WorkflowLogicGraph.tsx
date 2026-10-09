import {
  Background,
  BackgroundVariant,
  Handle,
  Position,
  ReactFlow,
  type NodeProps,
  type NodeTypes,
  type ReactFlowInstance,
  type Edge,
  type FitViewOptions,
} from "@xyflow/react";
import {
  IconArrowsSplit,
  IconCheck,
  IconCircle,
  IconGitBranch,
  IconPlayerPlay,
  IconRepeat,
  IconShieldCheck,
  IconSparkles,
  IconTools,
  IconZoomIn,
  IconZoomOut,
  IconFocus2,
} from "@tabler/icons-react";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { WorkflowLogic, WorkflowRun } from "./types";
import { DropdownSelect } from "../components/DropdownSelect";
import {
  buildWorkflowGraph,
  logicSteps,
  stepKindLabel,
  STEP_HEIGHT,
  STEP_WIDTH,
  type LogicNode,
} from "./workflow-graph";
import "@xyflow/react/dist/style.css";

const ICONS = {
  agent: IconSparkles,
  tool: IconTools,
  workflow: IconGitBranch,
  approval: IconShieldCheck,
  condition: IconGitBranch,
  parallel: IconArrowsSplit,
  foreach: IconRepeat,
  wait_for_input: IconPlayerPlay,
  emit: IconCheck,
};
function Handles({ loop = false }: { loop?: boolean }) {
  return (
    <>
      <Handle type="target" position={Position.Top} isConnectable={false} />
      <Handle type="source" position={Position.Bottom} isConnectable={false} />
      {loop && (
        <>
          <Handle
            id="bypass-in"
            type="target"
            position={Position.Left}
            isConnectable={false}
          />
          <Handle
            id="bypass-out"
            type="source"
            position={Position.Left}
            isConnectable={false}
          />
          <Handle
            id="loop-in"
            type="target"
            position={Position.Right}
            isConnectable={false}
          />
          <Handle
            id="loop-out"
            type="source"
            position={Position.Right}
            isConnectable={false}
          />
        </>
      )}
    </>
  );
}
const StepNode = memo(function StepNode({ data }: NodeProps<LogicNode>) {
  const step = data.step!;
  const Icon = ICONS[step.kind];
  return (
    <>
      <Handles loop />
      <button
        className={`workflow-flow-step kind-${step.kind} state-${data.state?.status ?? "unobserved"}`}
        aria-label={`${step.id}, ${stepKindLabel(step.kind)}${data.state ? `, ${data.state.status}` : ""}`}
        aria-pressed={data.selected ?? false}
        onClick={() => data.onSelect?.(step.id)}
        onFocus={(event) => {
          if (event.currentTarget.matches(":focus-visible"))
            data.onFocus?.(step.id);
        }}
      >
        <span className="workflow-flow-kind">
          <Icon size={15} aria-hidden="true" />
          {stepKindLabel(step.kind)}
          {data.state && (
            <span className="workflow-flow-state">{data.state.status}</span>
          )}
        </span>
        <strong>{data.label}</strong>
        <small>{step.summary}</small>
      </button>
    </>
  );
});
const BoundaryNode = memo(function BoundaryNode({
  data,
}: NodeProps<LogicNode>) {
  return (
    <>
      <Handles />
      <div className="workflow-flow-boundary">{data.label}</div>
    </>
  );
});
const JoinNode = memo(function JoinNode() {
  return (
    <>
      <Handles loop />
      <div className="workflow-flow-join" aria-hidden="true" />
    </>
  );
});
const NODE_TYPES = {
  workflowStep: StepNode,
  workflowBoundary: BoundaryNode,
  workflowJoin: JoinNode,
} as NodeTypes;
const FIT: FitViewOptions<LogicNode> = {
  padding: { top: "72px", bottom: "48px", left: "16px", right: "16px" },
  minZoom: 0.001,
  maxZoom: 1,
};

export function WorkflowLogicGraph({
  logic,
  run,
}: {
  logic: WorkflowLogic;
  run?: WorkflowRun | null;
}) {
  const flow = useRef<ReactFlowInstance<LogicNode, Edge> | null>(null);
  const canvas = useRef<HTMLDivElement>(null);
  const steps = useMemo(() => logicSteps(logic), [logic]);
  const [selectedId, setSelectedId] = useState(
    () =>
      steps.find((step) => step.kind === "condition")?.id ?? steps[0]?.id ?? "",
  );
  const structure = useMemo(() => buildWorkflowGraph(logic), [logic]);
  const selected = steps.find((step) => step.id === selectedId);
  const states = useMemo(
    () => new Map((run?.step_states ?? []).map((step) => [step.step_id, step])),
    [run],
  );
  const focusStep = useCallback(
    (id: string) => {
      setSelectedId(id);
      const node = structure.nodes.find((node) => node.data.step?.id === id);
      if (!node || !flow.current || !canvas.current) return;
      const { x, y, zoom } = flow.current.getViewport();
      const left = node.position.x * zoom + x;
      const top = node.position.y * zoom + y;
      if (
        left < 12 ||
        left + STEP_WIDTH * zoom > canvas.current.clientWidth - 12 ||
        top < 64 ||
        top + STEP_HEIGHT * zoom > canvas.current.clientHeight - 48
      ) {
        void flow.current.setCenter(
          node.position.x + STEP_WIDTH / 2,
          node.position.y + STEP_HEIGHT / 2,
          { zoom: Math.max(zoom, 0.85), duration: 0 },
        );
      }
    },
    [structure],
  );
  const nodes = useMemo(
    () =>
      structure.nodes.map((node) => ({
        ...node,
        data: {
          ...node.data,
          state: states.get(node.data.step?.id ?? ""),
          selected: node.data.step?.id === selectedId,
          onSelect: setSelectedId,
          onFocus: focusStep,
        },
      })),
    [structure, states, selectedId, focusStep],
  );
  const fit = useCallback(() => {
    void flow.current?.fitView(FIT);
  }, []);
  useEffect(() => {
    const observer = new ResizeObserver(fit);
    if (canvas.current) observer.observe(canvas.current);
    return () => observer.disconnect();
  }, [fit]);
  const navigate = (id: string) => {
    setSelectedId(id);
    const node = nodes.find((node) => node.data.step?.id === id);
    if (node)
      void flow.current?.setCenter(
        node.position.x + STEP_WIDTH / 2,
        node.position.y + STEP_HEIGHT / 2,
        { zoom: 1, duration: 0 },
      );
  };
  const current = selected ? states.get(selected.id) : undefined;
  return (
    <div className="workflow-logic-workspace">
      <div className="workflow-logic-canvas" ref={canvas}>
        <div
          className="workflow-logic-toolbar"
          role="group"
          aria-label="Workflow graph view"
        >
          <button
            type="button"
            onClick={() => void flow.current?.zoomIn()}
            aria-label="Zoom in"
          >
            <IconZoomIn size={18} />
          </button>
          <button
            type="button"
            onClick={() => void flow.current?.zoomOut()}
            aria-label="Zoom out"
          >
            <IconZoomOut size={18} />
          </button>
          <button type="button" onClick={fit}>
            <IconFocus2 size={18} />
            Fit workflow
          </button>
        </div>
        <ReactFlow<LogicNode, Edge>
          nodes={nodes}
          edges={structure.edges}
          nodeTypes={NODE_TYPES}
          onInit={(instance) => {
            flow.current = instance;
            void instance.fitView(FIT);
          }}
          onNodeClick={() => undefined}
          fitView
          fitViewOptions={FIT}
          minZoom={0.001}
          maxZoom={1.8}
          nodesDraggable={false}
          nodesConnectable={false}
          nodesFocusable={false}
          edgesFocusable={false}
          elementsSelectable={false}
          panOnDrag
          zoomOnScroll={false}
          zoomOnDoubleClick={false}
          preventScrolling={false}
          aria-label="Workflow logic graph"
        >
          <Background
            variant={BackgroundVariant.Dots}
            gap={20}
            size={1}
            color="var(--border)"
          />
        </ReactFlow>
        <span className="workflow-logic-pan-hint">
          Drag to pan · Use zoom to inspect
        </span>
      </div>
      <aside
        className="workflow-logic-inspector"
        aria-label="Workflow step details"
      >
        <span className="workflow-logic-eyebrow">STEP DETAILS</span>
        <label>
          Choose a step
          <DropdownSelect
            aria-label="Choose a step"
            value={selectedId}
            onChange={(event) => navigate(event.target.value)}
          >
            {steps.map((step) => (
              <option key={step.id} value={step.id}>
                {step.id} · {stepKindLabel(step.kind)}
              </option>
            ))}
          </DropdownSelect>
        </label>
        {selected && (
          <>
            <h4>{selected.id.replaceAll(/[-_]/g, " ")}</h4>
            <span className="workflow-logic-type">
              {stepKindLabel(selected.kind)}
            </span>
            <p className="workflow-logic-step-summary">{selected.summary}</p>
            {selected.branches.length > 0 && (
              <div className="workflow-logic-branches">
                {selected.branches.map((branch) => (
                  <p key={branch.label}>
                    <strong>{branch.label}</strong>
                    <span>
                      {branch.steps.length
                        ? branch.steps.map((step) => step.id).join(" → ")
                        : "Continue without steps"}
                    </span>
                  </p>
                ))}
              </div>
            )}
            {run && (
              <div
                className={`workflow-logic-execution state-${current?.status ?? "unobserved"}`}
              >
                <strong>Recorded execution</strong>
                <p>{current?.status ?? "No recorded execution"}</p>
                {current && (
                  <small>
                    {current.completed_executions} completed{" "}
                    {current.completed_executions === 1
                      ? "execution"
                      : "executions"}
                  </small>
                )}
              </div>
            )}
          </>
        )}
        <div className="workflow-logic-legend">
          <p>
            <IconCircle size={14} />
            {steps.length} definition steps
          </p>
          {run && (
            <>
              <p>
                <i className="legend-completed" />
                Completed
              </p>
              <p>
                <i className="legend-waiting" />
                Running or waiting
              </p>
              <p>
                <i className="legend-unobserved" />
                No recorded execution
              </p>
              <small>
                Loop steps summarize recorded iterations. Unvisited paths stay
                neutral.
              </small>
            </>
          )}
        </div>
        {logic.compensation.length > 0 && (
          <p className="workflow-logic-recovery-note">
            Recovery is a separate path and runs only after a failure.
          </p>
        )}
      </aside>
    </div>
  );
}
