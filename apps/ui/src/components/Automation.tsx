import { useId, useState, type ReactNode } from "react";
import {
  IconArrowUpRight,
  IconCalendarTime,
  IconSearch,
} from "@tabler/icons-react";
import { Button, TextInput } from "./Controls.js";
import { Badge } from "./ui/badge.js";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "./ui/table.js";

export function AutomationSurface({
  label,
  children,
  className = "",
}: {
  label: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <section className={`automation-surface ${className}`} aria-label={label}>
      {children}
    </section>
  );
}

export interface AutomationExample {
  id: string;
  name: string;
  description: string;
  timing: string;
  icon?: ReactNode;
}
export interface AutomationExampleGalleryProps {
  title: string;
  description: string;
  examples: readonly AutomationExample[];
  help?: ReactNode;
  disabled: boolean;
  onCreate: (id: string) => void;
}
/** Examples and callbacks only; the host owns prompts and thread creation. */
export function AutomationExampleGallery({
  title,
  description,
  examples,
  help,
  disabled,
  onCreate,
}: AutomationExampleGalleryProps) {
  const headingId = useId();
  return (
    <section className="automation-examples" aria-labelledby={headingId}>
      <header>
        <h3 id={headingId}>{title}</h3>
        <p>{description}</p>
      </header>
      <div className="automation-example-grid">
        {examples.map((example) => (
          <article
            className="automation-example"
            key={example.id}
            aria-label={example.name}
          >
            <span className="automation-example-icon">{example.icon}</span>
            <h4>{example.name}</h4>
            <p>{example.description}</p>
            <span className="automation-example-timing">
              <IconCalendarTime size={14} aria-hidden="true" />
              {example.timing}
            </span>
            <Button
              disabled={disabled}
              onClick={() => onCreate(example.id)}
              aria-label={`Create ${example.name} with agent`}
            >
              Create with agent{" "}
              <IconArrowUpRight size={16} aria-hidden="true" />
            </Button>
          </article>
        ))}
      </div>
      {help && <p className="automation-help">{help}</p>}
    </section>
  );
}
export interface AutomationWelcomeProps {
  title: string;
  description: string;
  icon: ReactNode;
  action: ReactNode;
  help?: ReactNode;
  features: readonly { title: string; description: string; icon: ReactNode }[];
}
/** Shared workflow landing presentation, independent of runtime capabilities. */
export function AutomationWelcome({
  title,
  description,
  icon,
  action,
  help,
  features,
}: AutomationWelcomeProps) {
  const headingId = useId();
  return (
    <section className="automation-welcome" aria-labelledby={headingId}>
      <span className="automation-welcome-icon">{icon}</span>
      <h3 id={headingId}>{title}</h3>
      <p>{description}</p>
      {action}
      {help && <p className="automation-help">{help}</p>}
      <div className="automation-welcome-steps">
        {features.map((feature) => (
          <div key={feature.title}>
            {feature.icon}
            <h4>{feature.title}</h4>
            <p>{feature.description}</p>
          </div>
        ))}
      </div>
    </section>
  );
}
export interface AutomationOverviewItem {
  id: string;
  label: ReactNode;
  value: ReactNode;
  title?: string;
}
export function AutomationOverview({
  items,
}: {
  items: readonly AutomationOverviewItem[];
}) {
  return (
    <dl className="automation-overview">
      {items.map((item) => (
        <div key={item.id}>
          <dt>{item.label}</dt>
          <dd title={item.title}>{item.value}</dd>
        </div>
      ))}
    </dl>
  );
}

export interface AutomationInventoryRow {
  id: string;
  name: string;
  kind: string;
  actionLabel: string;
  searchText: string;
  repeat: ReactNode;
  nextOccurrence: ReactNode;
  nextOccurrenceTitle: string;
  occurrenceLabel: string;
  status: "enabled" | "paused" | "blocked";
  note?: string;
}
export interface AutomationInventoryProps {
  rows: readonly AutomationInventoryRow[];
  selectedId?: string | undefined;
  busy: boolean;
  onInspect: (id: string) => void;
}
/** Filter only the records supplied by the host; fetching and authority stay there. */
export function AutomationInventory({
  rows,
  selectedId,
  busy,
  onInspect,
}: AutomationInventoryProps) {
  const [query, setQuery] = useState("");
  const visible = rows.filter((row) =>
    row.searchText
      .toLocaleLowerCase()
      .includes(query.trim().toLocaleLowerCase()),
  );
  return (
    <div className="automation-inventory">
      <p className="catalog-summary">
        {rows.length} {rows.length === 1 ? "schedule" : "schedules"}
      </p>
      <div className="catalog-search">
        <IconSearch size={18} aria-hidden="true" />
        <TextInput
          aria-label="Search loaded schedules"
          placeholder="Search schedules"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
      </div>
      <div className="catalog-table-container">
        <Table className="catalog-table" aria-label="Schedules">
          <colgroup>
            <col className="catalog-name-column" />
            <col />
            <col />
            <col />
          </colgroup>
          <TableHeader>
            <TableRow>
              <TableHead scope="col">Task or workflow</TableHead>
              <TableHead scope="col">Repeat</TableHead>
              <TableHead scope="col">Next occurrence</TableHead>
              <TableHead scope="col">Status</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {visible.map((row) => (
              <TableRow
                className="catalog-inventory-row"
                data-expanded={row.id === selectedId}
                key={row.id}
              >
                <TableHead scope="row">
                  <Button
                    variant="tertiary"
                    className="catalog-name"
                    aria-pressed={row.id === selectedId}
                    aria-label={row.actionLabel}
                    disabled={busy}
                    onClick={() => onInspect(row.id)}
                  >
                    {row.name}
                  </Button>
                  <small>{row.kind}</small>
                </TableHead>
                <TableCell data-label="Repeat">{row.repeat}</TableCell>
                <TableCell
                  data-label={row.occurrenceLabel}
                  title={row.nextOccurrenceTitle}
                >
                  {row.nextOccurrence}
                </TableCell>
                <TableCell data-label="Status">
                  <Badge className="automation-status" data-state={row.status}>
                    {row.status[0]!.toUpperCase() + row.status.slice(1)}
                  </Badge>
                  {row.note && <small>{row.note}</small>}
                </TableCell>
              </TableRow>
            ))}
            {!visible.length && (
              <TableRow>
                <TableCell className="catalog-empty" colSpan={4}>
                  No loaded schedules match your search.
                </TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      </div>
    </div>
  );
}
