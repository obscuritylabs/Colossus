import { DropdownSelect } from "./DropdownSelect";
import { useState } from "react";

type ObjectValue = Record<string, unknown>;
type Field = { key: string; label: string } & (
  | {
      type: "text" | "reference" | "number" | "boolean" | "lines";
      optional?: boolean;
    }
  | { type: "select"; options: readonly string[] }
  | { type: "map"; fields?: readonly Field[]; reference?: boolean }
  | { type: "rows"; fields: readonly Field[] }
  | { type: "object"; fields: readonly Field[]; optional?: boolean }
);
const identityFields: readonly Field[] = [
  { key: "issuer", label: "Exact issuer", type: "text" },
  { key: "subject", label: "Exact subject", type: "text" },
];
const trustFields: readonly Field[] = [
  {
    key: "mode",
    label: "Signature policy",
    type: "select",
    options: ["required", "optional", "disabled"],
  },
  { key: "publicKeys", label: "PEM public key paths", type: "lines" },
  {
    key: "identities",
    label: "Keyless identities",
    type: "rows",
    fields: identityFields,
  },
  {
    key: "trustRootPath",
    label: "Offline Sigstore trust root path",
    type: "text",
    optional: true,
  },
];
const registryFields: readonly Field[] = [
  { key: "origin", label: "Exact registry origin", type: "text" },
  { key: "trustProfile", label: "Trust profile", type: "text" },
  {
    key: "tokenOrigins",
    label: "Allowed token-service origins",
    type: "lines",
  },
  {
    key: "blobRedirectOrigins",
    label: "Allowed blob-redirect origins",
    type: "lines",
  },
  {
    key: "caBundlePath",
    label: "Registry CA bundle path",
    type: "text",
    optional: true,
  },
  {
    key: "tokenCaBundlePaths",
    label: "Token origin → CA bundle path",
    type: "map",
  },
  {
    key: "blobRedirectCaBundlePaths",
    label: "Blob origin → CA bundle path",
    type: "map",
  },
  {
    key: "allowNonPublic",
    label: "Allow explicitly configured private or loopback networks",
    type: "boolean",
  },
];
const oauthFields: readonly Field[] = [
  { key: "clientId", label: "OAuth client ID", type: "text" },
  {
    key: "clientSecretReference",
    label: "Client secret credential reference",
    type: "reference",
    optional: true,
  },
  {
    key: "callbackPort",
    label: "Registered loopback callback port",
    type: "number",
  },
  { key: "scopes", label: "OAuth scopes", type: "lines" },
];
const mcpFields: readonly Field[] = [
  {
    key: "enabled",
    label: "Explicitly enable this MCP server",
    type: "boolean",
  },
  {
    key: "allowedTools",
    label: "Allowed tool names (or a sole *)",
    type: "lines",
  },
  {
    key: "environment",
    label: "Environment credential references",
    type: "map",
    reference: true,
  },
  {
    key: "credentialHeaders",
    label: "HTTP credential header overlays",
    type: "map",
    fields: [
      { key: "reference", label: "Credential reference", type: "reference" },
      {
        key: "scheme",
        label: "Authentication scheme",
        type: "text",
        optional: true,
      },
    ],
  },
  {
    key: "oauth",
    label: "OAuth overlay",
    type: "object",
    fields: oauthFields,
    optional: true,
  },
  { key: "allowStateless", label: "Allow stateless HTTP", type: "boolean" },
  {
    key: "timeoutMs",
    label: "Timeout (milliseconds)",
    type: "number",
    optional: true,
  },
  {
    key: "maxOutputBytes",
    label: "Maximum output bytes",
    type: "number",
    optional: true,
  },
];

function object(value: unknown): ObjectValue {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as ObjectValue)
    : {};
}

