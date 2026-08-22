import {
  Handle,
  Position,
  ReactFlow,
  type Edge,
  type Node,
  type NodeProps,
  type NodeTypes,
  type ReactFlowInstance,
} from "@xyflow/react";
import {
  IconChevronDown,
  IconChevronRight,
  IconSparkles,
} from "@tabler/icons-react";
import { memo, useCallback, useEffect, useMemo, useRef } from "react";
import type { ComponentType } from "react";

import "@xyflow/react/dist/style.css";

type IconComponent = ComponentType<{ size?: number; stroke?: number }>;

export type SessionTopologyRecordModel = {
  id: string;
  title: string;
  meta: string;
  statusLabel: string;
  tone: "active" | "complete" | "warning" | "muted";
  onSelect: () => void;
};

export type SessionTopologyFamilyModel = {
  id: string;
  label: string;
  layer: string;
  icon: IconComponent;
  count: number;
  open: boolean;
  records: readonly SessionTopologyRecordModel[];
  onToggle: () => void;
};

export type SessionTopologyPrimaryModel = {
  name: string;
  startedLabel: string;
  stateLabel: string;
};

type PrimaryNodeData = SessionTopologyPrimaryModel;
type FamilyNodeData = SessionTopologyFamilyModel;
type RecordNodeData = SessionTopologyRecordModel;
type EmptyNodeData = { label: string };

type PrimaryNode = Node<PrimaryNodeData, "sessionPrimary">;
type FamilyNode = Node<FamilyNodeData, "sessionFamily">;
type RecordNode = Node<RecordNodeData, "sessionRecord">;
type EmptyNode = Node<EmptyNodeData, "sessionEmpty">;
type SessionFlowNode = PrimaryNode | FamilyNode | RecordNode | EmptyNode;

const ROOT_WIDTH = 190;
const ROOT_HEIGHT = 86;
const FAMILY_WIDTH = 250;
const FAMILY_HEIGHT = 58;
const RECORD_WIDTH = 300;
const RECORD_HEIGHT = 58;
const FAMILY_X = 300;
const RECORD_X = 650;
const ROW_GAP = 12;
const RECORD_GAP = 7;

const FIT_VIEW_OPTIONS = {
  padding: 0.08,
  minZoom: 0.55,
  maxZoom: 1,
} as const;

const DEFAULT_EDGE_OPTIONS = {
  type: "step",
  focusable: false,
  selectable: false,
  deletable: false,
  style: { stroke: "#3a5777", strokeWidth: 1 },
} as const;

function PrimaryNodeCard({ data }: { data: PrimaryNodeData }) {
  return (
    <article className="session-map-primary">
      <span aria-hidden="true">
        <IconSparkles size={20} stroke={1.65} />
      </span>
      <div>
        <strong>{data.name}</strong>
        <small>Started {data.startedLabel}</small>
      </div>
      <em>
        <i /> {data.stateLabel}
      </em>
    </article>
  );
}

const PrimaryFlowNode = memo(function PrimaryFlowNode({
  data,
}: NodeProps<PrimaryNode>) {
  return (
    <>
      <PrimaryNodeCard data={data} />
      <Handle
        className="session-map-handle"
        type="source"
        position={Position.Right}
        isConnectable={false}
      />
    </>
  );
});

const FamilyFlowNode = memo(function FamilyFlowNode({
  data,
}: NodeProps<FamilyNode>) {
  const Icon = data.icon;
  return (
    <>
      <Handle
        className="session-map-handle"
        type="target"
        position={Position.Left}
        isConnectable={false}
      />
      <button
        className={`session-map-family family-${data.layer} nodrag nopan`}
        type="button"
        aria-expanded={data.open}
        onClick={data.onToggle}
      >
        <span aria-hidden="true">
          <Icon size={18} stroke={1.6} />
        </span>
        <span>
          <strong>
            {data.label} <b>{data.count}</b>
          </strong>
          <small>
            {data.count === 0
              ? "No released records"
              : `${data.count} released`}
          </small>
        </span>
        <IconChevronDown size={15} stroke={1.6} aria-hidden="true" />
      </button>
      <Handle
        className="session-map-handle"
        type="source"
        position={Position.Right}
        isConnectable={false}
      />
    </>
  );
});

const RecordFlowNode = memo(function RecordFlowNode({
  data,
}: NodeProps<RecordNode>) {
  return (
    <>
      <Handle
        className="session-map-handle"
        type="target"
        position={Position.Left}
        isConnectable={false}
      />
      <button
        className="session-map-record nodrag nopan"
        type="button"
        onClick={data.onSelect}
      >
        <span>
          <strong>{data.title}</strong>
          <small>{data.meta}</small>
        </span>
        <em className={`tone-${data.tone}`}>
          <i /> {data.statusLabel}
        </em>
        <IconChevronRight size={15} stroke={1.6} aria-hidden="true" />
      </button>
    </>
  );
});

const EmptyFlowNode = memo(function EmptyFlowNode({
  data,
}: NodeProps<EmptyNode>) {
  return (
    <>
      <Handle
        className="session-map-handle"
        type="target"
        position={Position.Left}
        isConnectable={false}
      />
      <div className="session-map-child-empty">{data.label}</div>
    </>
  );
});

const NODE_TYPES = {
  sessionPrimary: PrimaryFlowNode,
  sessionFamily: FamilyFlowNode,
  sessionRecord: RecordFlowNode,
  sessionEmpty: EmptyFlowNode,
} as NodeTypes;

function edge(id: string, source: string, target: string): Edge {
  return {
    id,
    source,
    target,
    ...DEFAULT_EDGE_OPTIONS,
  };
}

