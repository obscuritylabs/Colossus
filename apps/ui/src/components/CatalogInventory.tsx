import {
  IconActivityHeartbeat,
  IconChevronRight,
  IconCloud,
  IconCpu,
  IconDots,
  IconInfoCircle,
  IconKey,
  IconPlus,
  IconSearch,
  IconTrash,
  IconX,
} from "@tabler/icons-react";
import {
  Fragment,
  type ReactNode,
  useEffect,
  useId,
  useRef,
  useState,
} from "react";
import { Button } from "./Controls.js";

export interface CatalogInventoryRow {
  id: string;
  label: string;
  name: string;
  description: string;
  icon?: ReactNode;
  searchText: string;
  connection: ReactNode;
  usage: string;
  details: ReactNode;
  actionLabel?: string;
  actionIcon?: ReactNode;
  actionAriaLabel?: string;
  mutationDisabled?: boolean;
  onEdit: () => void;
  onDelete?: (trigger: HTMLButtonElement) => void;
}

export interface CatalogInventoryKind {
  title: string;
  singular: string;
  plural: string;
  column: string;
  description: string;
  connectionLabel: string;
  usageLabel: string;
  Icon: typeof IconCloud;
  className: string;
}

const catalogKinds = {
  model: {
    title: "Models",
    singular: "model",
    plural: "models",
    column: "Model",
    description: "Choose and manage the models your workspaces use.",
    connectionLabel: "Provider",
    usageLabel: "Active workspaces",
    Icon: IconCpu,
    className: "models-settings",
  },
  provider: {
    title: "Providers",
    singular: "provider",
    plural: "providers",
    column: "Provider",
    description: "Saved connections for your workspaces.",
    connectionLabel: "Sign-in",
    usageLabel: "Configured models",
    Icon: IconCloud,
    className: "providers-settings",
  },
  credential: {
    title: "Credentials",
    singular: "credential",
    plural: "credentials",
    column: "Credential",
    description: "API keys, tokens, and client secrets for your connections.",
    connectionLabel: "Status",
    usageLabel: "Used by",
    Icon: IconKey,
    className: "credentials-settings",
  },
  search: {
    title: "Search services",
    singular: "search service",
    plural: "search services",
    column: "Service",
    description:
      "Saved search services. Enable each service in Workspace settings.",
    connectionLabel: "Credential",
    usageLabel: "Active workspaces",
    Icon: IconSearch,
    className: "search-settings",
  },
  telemetry: {
    title: "Telemetry connections",
    singular: "telemetry connection",
    plural: "telemetry connections",
    column: "Connection",
    description: "Manage collectors, exported signals, and audit content.",
    connectionLabel: "Exported data",
    usageLabel: "Active workspaces",
    Icon: IconActivityHeartbeat,
    className: "telemetry-settings",
  },
} as const;

