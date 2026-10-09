import { DropdownSelect } from "./DropdownSelect";
import { calendarOccurrence, type WorkflowCalendar } from "../workflows";

export interface CalendarDraft {
  repeat: "daily" | "weekly";
  date: string;
  time: string;
  timezone: string;
  weekdays: number[];
}
export function defaultCalendarDraft(): CalendarDraft {
  const tomorrow = new Date();
  tomorrow.setDate(tomorrow.getDate() + 1);
  const date = `${tomorrow.getFullYear()}-${String(tomorrow.getMonth() + 1).padStart(2, "0")}-${String(tomorrow.getDate()).padStart(2, "0")}`;
  return {
    repeat: "daily",
    date,
    time: "09:00",
    timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
    weekdays: [1],
  };
}
export function prepareCalendar(draft: CalendarDraft): {
  calendar: WorkflowCalendar;
  starts_at: string;
} {
  if (draft.repeat === "weekly" && !draft.weekdays.length)
    throw new Error("Choose at least one weekday.");
  const calendar = {
    timezone: draft.timezone,
    time: draft.time,
    weekdays:
      draft.repeat === "daily" ? [] : [...draft.weekdays].sort((a, b) => a - b),
  };
  let date = draft.date;
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date))
    throw new Error("Choose the first date.");
  if (calendar.weekdays.length) {
    const candidate = new Date(`${date}T12:00:00Z`);
    for (let count = 0; count < 7; count++) {
      if (calendar.weekdays.includes(candidate.getUTCDay() || 7)) {
        date = candidate.toISOString().slice(0, 10);
        break;
      }
      candidate.setUTCDate(candidate.getUTCDate() + 1);
    }
  }
  return {
    calendar,
    starts_at: calendarOccurrence(`${date}T${draft.time}`, calendar.timezone),
  };
}
export function ScheduleTiming({
  value,
  onChange,
}: {
  value: CalendarDraft;
  onChange: (value: CalendarDraft) => void;
}) {
  const change = (next: Partial<CalendarDraft>) =>
    onChange({ ...value, ...next });
  const zones = [
    ...new Set([
      Intl.DateTimeFormat().resolvedOptions().timeZone,
      "America/New_York",
      "America/Los_Angeles",
      "Europe/London",
      "Europe/Berlin",
      "Asia/Tokyo",
      "UTC",
    ]),
  ];
  return (
    <div className="workflow-form">
      <div className="workflow-columns">
        <label>
          Repeat
          <DropdownSelect
            aria-label="Repeat"
            value={value.repeat}
            onChange={(event) =>
              change({ repeat: event.target.value as CalendarDraft["repeat"] })
            }
          >
            <option value="daily">Daily</option>
            <option value="weekly">Weekly</option>
          </DropdownSelect>
        </label>
        <label>
          Time
          <input
            type="time"
            required
            value={value.time}
            onChange={(event) => change({ time: event.target.value })}
          />
        </label>
      </div>
      {value.repeat === "weekly" && (
        <fieldset className="workflow-weekdays">
          <legend>Run on</legend>
          {["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].map(
            (day, index) => (
              <label key={day}>
                <input
                  type="checkbox"
                  checked={value.weekdays.includes(index + 1)}
                  onChange={(event) =>
                    change({
                      weekdays: event.target.checked
                        ? [...value.weekdays, index + 1].sort()
                        : value.weekdays.filter((day) => day !== index + 1),
                    })
                  }
                />
                <span>{day}</span>
              </label>
            ),
          )}
        </fieldset>
      )}
      <div className="workflow-columns">
        <label>
          Time zone
          <DropdownSelect
            aria-label="Time zone"
            value={zones.includes(value.timezone) ? value.timezone : "custom"}
            onChange={(event) =>
              change({
                timezone:
                  event.target.value === "custom" ? "" : event.target.value,
              })
            }
          >
            {zones.map((zone) => (
              <option key={zone} value={zone}>
                {zone}
              </option>
            ))}
            <option value="custom">Other timezone…</option>
          </DropdownSelect>
        </label>
        <label>
          First date
          <input
            type="date"
            required
            value={value.date}
            onChange={(event) => change({ date: event.target.value })}
          />
        </label>
      </div>
      {!zones.includes(value.timezone) && (
        <label>
          IANA timezone
          <input
            value={value.timezone}
            required
            maxLength={128}
            placeholder="America/Chicago"
            onChange={(event) => change({ timezone: event.target.value })}
          />
        </label>
      )}
      <p className="workflow-help">
        Keeps this local time through daylight saving changes. Missing times are
        skipped; repeated times run once. Weekly schedules start on the first
        selected weekday on or after the first date.
      </p>
    </div>
  );
}
