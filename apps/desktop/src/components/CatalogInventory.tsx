import {
  IconChevronRight,
  IconCloud,
  IconCpu,
  IconDots,
  IconInfoCircle,
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
import "./catalog-inventory.css";

export interface CatalogInventoryRow {
  id: string;
  label: string;
  name: string;
  description: string;
  searchText: string;
  connection: ReactNode;
  usage: string;
  details: ReactNode;
  onEdit: () => void;
  onDelete: (trigger: HTMLButtonElement) => void;
}

/** Shared presentation only; catalog mutations remain with the settings owner. */
export function CatalogInventory({
  kind,
  summary,
  rows,
  busy,
  editing,
  onAdd,
  children,
}: {
  kind: "model" | "provider";
  summary: string;
  rows: CatalogInventoryRow[];
  busy: boolean;
  editing: boolean;
  onAdd: () => void;
  children: ReactNode;
}) {
  const [query, setQuery] = useState("");
  const search = useRef<HTMLInputElement>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const id = useId();
  const providers = kind === "provider";
  const title = providers ? "Providers" : "Models";
  const connectionLabel = providers ? "Sign-in" : "Provider";
  const usageLabel = providers ? "Configured models" : "Active workspaces";
  const needle = query.trim().toLocaleLowerCase();
  const visible = rows.filter((row) =>
    row.searchText.toLocaleLowerCase().includes(needle),
  );
  const Icon = providers ? IconCloud : IconCpu;

  return (
    <section
      className={`managed-settings-body ${kind}s-settings catalog-settings`}
      aria-labelledby={`${id}-heading`}
    >
      <header className="catalog-heading">
        <div>
          <h3 id={`${id}-heading`}>{title}</h3>
          <p>
            {providers
              ? "Connections used by your configured models."
              : "Choose and manage the models your workspaces use."}
          </p>
        </div>
        <button
          id={`add-${kind}`}
          className="button primary"
          type="button"
          disabled={busy || editing}
          onClick={onAdd}
        >
          <IconPlus size={16} aria-hidden="true" /> Add {kind}
        </button>
      </header>
      <p className="catalog-summary">{summary}</p>
      {children}
      <div className="catalog-search">
        <IconSearch size={18} aria-hidden="true" />
        <input
          ref={search}
          aria-label={`Search ${kind}s`}
          placeholder={`Search ${kind}s`}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        {query ? (
          <button
            type="button"
            className="icon-button"
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
        <table className="catalog-table" aria-label={`Configured ${kind}s`}>
          <colgroup>
            <col className="catalog-name-column" />
            <col />
            <col />
            <col className="catalog-actions-column" />
          </colgroup>
          <thead>
            <tr>
              <th scope="col">{providers ? "Provider" : "Model"}</th>
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
                        <span className="resource-icon">
                          <Icon size={19} aria-hidden="true" />
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
                        aria-label={`${usageLabel} for ${row.label}: ${row.usage}`}
                        aria-expanded={open}
                        aria-controls={detailsId}
                        onClick={toggle}
                      >
                        {row.usage}
                        <IconChevronRight size={16} aria-hidden="true" />
                      </button>
                    </td>
                    <td className="catalog-row-actions">
                      <button
                        type="button"
                        className="catalog-text-button"
                        disabled={busy}
                        aria-label={`Edit ${row.label}`}
                        onClick={row.onEdit}
                      >
                        Edit
                      </button>
                      <CatalogActions
                        label={row.label}
                        busy={busy}
                        onDetails={() => setExpanded(row.id)}
                        onDelete={row.onDelete}
                      />
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
                    {rows.length ? `No matching ${kind}s` : `No ${kind}s yet`}
                  </strong>
                  <p>
                    {rows.length
                      ? "Try a different name or clear your search."
                      : `Add a ${kind} to get started.`}
                  </p>
                </td>
              </tr>
            ) : null}
          </tbody>
        </table>
      </div>
      <p className="catalog-hint" role="status">
        {needle
          ? `${visible.length} of ${rows.length} ${kind}s match your search.`
          : `Select a ${kind} to view its ${providers ? "connection details and models" : "settings and workspace usage"}.`}
      </p>
    </section>
  );
}

function CatalogActions({
  label,
  busy,
  onDetails,
  onDelete,
}: {
  label: string;
  busy: boolean;
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
        className="icon-button"
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
              event.currentTarget.querySelectorAll<HTMLButtonElement>("button"),
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
            disabled={busy}
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