/** Shared presentation only; catalog mutations remain with the host. */
export function CatalogInventory({
  kind,
  summary,
  rows,
  busy,
  editing,
  onAdd,
  children,
  footer,
  addLabel,
  emptyDescription,
  headingLevel = 3,
}: {
  kind: keyof typeof catalogKinds | CatalogInventoryKind;
  summary: string;
  rows: CatalogInventoryRow[];
  busy: boolean;
  editing: boolean;
  onAdd: () => void;
  children?: ReactNode;
  footer?: ReactNode;
  addLabel?: string;
  emptyDescription?: string;
  headingLevel?: 2 | 3;
}) {
  const [query, setQuery] = useState("");
  const search = useRef<HTMLInputElement>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const id = useId();
  const {
    title,
    singular,
    plural,
    column,
    description,
    connectionLabel,
    usageLabel,
    Icon,
    className,
  } = typeof kind === "string" ? catalogKinds[kind] : kind;
  const Heading = headingLevel === 2 ? "h2" : "h3";
  const searchLabel =
    kind === "search" ? "Search services" : `Search ${plural}`;
  const needle = query.trim().toLocaleLowerCase();
  const visible = rows.filter((row) =>
    row.searchText.toLocaleLowerCase().includes(needle),
  );

  return (
    <section
      className={`managed-settings-body ${className} catalog-settings`}
      aria-labelledby={`${id}-heading`}
    >
      <header className="catalog-heading">
        <div>
          <Heading id={`${id}-heading`}>{title}</Heading>
          <p>{description}</p>
        </div>
        <Button
          id={typeof kind === "string" ? `add-${kind}` : undefined}
          variant="primary"
          type="button"
          disabled={busy || editing}
          onClick={() => {
            setQuery("");
            onAdd();
          }}
        >
          <IconPlus size={16} aria-hidden="true" />{" "}
          {addLabel ?? `Add ${singular}`}
        </Button>
      </header>
      <p className="catalog-summary">{summary}</p>
      {children}
      <div className="catalog-search">
        <IconSearch size={18} aria-hidden="true" />
        <input
          ref={search}
          aria-label={searchLabel}
          placeholder={searchLabel}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        {query ? (
          <button
            type="button"
            className="ui-icon-button"
            aria-label="Clear search"
            onClick={() => {
              setQuery("");
              search.current?.focus();
            }}
          >
            <IconX size={16} aria-hidden="true" />
          </button>
        ) : null}
      </div>
      <div className="catalog-table-container">
        <table className="catalog-table" aria-label={`Configured ${plural}`}>
          <colgroup>
            <col className="catalog-name-column" />
            <col />
            <col />
            <col className="catalog-actions-column" />
          </colgroup>
          <thead>
            <tr>
              <th scope="col">{column}</th>
              <th scope="col">{connectionLabel}</th>
              <th scope="col">{usageLabel}</th>
              <th scope="col">
                <span className="sr-only">Actions</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {visible.map((row) => {
              const open = expanded === row.id;
              const detailsId = `${id}-${row.id}-details`;
              const toggle = () => setExpanded(open ? null : row.id);
              return (
                <Fragment key={row.id}>
                  <tr className="catalog-inventory-row" data-expanded={open}>
                    <th scope="row">
                      <div className="catalog-identity">
                        <span
                          className={
                            row.icon ? "catalog-provider-icon" : "resource-icon"
                          }
                        >
                          {row.icon ?? <Icon size={19} aria-hidden="true" />}
                        </span>
                        <div>
                          <button
                            type="button"
                            className="catalog-name"
                            aria-label={`Details for ${row.label}`}
                            aria-expanded={open}
                            aria-controls={detailsId}
                            onClick={toggle}
                          >
                            {row.name}
                          </button>
                          <small>{row.description}</small>
                        </div>
                      </div>
                    </th>
                    <td data-label={connectionLabel}>
                      <span className="catalog-connection">
                        {row.connection}
                      </span>
                    </td>
                    <td data-label={usageLabel}>
                      <button
                        type="button"
                        className="catalog-text-button catalog-usage"
                        aria-label={`${kind === "credential" ? "Connections" : usageLabel} for ${row.label}: ${row.usage}`}
                        aria-expanded={open}
                        aria-controls={detailsId}
                        onClick={toggle}
                      >
                        {row.usage}
                        <IconChevronRight size={16} aria-hidden="true" />
                      </button>
                    </td>
                    <td className="catalog-row-actions">
                      <div className="catalog-action-group">
                        <button
                          type="button"
                          className={
                            row.actionIcon
                              ? "ui-icon-button"
                              : "catalog-text-button"
                          }
                          disabled={busy || row.mutationDisabled}
                          aria-label={
                            row.actionAriaLabel ?? `Edit ${row.label}`
                          }
                          title={row.actionAriaLabel ?? `Edit ${row.label}`}
                          onClick={row.onEdit}
                        >
                          {row.actionIcon ?? row.actionLabel ?? "Edit"}
                        </button>
                        {row.onDelete ? (
                          <CatalogActions
                            label={row.label}
                            busy={busy}
                            deleteDisabled={row.mutationDisabled}
                            onDetails={() => setExpanded(row.id)}
                            onDelete={row.onDelete}
                          />
                        ) : null}
                      </div>
                    </td>
                  </tr>
                  <tr
                    hidden={!open}
                    className="catalog-detail-row"
                    id={detailsId}
                  >
                    <td colSpan={4}>
                      <section
                        className="catalog-details"
                        aria-label={`Details for ${row.label}`}
                      >
                        {row.details}
                      </section>
                    </td>
                  </tr>
                </Fragment>
              );
            })}
            {!visible.length ? (
              <tr>
                <td colSpan={4} className="catalog-empty">
                  <strong>
                    {rows.length ? `No matching ${plural}` : `No ${plural} yet`}
                  </strong>
                  <p>
                    {rows.length
                      ? "Try a different name or clear your search."
                      : (emptyDescription ??
                        `Add a ${singular} to get started.`)}
                  </p>
                </td>
              </tr>
            ) : null}
          </tbody>
        </table>
      </div>
      <p className={needle ? "catalog-hint" : "sr-only"} role="status">
        {needle
          ? `${visible.length} of ${rows.length} ${plural} match your search.`
          : ""}
      </p>
      {footer}
    </section>
  );
}

