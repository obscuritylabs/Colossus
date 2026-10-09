import {
  Background,
  BackgroundVariant,
  Handle,
  MarkerType,
  Position,
  ReactFlow,
  type Edge,
  type Node,
  type NodeProps,
  type ReactFlowInstance,
} from "@xyflow/react";
import {
  IconCloud,
  IconCpu,
  IconFocus2,
  IconUserCircle,
  IconZoomIn,
  IconZoomOut,
} from "@tabler/icons-react";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { GraphWorkspace } from "@colossus/ui";
import { DropdownSelect } from "./DropdownSelect";
import {
  MODEL_ROLES,
  ModelRoleSelect,
  type RoleModel,
} from "./ModelRoleRouting";
import { modelRouteGroups } from "./model-routing-map";
import "@xyflow/react/dist/style.css";
import "@colossus/ui/styles/workflow-logic.css";

type RouteNode = Node<
  {
    kind: "role" | "model" | "provider";
    label: string;
    detail: string;
    selected: boolean;
    onSelect?: () => void;
  },
  "route"
>;

const RoutingNode = memo(function RoutingNode({ data }: NodeProps<RouteNode>) {
  const Icon =
    data.kind === "role"
      ? IconUserCircle
      : data.kind === "model"
        ? IconCpu
        : IconCloud;
  const content = (
    <>
      <span className="workflow-flow-kind">
        <Icon size={15} aria-hidden="true" />
        {data.kind}
      </span>
      <strong>{data.label}</strong>
      <small>{data.detail}</small>
    </>
  );
  return (
    <>
      {data.kind !== "role" ? (
        <Handle type="target" position={Position.Left} isConnectable={false} />
      ) : null}
      {data.kind !== "provider" ? (
        <Handle type="source" position={Position.Right} isConnectable={false} />
      ) : null}
      {data.kind === "role" ? (
        <button
          type="button"
          className="workflow-flow-step routing-flow-node"
          aria-label={`${data.kind}: ${data.label}. ${data.detail}`}
          aria-pressed={data.selected}
          onClick={data.onSelect}
          onFocus={(event) => {
            if (event.currentTarget.matches(":focus-visible"))
              data.onSelect?.();
          }}
        >
          {content}
        </button>
      ) : (
        <div
          className="workflow-flow-step routing-flow-node"
          data-highlighted={data.selected}
        >
          {content}
        </div>
      )}
    </>
  );
});
const NODE_TYPES = { route: RoutingNode };
const FIT = { padding: 0.13, minZoom: 0.25, maxZoom: 1 };

