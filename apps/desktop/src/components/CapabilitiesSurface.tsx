import { Button, TextInput, WorkspaceSurfaceHeader } from "@colossus/ui";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@colossus/ui/components/ui/table";
import { useEffect, useMemo, useRef, useState } from "react";
import { getCapabilityInventory } from "../api";
import type {
  CapabilityEntry,
  CapabilityInventory,
  CapabilityKind,
} from "../capabilities";
import type { DesktopStatus } from "../types";
import "./capabilities.css";

const SECTIONS: readonly {
  kind: CapabilityKind;
  title: string;
  description: string;
}[] = [
  {
    kind: "tools",
    title: "Tools",
    description:
      "Active tool definitions from this runtime's access profile. Each call still follows policy and approval checks.",
  },
  {
    kind: "mcp",
    title: "MCP servers",
    description:
      "Configured servers and their permitted tools. Connections and remote tool discovery are checked when used.",
  },
  {
    kind: "search",
    title: "Search services",
    description:
      "Search profiles configured in the selected runtime. Health is checked when a search runs.",
  },
  {
    kind: "plugins",
    title: "Plugins",
    description:
      "Installed Agent Plugins and their skills from the runtime catalog.",
  },
];

function CapabilityTable({
  entries,
  title,
}: {
  entries: CapabilityEntry[];
  title: string;
}) {
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(0);
  const filtered = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    return entries.filter((entry) =>
      `${entry.name} ${entry.description} ${entry.detail}`
        .toLocaleLowerCase()
        .includes(needle),
    );
  }, [entries, query]);
  const pageSize = 20;
  const pages = Math.max(1, Math.ceil(filtered.length / pageSize));
  const currentPage = Math.min(page, pages - 1);
  const visible = filtered.slice(
    currentPage * pageSize,
    (currentPage + 1) * pageSize,
  );
  return (
    <div className="capability-table">
      <TextInput
        type="search"
        aria-label={`Search ${title.toLowerCase()}`}
        placeholder={`Search ${title.toLowerCase()}…`}
        value={query}
        onChange={(event) => {
          setQuery(event.target.value);
          setPage(0);
        }}
      />
      <Table aria-label={title}>
        <TableHeader>
          <TableRow>
            <TableHead scope="col">Name</TableHead>
            <TableHead scope="col">Details</TableHead>
            <TableHead scope="col">Status</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {visible.map((entry) => (
            <TableRow key={entry.name}>
              <TableHead scope="row">
                <div className="capability-entry-copy">
                  <strong>{entry.name}</strong>
                  {entry.description ? <span>{entry.description}</span> : null}
                </div>
              </TableHead>
              <TableCell>
                <span className="capability-entry-detail">
                  {entry.detail || "—"}
                </span>
              </TableCell>
              <TableCell>
                <span className="status-chip tone-neutral">{entry.status}</span>
              </TableCell>
            </TableRow>
          ))}
          {!visible.length ? (
            <TableRow>
              <TableCell colSpan={3}>
                {query
                  ? "No matching entries."
                  : `No ${title.toLowerCase()} reported by this runtime.`}
              </TableCell>
            </TableRow>
          ) : null}
        </TableBody>
      </Table>
      <div className="capability-inventory-toolbar">
        <span role="status">
          {filtered.length} {filtered.length === 1 ? "entry" : "entries"}
          {pages > 1 ? ` · Page ${currentPage + 1} of ${pages}` : ""}
        </span>
        {pages > 1 ? (
          <div className="resource-actions">
            <Button
              aria-label={`Previous ${title.toLowerCase()} page`}
              disabled={currentPage === 0}
              onClick={() => setPage(currentPage - 1)}
            >
              Previous
            </Button>
            <Button
              aria-label={`Next ${title.toLowerCase()} page`}
              disabled={currentPage + 1 === pages}
              onClick={() => setPage(currentPage + 1)}
            >
              Next
            </Button>
          </div>
        ) : null}
      </div>
    </div>
  );
}

export function CapabilitiesSurface({ desktop }: { desktop: DesktopStatus }) {
  const targetId = desktop.selectedTargetId;
  const connected =
    desktop.connection.state === "connected" &&
    desktop.connection.targetId === targetId;
  const [inventory, setInventory] = useState<CapabilityInventory | null>(null);
  const [loading, setLoading] = useState(false);
  const [failure, setFailure] = useState("");
  const epoch = useRef(0);

  async function refresh() {
    const request = ++epoch.current;
    setInventory(null);
    setFailure("");
    if (!targetId || !connected) {
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      const next = await getCapabilityInventory(targetId);
      if (epoch.current === request && next.targetId === targetId)
        setInventory(next);
    } catch {
      if (epoch.current === request)
        setFailure(
          "The capability inventory could not be loaded. Check the runtime connection and refresh.",
        );
    } finally {
      if (epoch.current === request) setLoading(false);
    }
  }

  useEffect(() => {
    void refresh();
    return () => {
      epoch.current++;
    };
  }, [targetId, connected]);

  const visible =
    inventory?.targetId === targetId && connected ? inventory : null;
  return (
    <>
      <WorkspaceSurfaceHeader
        eyebrow="This Workspace"
        title="Capabilities"
        description="Tools and services exposed by the selected runtime."
      />
      <div className="overview-scroll capabilities-scroll" tabIndex={0}>
        <div className="capability-inventory-toolbar">
          <span>
            {desktop.accessProfile.replaceAll("_", " ")} profile ·{" "}
            {desktop.executionBoundary.replaceAll("_", " ")} boundary
          </span>
          <Button
            variant="secondary"
            disabled={loading || !connected}
            onClick={() => void refresh()}
          >
            {loading ? "Refreshing…" : "Refresh inventory"}
          </Button>
        </div>
        {!connected ? (
          <p role="status" className="inline-empty">
            Connect this Workspace to read its capability inventory.
          </p>
        ) : null}
        {loading ? <p role="status">Reading runtime capabilities…</p> : null}
        {failure ? <p role="alert">{failure}</p> : null}
        {SECTIONS.map(({ kind, title, description }) => {
          const section = visible?.sections.find(
            (candidate) => candidate.kind === kind,
          );
          return (
            <section
              key={kind}
              className="capability-inventory-section"
              aria-labelledby={`capability-${kind}`}
            >
              <div className="section-heading">
                <div>
                  <h3 id={`capability-${kind}`}>{title}</h3>
                  <p>{description}</p>
                  {section?.available && section.message ? (
                    <p role="status">{section.message}</p>
                  ) : null}
                </div>
              </div>
              {section?.available ? (
                <CapabilityTable
                  key={`${targetId}:${kind}`}
                  title={title}
                  entries={section.entries}
                />
              ) : (
                <p className="inline-empty">
                  {section?.message ||
                    (loading ? "Loading…" : "Inventory unavailable.")}
                </p>
              )}
            </section>
          );
        })}
      </div>
    </>
  );
}
