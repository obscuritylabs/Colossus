import { useId, useState } from "react";
import { DropdownSelect } from "./DropdownSelect";
import { scheduleInputs } from "../workflows";

/** Simple schema fields and a lossless JSON fallback share one exact input snapshot. */
export function WorkflowInputs({
  schema,
  value,
  onChange,
}: {
  schema: Record<string, unknown> | null;
  value: string;
  onChange: (value: string) => void;
}) {
  const id = useId();
  const [json, setJson] = useState(false);
  const properties = schema?.properties;
  const fields =
    properties && typeof properties === "object" && !Array.isArray(properties)
      ? Object.entries(properties as Record<string, unknown>)
      : [];
  let data: Record<string, unknown> | null = null;
  try {
    data = scheduleInputs(value);
  } catch {
    /* Keep invalid edits in the JSON editor. */
  }
  const simple =
    fields.length > 0 &&
    fields.length <= 32 &&
    fields.every(([, field]) => {
      if (!field || typeof field !== "object" || Array.isArray(field))
        return false;
      const type = (field as Record<string, unknown>).type;
      return ["string", "number", "integer", "boolean"].includes(String(type));
    });
  const required = Array.isArray(schema?.required) ? schema.required : [];
  function update(key: string, next: unknown, remove = false) {
    const copy = { ...data };
    if (remove) delete copy[key];
    else copy[key] = next;
    onChange(JSON.stringify(copy, null, 2));
  }
  return (
    <section className="workflow-inputs" aria-label="Workflow inputs">
      <div className="workflow-detail-header">
        <h4>Inputs</h4>
        {simple && (
          <button
            className="button secondary compact"
            type="button"
            onClick={() => setJson(!json)}
          >
            {json ? "Use fields" : "Edit JSON"}
          </button>
        )}
      </div>
      {simple && data && !json ? (
        <div className="workflow-form">
          {fields.map(([key, raw]) => {
            const field = raw as Record<string, unknown>;
            const label = typeof field.title === "string" ? field.title : key;
            const current = data?.[key];
            const enums = Array.isArray(field.enum) ? field.enum : null;
            return (
              <label key={key} htmlFor={`${id}-${key}`}>
                {label}
                {required.includes(key) ? " *" : ""}
                {enums ? (
                  <DropdownSelect
                    id={`${id}-${key}`}
                    value={current === undefined ? "" : JSON.stringify(current)}
                    onChange={(event) =>
                      update(
                        key,
                        event.target.value
                          ? JSON.parse(event.target.value)
                          : undefined,
                        !event.target.value,
                      )
                    }
                  >
                    <option value="">Choose a value</option>
                    {enums.map((option) => (
                      <option
                        key={JSON.stringify(option)}
                        value={JSON.stringify(option)}
                      >
                        {String(option)}
                      </option>
                    ))}
                  </DropdownSelect>
                ) : field.type === "boolean" ? (
                  <DropdownSelect
                    id={`${id}-${key}`}
                    value={current === undefined ? "" : String(current)}
                    onChange={(event) =>
                      update(
                        key,
                        event.target.value === "true",
                        !event.target.value,
                      )
                    }
                  >
                    <option value="">Not supplied</option>
                    <option value="true">Yes</option>
                    <option value="false">No</option>
                  </DropdownSelect>
                ) : (
                  <input
                    id={`${id}-${key}`}
                    type={field.type === "string" ? "text" : "number"}
                    step={field.type === "integer" ? "1" : "any"}
                    value={current === undefined ? "" : String(current)}
                    required={required.includes(key)}
                    onChange={(event) => {
                      const text = event.target.value;
                      update(
                        key,
                        field.type === "string" ? text : Number(text),
                        text === "" &&
                          (field.type !== "string" || !required.includes(key)),
                      );
                    }}
                  />
                )}
                {typeof field.description === "string" && (
                  <small>{field.description}</small>
                )}
              </label>
            );
          })}
        </div>
      ) : (
        <label>
          Inputs (JSON object)
          <textarea
            rows={7}
            spellCheck={false}
            value={value}
            onChange={(event) => onChange(event.target.value)}
          />
        </label>
      )}
      <p className="workflow-help">
        The runtime validates these inputs against the workflow schema before
        starting a run.
      </p>
    </section>
  );
}