function buildGraph(
  primary: SessionTopologyPrimaryModel,
  families: readonly SessionTopologyFamilyModel[],
): { nodes: SessionFlowNode[]; edges: Edge[] } {
  const nodes: SessionFlowNode[] = [];
  const edges: Edge[] = [];
  const rows: Array<{
    family: SessionTopologyFamilyModel;
    y: number;
  }> = [];
  let nextY = 0;

  for (const family of families) {
    const recordCount = family.open ? Math.max(family.records.length, 1) : 0;
    const recordStackHeight =
      recordCount === 0
        ? 0
        : recordCount * RECORD_HEIGHT + (recordCount - 1) * RECORD_GAP;
    rows.push({ family, y: nextY });
    nextY += Math.max(FAMILY_HEIGHT, recordStackHeight) + ROW_GAP;
  }

  const graphHeight = Math.max(nextY - ROW_GAP, ROOT_HEIGHT);
  nodes.push({
    id: "session-primary",
    type: "sessionPrimary",
    position: { x: 0, y: (graphHeight - ROOT_HEIGHT) / 2 },
    sourcePosition: Position.Right,
    data: primary,
    draggable: false,
    selectable: false,
    focusable: false,
    ariaLabel: `Primary session ${primary.name}`,
    style: { width: ROOT_WIDTH, height: ROOT_HEIGHT },
  });

  for (const { family, y } of rows) {
    const familyId = `family:${family.id}`;
    nodes.push({
      id: familyId,
      type: "sessionFamily",
      position: { x: FAMILY_X, y },
      sourcePosition: Position.Right,
      targetPosition: Position.Left,
      data: family,
      draggable: false,
      selectable: false,
      focusable: false,
      ariaLabel: `${family.label}, ${family.count} released`,
      style: { width: FAMILY_WIDTH, height: FAMILY_HEIGHT },
    });
    edges.push(edge(`primary:${family.id}`, "session-primary", familyId));

    if (!family.open) continue;
    const records: readonly (SessionTopologyRecordModel | null)[] =
      family.records.length === 0 ? [null] : family.records;
    records.forEach((record, index) => {
      const recordId = record?.id ?? `empty:${family.id}`;
      const nodeId = `record:${family.id}:${recordId}`;
      const position = {
        x: RECORD_X,
        y: y + index * (RECORD_HEIGHT + RECORD_GAP),
      };
      if (record === null) {
        nodes.push({
          id: nodeId,
          type: "sessionEmpty",
          position,
          targetPosition: Position.Left,
          data: { label: "No released records in this family." },
          draggable: false,
          selectable: false,
          focusable: false,
          ariaLabel: `${family.label}: no released records`,
          style: { width: RECORD_WIDTH, height: RECORD_HEIGHT },
        });
      } else {
        nodes.push({
          id: nodeId,
          type: "sessionRecord",
          position,
          targetPosition: Position.Left,
          data: record,
          draggable: false,
          selectable: false,
          focusable: false,
          ariaLabel: record.title,
          style: { width: RECORD_WIDTH, height: RECORD_HEIGHT },
        });
      }
      edges.push(
        edge(`record-edge:${family.id}:${recordId}`, familyId, nodeId),
      );
    });
  }

  return { nodes, edges };
}

export function SessionTopologyGraph({
  primary,
  families,
  fitRequest,
}: {
  primary: SessionTopologyPrimaryModel;
  families: readonly SessionTopologyFamilyModel[];
  fitRequest: number;
}) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const flowRef = useRef<ReactFlowInstance<SessionFlowNode, Edge> | null>(null);
  const { nodes, edges } = useMemo(
    () => buildGraph(primary, families),
    [families, primary],
  );
  const layoutKey = useMemo(() => nodes.map(({ id }) => id).join("|"), [nodes]);

  const fitGraph = useCallback((duration = 0) => {
    void flowRef.current?.fitView({ ...FIT_VIEW_OPTIONS, duration });
  }, []);

  useEffect(() => {
    const frame = requestAnimationFrame(() => fitGraph());
    return () => cancelAnimationFrame(frame);
  }, [fitGraph, fitRequest, layoutKey]);

  useEffect(() => {
    const container = containerRef.current;
    if (container === null || typeof ResizeObserver === "undefined") return;

    let frame = 0;
    const observer = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => fitGraph(120));
    });
    observer.observe(container);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    };
  }, [fitGraph]);

  const handleInit = useCallback(
    (instance: ReactFlowInstance<SessionFlowNode, Edge>) => {
      flowRef.current = instance;
      requestAnimationFrame(() => fitGraph());
    },
    [fitGraph],
  );
  // React Flow disables wrapper hit-testing when no node-level interaction is
  // registered. The custom nodes own native buttons, so keep their wrappers
  // interactive without making the graph nodes selectable.
  const allowNodePointerEvents = useCallback(() => undefined, []);

  return (
    <div
      ref={containerRef}
      className="session-map-flow"
      aria-label="Session topology graph"
    >
      <ReactFlow<SessionFlowNode, Edge>
        nodes={nodes}
        edges={edges}
        nodeTypes={NODE_TYPES}
        defaultEdgeOptions={DEFAULT_EDGE_OPTIONS}
        onInit={handleInit}
        onNodeClick={allowNodePointerEvents}
        fitView
        fitViewOptions={FIT_VIEW_OPTIONS}
        minZoom={0.55}
        maxZoom={1.4}
        nodesDraggable={false}
        nodesConnectable={false}
        nodesFocusable={false}
        edgesFocusable={false}
        elementsSelectable={false}
        panOnDrag
        panOnScroll={false}
        zoomOnScroll={false}
        zoomOnDoubleClick={false}
        preventScrolling={false}
        onlyRenderVisibleElements
      />
    </div>
  );
}