export function ModelRoutingGraph({
  roles,
  models,
  onChange,
  disabled = false,
}: {
  roles: Record<string, string>;
  models: RoleModel[];
  onChange?: ((roles: Record<string, string>) => void) | undefined;
  disabled?: boolean;
}) {
  const [selectedRole, setSelectedRole] = useState("primary");
  const flow = useRef<ReactFlowInstance<RouteNode, Edge> | null>(null);
  const canvas = useRef<HTMLDivElement>(null);
  const profile =
    roles[selectedRole] ||
    (selectedRole === "primary" ? "" : roles.primary) ||
    "";
  const model = models.find((candidate) => candidate.profile === profile);
  const role = MODEL_ROLES.find((candidate) => candidate.id === selectedRole)!;
  const inherited = selectedRole !== "primary" && !roles[selectedRole];
  const selectRole = useCallback((id: string) => {
    setSelectedRole(id);
  }, []);
  const { nodes, edges } = useMemo(() => {
    const groups = modelRouteGroups(roles, models);
    const providers = [
      ...new Set(groups.map((group) => group.model?.providerProfile || "")),
    ];
    const height = (MODEL_ROLES.length - 1) * 86;
    const nodes: RouteNode[] = MODEL_ROLES.map((item, index) => ({
      id: `role:${item.id}`,
      type: "route",
      position: { x: 0, y: index * 86 },
      width: 210,
      height: 72,
      data: {
        kind: "role",
        label: item.label,
        detail:
          !roles[item.id] && item.id === "primary"
            ? "Assignment required"
            : item.id !== "primary" && !roles[item.id]
              ? "Inherits Primary"
              : "Explicit assignment",
        selected: selectedRole === item.id,
        onSelect: () => selectRole(item.id),
      },
    }));
    groups.forEach((group, index) =>
      nodes.push({
        id: `model:${group.profile}`,
        type: "route",
        position: {
          x: 320,
          y:
            groups.length === 1
              ? height / 2
              : (index * height) / (groups.length - 1),
        },
        width: 210,
        height: 72,
        data: {
          kind: "model",
          label:
            group.model?.label ||
            (group.profile ? "Unavailable model" : "No model assigned"),
          detail:
            group.model?.model || group.profile || "Choose a primary model",
          selected: group.profile === profile,
        },
      }),
    );
    providers.forEach((provider, index) =>
      nodes.push({
        id: `provider:${provider}`,
        type: "route",
        position: {
          x: 640,
          y:
            providers.length === 1
              ? height / 2
              : (index * height) / (providers.length - 1),
        },
        width: 210,
        height: 72,
        data: {
          kind: "provider",
          label: provider || "Unresolved",
          detail: provider ? "Configured connection" : "No available provider",
          selected: provider === (model?.providerProfile || ""),
        },
      }),
    );
    const edges: Edge[] = [];
    for (const group of groups) {
      for (const item of group.roles) {
        const active = item.id === selectedRole;
        const color = active ? "var(--blue)" : "var(--border-strong)";
        edges.push({
          id: `role-edge:${item.id}`,
          source: `role:${item.id}`,
          target: `model:${group.profile}`,
          type: "smoothstep",
          markerEnd: { type: MarkerType.ArrowClosed, color },
          style: {
            stroke: color,
            strokeWidth: active ? 2.5 : 1.5,
            strokeDasharray: item.inherited ? "5 4" : undefined,
          },
        });
      }
      const active = group.profile === profile;
      const color = active ? "var(--blue)" : "var(--border-strong)";
      edges.push({
        id: `provider-edge:${group.profile}`,
        source: `model:${group.profile}`,
        target: `provider:${group.model?.providerProfile || ""}`,
        type: "smoothstep",
        markerEnd: { type: MarkerType.ArrowClosed, color },
        style: { stroke: color, strokeWidth: active ? 2.5 : 1.5 },
      });
    }
    return { nodes, edges };
  }, [
    roles,
    models,
    selectedRole,
    profile,
    model?.providerProfile,
    selectRole,
  ]);
  const fit = useCallback(() => {
    void flow.current?.fitView(FIT);
  }, []);
  useEffect(() => {
    if (!canvas.current) return;
    const observer = new ResizeObserver(fit);
    observer.observe(canvas.current);
    return () => observer.disconnect();
  }, [fit]);
  useEffect(() => {
    const frame = requestAnimationFrame(fit);
    return () => cancelAnimationFrame(frame);
  }, [roles, models, fit]);
  return (
    <GraphWorkspace
      canvasRef={canvas}
      label="Routing graph view"
      fitLabel="Fit routes"
      onZoomIn={() => void flow.current?.zoomIn()}
      onZoomOut={() => void flow.current?.zoomOut()}
      onFit={fit}
      zoomInIcon={<IconZoomIn size={18} />}
      zoomOutIcon={<IconZoomOut size={18} />}
      fitIcon={<IconFocus2 size={18} />}
      hint="Dashed: inherits Primary · Drag to pan"
      inspectorLabel="Route details"
      inspector={
        <>
          <span className="workflow-logic-eyebrow">ROUTE DETAILS</span>
          <label>
            Choose a role
            <DropdownSelect
              aria-label="Inspect role"
              value={selectedRole}
              onChange={(event) => selectRole(event.target.value)}
            >
              {MODEL_ROLES.map((item) => (
                <option key={item.id} value={item.id}>
                  {item.label}
                </option>
              ))}
            </DropdownSelect>
          </label>
          <h4>{role.label}</h4>
          <p className="workflow-logic-step-summary">
            {!profile
              ? "Choose a Primary model to resolve this route."
              : inherited
                ? "Inherits the Primary assignment."
                : "Uses its own model assignment."}
          </p>
          {onChange ? (
            <label>
              Model assignment
              <ModelRoleSelect
                roleId={selectedRole}
                roles={roles}
                models={models}
                onChange={onChange}
                disabled={disabled}
                label={`${role.label} assignment`}
              />
            </label>
          ) : null}
          <dl className="routing-graph-route-details">
            <dt>Model</dt>
            <dd>
              {model?.label || (profile ? "Unavailable model" : "Not assigned")}
            </dd>
            <dt>Provider</dt>
            <dd>{model?.providerProfile || "Unresolved"}</dd>
          </dl>
          <p className="workflow-logic-step-summary">{role.description}</p>
        </>
      }
    >
      <ReactFlow<RouteNode, Edge>
        nodes={nodes}
        edges={edges}
        nodeTypes={NODE_TYPES}
        onInit={(instance) => {
          flow.current = instance;
          void instance.fitView(FIT);
        }}
        onNodeClick={() => undefined}
        fitView
        fitViewOptions={FIT}
        minZoom={0.25}
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
        aria-label="Model routing graph"
      >
        <Background
          variant={BackgroundVariant.Dots}
          gap={20}
          size={1}
          color="var(--border)"
        />
      </ReactFlow>
    </GraphWorkspace>
  );
}
