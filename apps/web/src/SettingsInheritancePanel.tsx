import { Button } from "@colossus/ui";
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
import type { ChatSettingsSnapshot, InheritedSection } from "./chat-settings";
import type { ConnectionSettingsSection } from "./routes";

export function SettingsInheritancePanel({
  saved,
  section,
  busy,
  onReset,
  roleFamily = "model",
}: {
  saved: ChatSettingsSnapshot;
  section: ConnectionSettingsSection;
  busy: boolean;
  onReset: (section: InheritedSection, id: string) => void;
  roleFamily?: "model" | "search";
}) {
  const inheritance = saved.inheritance;
  if (!inheritance || section === "credentials" || section === "telemetry")
    return null;
  const kind = section === "routing" ? "roles" : section;
  const disabled = new Set(inheritance.disabled?.[kind] ?? []);
  const overrides = new Set(inheritance.overrides[kind]);
  const rows = (
    inheritance.global_entries?.[kind] ??
    Object.keys(inheritance.provenance[kind]).filter(
      (id) => inheritance.provenance[kind][id] === "global",
    )
  )
    .filter(
      (id) =>
        kind !== "roles" ||
        id.startsWith("search:") === (roleFamily === "search"),
    )
    .map((id) => ({
      id,
      status: disabled.has(id)
        ? "Disabled in this project"
        : overrides.has(id)
          ? "Project override"
          : "Inherited from Control Plane",
    }));
  const columns: DataTableColumn<(typeof rows)[number]>[] = [
    {
      id: "profile",
      label: "Definition",
      value: (row) =>
        row.id === "search:agent"
          ? "Agent search"
          : row.id === "search:research"
            ? "Research search"
            : row.id,
    },
    { id: "source", label: "Project setting", value: (row) => row.status },
    {
      id: "actions",
      label: "Actions",
      value: () => "",
      cell: (row) =>
        overrides.has(row.id) || disabled.has(row.id) ? (
          <Button disabled={busy} onClick={() => onReset(kind, row.id)}>
            {disabled.has(row.id)
              ? "Enable inherited definition"
              : "Reset to Control Plane"}
          </Button>
        ) : (
          <span>Up to date</span>
        ),
    },
  ];
  return (
    <section
      className="control-card"
      aria-label={
        roleFamily === "search"
          ? "Inherited search routing"
          : "Inherited configuration"
      }
    >
      <p className="field-help">
        Control Plane defaults flow into this project. Editing an inherited
        definition creates a project override. Secrets require an explicit
        attachment and remain separate from definition inheritance.
      </p>
      {rows.length ? (
        <details>
          <summary>
            {rows.length} global{" "}
            {roleFamily === "search" ? "search routes" : "definitions"} ·
            revision {inheritance.global_revision}
          </summary>
          <DataTable
            data={rows}
            columns={columns}
            getRowId={(row) => row.id}
            label="Inherited definitions"
            search={{
              columnId: "profile",
              label: "Search inherited definitions",
            }}
          />
        </details>
      ) : (
        <p className="field-help">
          No global definitions in this section. New definitions belong to this
          project.
        </p>
      )}
    </section>
  );
}