export function PluginConfigurationEditor({
  id,
  value,
  onChange,
}: {
  id: string;
  value: unknown;
  onChange: (value: unknown) => void;
}) {
  const fields =
    id === "plugins.trustProfiles"
      ? trustFields
      : id === "plugins.registries"
        ? registryFields
        : mcpFields;
  const initial =
    id === "plugins.trustProfiles"
      ? { mode: "required" }
      : id === "plugins.registries"
        ? { auth: { kind: "anonymous" }, trustProfile: "default" }
        : { enabled: false, allowedTools: [] };
  return (
    <div className="plugin-configuration">
      <p>
        {id === "plugins.mcpServers"
          ? "Keys must be qualified plugin/server names. Adding credentials never enables a server."
          : "Profiles contain exact origins, public identities, paths, and credential references only—not stored secrets."}
      </p>
      <NamedMap
        label={id}
        value={value}
        initial={initial}
        onChange={onChange}
        render={(entry, change) => (
          <>
            {id === "plugins.registries" && (
              <RegistryAuth
                value={entry.auth}
                onChange={(auth) => change({ ...entry, auth })}
              />
            )}
            <ObjectFields
              label={id}
              value={entry}
              fields={fields}
              onChange={change}
            />
          </>
        )}
      />
    </div>
  );
}

function RegistryAuth({
  value,
  onChange,
}: {
  value: unknown;
  onChange: (value: ObjectValue) => void;
}) {
  const entry = object(value);
  const kind = String(entry.kind ?? "anonymous");
  return (
    <fieldset>
      <legend>Registry authentication</legend>
      <label>
        Credential source
        <DropdownSelect
          value={kind}
          onChange={(event) => onChange({ kind: event.target.value })}
        >
          {["anonymous", "bearer", "basic", "docker"].map((option) => (
            <option value={option} key={option}>
              {option}
            </option>
          ))}
        </DropdownSelect>
      </label>
      {kind === "docker" ? (
        <>
          <p>
            Docker credentials are consulted only when explicitly selected.
            Every helper requires an exact configured executable and a
            subprocess permit.
          </p>
          <ObjectFields
            label="Docker"
            value={entry}
            onChange={onChange}
            fields={[
              {
                key: "configPath",
                label: "Docker config path",
                type: "text",
                optional: true,
              },
              {
                key: "helperExecutables",
                label: "Helper suffix → exact executable",
                type: "map",
              },
            ]}
          />
        </>
      ) : kind === "bearer" || kind === "basic" ? (
        <ObjectFields
          label="Registry credentials"
          value={entry}
          onChange={onChange}
          fields={[
            ...(kind === "basic"
              ? [{ key: "username", label: "Username", type: "text" } as const]
              : []),
            {
              key: "credentialReference",
              label: "Credential reference",
              type: "reference",
            },
          ]}
        />
      ) : null}
    </fieldset>
  );
}

