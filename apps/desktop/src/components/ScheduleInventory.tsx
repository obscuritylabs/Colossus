import { useState } from "react";
import { IconSearch } from "@tabler/icons-react";
import { occurrence, recurrence, type WorkflowSchedule } from "../workflows";
import "./catalog-inventory.css";

export function ScheduleInventory({
  items,
  selectedId,
  busy,
  onInspect,
}: {
  items: WorkflowSchedule[];
  selectedId: string | undefined;
  busy: boolean;
  onInspect: (id: string) => void;
}) {
  const [query, setQuery] = useState("");
  const visible = items.filter((item) =>
    `${item.record.task?.name || ""} ${item.record.schedule_id} ${item.record.workflow_name} ${recurrence(item.record)}`
      .toLocaleLowerCase()
      .includes(query.trim().toLocaleLowerCase()),
  );
  return (
    <div className="workflow-inventory">
      <p className="catalog-summary">
        {items.length} {items.length === 1 ? "schedule" : "schedules"}
      </p>
      <div className="catalog-search">
        <IconSearch size={18} aria-hidden="true" />
        <input
          aria-label="Search loaded schedules"
          placeholder="Search schedules"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
      </div>
      <div className="catalog-table-container">
        <table className="catalog-table" aria-label="Schedules">
          <colgroup>
            <col className="catalog-name-column" />
            <col />
            <col />
            <col />
          </colgroup>
          <thead>
            <tr>
              <th scope="col">Task or workflow</th>
              <th scope="col">Repeat</th>
              <th scope="col">Next occurrence</th>
              <th scope="col">Status</th>
            </tr>
          </thead>
          <tbody>
            {visible.map(({ record, controllable }) => (
              <tr
                className="catalog-inventory-row"
                data-expanded={record.schedule_id === selectedId}
                key={record.schedule_id}
              >
                <th scope="row">
                  <button
                    className="catalog-name"
                    aria-pressed={record.schedule_id === selectedId}
                    aria-label={`${record.task?.name || record.schedule_id} ${record.task ? "Agent task" : `${record.workflow_name} ${record.workflow_version}`}`}
                    disabled={busy}
                    onClick={() => onInspect(record.schedule_id)}
                  >
                    {record.task?.name || record.schedule_id}
                  </button>
                  <small>
                    {record.task
                      ? "Agent task"
                      : `${record.workflow_name} · ${record.workflow_version}`}
                  </small>
                </th>
                <td data-label="Repeat">{recurrence(record)}</td>
                <td
                  data-label={
                    record.enabled ? "Next occurrence" : "Retained occurrence"
                  }
                  title={occurrence(record.next_fire_at)}
                >
                  {new Date(record.next_fire_at).toLocaleString()}
                  {!record.enabled && <small>Retained while paused</small>}
                </td>
                <td data-label="Status">
                  <span
                    className="workflow-status"
                    data-state={
                      record.blocked_reason
                        ? "blocked"
                        : record.enabled
                          ? "enabled"
                          : "paused"
                    }
                  >
                    {record.blocked_reason
                      ? "Blocked"
                      : record.enabled
                        ? "Enabled"
                        : "Paused"}
                  </span>
                  {!controllable && <small>Legacy record</small>}
                </td>
              </tr>
            ))}
            {!visible.length && (
              <tr>
                <td className="catalog-empty" colSpan={4}>
                  No loaded schedules match your search.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