function CatalogActions({
  label,
  busy,
  onDetails,
  onDelete,
  deleteDisabled,
}: {
  label: string;
  busy: boolean;
  deleteDisabled?: boolean | undefined;
  onDetails: () => void;
  onDelete: (trigger: HTMLButtonElement) => void;
}) {
  const [open, setOpen] = useState(false);
  const wrapper = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const id = useId();
  useEffect(() => {
    if (!open) return;
    menu.current?.querySelector<HTMLButtonElement>("button")?.focus();
    const dismiss = (event: PointerEvent) => {
      if (
        event.target instanceof Node &&
        !wrapper.current?.contains(event.target)
      )
        setOpen(false);
    };
    document.addEventListener("pointerdown", dismiss);
    return () => document.removeEventListener("pointerdown", dismiss);
  }, [open]);
  const close = () => {
    setOpen(false);
    trigger.current?.focus();
  };
  return (
    <div
      className="catalog-actions-menu"
      ref={wrapper}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false);
      }}
    >
      <button
        ref={trigger}
        className="ui-icon-button"
        type="button"
        disabled={busy}
        aria-label={`More actions for ${label}`}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        onClick={() => setOpen(!open)}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            setOpen(true);
          }
        }}
      >
        <IconDots size={20} aria-hidden="true" />
      </button>
      {open ? (
        <div
          ref={menu}
          id={id}
          className="catalog-actions-popup"
          role="menu"
          aria-label={`Actions for ${label}`}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              event.stopPropagation();
              close();
              return;
            }
            const items = Array.from(
              event.currentTarget.querySelectorAll<HTMLButtonElement>(
                "button:not(:disabled)",
              ),
            );
            const index = items.indexOf(
              document.activeElement as HTMLButtonElement,
            );
            const next =
              event.key === "ArrowDown"
                ? (index + 1) % items.length
                : event.key === "ArrowUp"
                  ? (index + items.length - 1) % items.length
                  : event.key === "Home"
                    ? 0
                    : event.key === "End"
                      ? items.length - 1
                      : null;
            if (next !== null) {
              event.preventDefault();
              items[next]?.focus();
            }
          }}
        >
          <button
            type="button"
            role="menuitem"
            tabIndex={-1}
            onClick={() => {
              close();
              onDetails();
            }}
          >
            <IconInfoCircle size={16} aria-hidden="true" /> View details
          </button>
          <button
            type="button"
            role="menuitem"
            tabIndex={-1}
            className="catalog-delete-action"
            disabled={busy || deleteDisabled}
            aria-label={`Delete ${label}`}
            onClick={() => {
              close();
              onDelete(trigger.current!);
            }}
          >
            <IconTrash size={16} aria-hidden="true" /> Delete
          </button>
        </div>
      ) : null}
    </div>
  );
}
