import { AutomationInventory } from "@colossus/ui/automations";
import { occurrence, recurrence, type WorkflowSchedule } from "../workflows";
import "@colossus/ui/styles/automations.css";

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
  return (
    <AutomationInventory
      selectedId={selectedId}
      busy={busy}
      onInspect={onInspect}
      rows={items.map(({ record, controllable }) => ({
        id: record.schedule_id,
        name: record.task?.name || record.schedule_id,
        kind: record.task
          ? "Agent task"
          : `${record.workflow_name} · ${record.workflow_version}`,
        actionLabel: `${record.task?.name || record.schedule_id} ${record.task ? "Agent task" : `${record.workflow_name} ${record.workflow_version}`}`,
        searchText: `${record.task?.name || ""} ${record.schedule_id} ${record.workflow_name} ${recurrence(record)}`,
        repeat: recurrence(record),
        nextOccurrence: (
          <>
            {new Date(record.next_fire_at).toLocaleString()}
            {!record.enabled && <small>Retained while paused</small>}
          </>
        ),
        nextOccurrenceTitle: occurrence(record.next_fire_at),
        occurrenceLabel: record.enabled
          ? "Next occurrence"
          : "Retained occurrence",
        status: record.blocked_reason
          ? "blocked"
          : record.enabled
            ? "enabled"
            : "paused",
        ...(!controllable ? { note: "Legacy record" } : {}),
      }))}
    />
  );
}