function ObjectFields({
  label,
  value,
  fields,
  onChange,
}: {
  label: string;
  value: ObjectValue;
  fields: readonly Field[];
  onChange: (value: ObjectValue) => void;
}) {
  return (
    <>
      {fields.map((field) => {
        const entry = value[field.key];
        const change = (next: unknown) =>
          onChange({ ...value, [field.key]: next });
        const key = `${label}.${field.key}`;
        if (field.type === "object")
          return (
            <fieldset key={key}>
              <legend>{field.label}</legend>
              {field.optional && (
                <label>
                  <input
                    type="checkbox"
                    checked={entry != null}
                    onChange={(event) =>
                      change(event.target.checked ? {} : null)
                    }
                  />{" "}
                  Configure {field.label}
                </label>
              )}
              {(!field.optional || entry != null) && (
                <ObjectFields
                  label={key}
                  value={object(entry)}
                  fields={field.fields}
                  onChange={change}
                />
              )}
            </fieldset>
          );
        if (field.type === "map")
          return (
            <NamedMap
              key={key}
              label={field.label}
              value={entry}
              initial={field.fields ? {} : ""}
              onChange={change}
              render={(row, update) => (
                <ObjectFields
                  label={key}
                  value={row}
                  fields={field.fields ?? []}
                  onChange={update}
                />
              )}
              scalar={!field.fields}
              reference={field.reference}
            />
          );
        if (field.type === "rows") {
          const rows: unknown[] = Array.isArray(entry) ? entry : [];
          return (
            <fieldset key={key}>
              <legend>{field.label}</legend>
              {rows.map((row, index) => (
                <fieldset key={index}>
                  <legend>
                    {field.label} {index + 1}
                  </legend>
                  <ObjectFields
                    label={`${key}.${index}`}
                    value={object(row)}
                    fields={field.fields}
                    onChange={(next) =>
                      change(
                        rows.map((candidate, at) =>
                          at === index ? next : candidate,
                        ),
                      )
                    }
                  />
                  <button
                    type="button"
                    onClick={() => change(rows.filter((_, at) => at !== index))}
                  >
                    Remove identity
                  </button>
                </fieldset>
              ))}
              <button type="button" onClick={() => change([...rows, {}])}>
                Add identity
              </button>
            </fieldset>
          );
        }
        if (field.type === "boolean")
          return (
            <label key={key}>
              <input
                type="checkbox"
                checked={entry === true}
                onChange={(event) => change(event.target.checked)}
              />{" "}
              {field.label}
            </label>
          );
        if (field.type === "select")
          return (
            <label key={key}>
              {field.label}
              <DropdownSelect
                value={String(entry ?? field.options[0])}
                onChange={(event) => change(event.target.value)}
              >
                {field.options.map((option) => (
                  <option value={option} key={option}>
                    {option}
                  </option>
                ))}
              </DropdownSelect>
            </label>
          );
        if (field.type === "lines")
          return (
            <label key={key}>
              {field.label}
              <textarea
                rows={3}
                value={Array.isArray(entry) ? entry.join("\n") : ""}
                placeholder="One exact value per line"
                onChange={(event) =>
                  change(
                    event.target.value
                      .split("\n")
                      .map((line) => line.trim())
                      .filter(Boolean),
                  )
                }
              />
            </label>
          );
        return (
          <label key={key}>
            {field.label}
            <input
              type={field.type === "number" ? "number" : "text"}
              min={field.type === "number" ? 1 : undefined}
              value={
                typeof entry === "string" || typeof entry === "number"
                  ? entry
                  : ""
              }
              placeholder={
                field.type === "reference"
                  ? "host:credential-id or env:VARIABLE"
                  : undefined
              }
              onChange={(event) =>
                change(
                  event.target.value === "" && field.optional
                    ? null
                    : field.type === "number"
                      ? Number(event.target.value)
                      : event.target.value,
                )
              }
            />
          </label>
        );
      })}
    </>
  );
}

function NamedMap({
  label,
  value,
  initial,
  onChange,
  render,
  scalar = false,
  reference = false,
}: {
  label: string;
  value: unknown;
  initial: unknown;
  onChange: (value: ObjectValue) => void;
  render: (
    value: ObjectValue,
    onChange: (value: ObjectValue) => void,
  ) => React.ReactNode;
  scalar?: boolean;
  reference?: boolean | undefined;
}) {
  const [name, setName] = useState("");
  const entries = object(value);
  function add() {
    const key = name.trim();
    if (key && !Object.hasOwn(entries, key)) {
      onChange({ ...entries, [key]: initial });
      setName("");
    }
  }
  return (
    <fieldset>
      <legend>{label}</legend>
      {Object.entries(entries).map(([key, entry]) => (
        <details key={key} open>
          <summary>{key}</summary>
          {scalar ? (
            <label>
              {key}
              <input
                value={typeof entry === "string" ? entry : ""}
                placeholder={
                  reference ? "host:credential-id or env:VARIABLE" : undefined
                }
                onChange={(event) =>
                  onChange({ ...entries, [key]: event.target.value })
                }
              />
            </label>
          ) : (
            render(object(entry), (next) =>
              onChange({ ...entries, [key]: next }),
            )
          )}
          <button
            type="button"
            onClick={() =>
              onChange(
                Object.fromEntries(
                  Object.entries(entries).filter(
                    ([candidate]) => candidate !== key,
                  ),
                ),
              )
            }
          >
            Remove {key}
          </button>
        </details>
      ))}
      <label>
        New {label} key
        <input value={name} onChange={(event) => setName(event.target.value)} />
      </label>
      <button
        type="button"
        disabled={!name.trim() || Object.hasOwn(entries, name.trim())}
        onClick={add}
      >
        Add {label} entry
      </button>
    </fieldset>
  );
}
