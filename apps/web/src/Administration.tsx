import { useEffect, useMemo, useState } from "react";
import { Button, DropdownSelect, TextInput } from "@colossus/ui";
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
import { IconPlus, IconSettings, IconShieldLock } from "@tabler/icons-react";
import { request } from "./api";
import type {
  AdminSettings,
  AuthConfig,
  Classification,
  Project,
  User,
} from "./control-api";
import { errorMessage, useResource } from "./resources";
import { LoadState } from "./Home";
import { ProjectEditor, SectionTabs } from "./Projects";
import { globalHref, type AdminView } from "./routes";
export function Administration({
  authConfig,
  onRefresh,
  onSettings,
  view,
  onView,
}: {
  authConfig: AuthConfig | null;
  onRefresh: () => void;
  onSettings: (settings: AdminSettings) => void;
  view?: AdminView;
  onView?: (value: AdminView) => void;
}) {
  const [localTab, setTab] = useState("users");
  const tab = view ?? localTab;
  return (
    <section className="control-page">
      <header className="page-heading">
        <div>
          <span className="eyebrow">Administration</span>
          <h1>Control Plane administration</h1>
          <p>Manage identities, project structure, and environment settings.</p>
        </div>
      </header>
      <SectionTabs
        items={[
          { id: "users", label: "Users" },
          { id: "projects", label: "Projects" },
          { id: "settings", label: "Authentication & settings" },
        ]}
        current={tab}
        onChange={(value) =>
          onView ? onView(value as AdminView) : setTab(value)
        }
        hrefFor={(value) => globalHref("admin", undefined, value as AdminView)}
      />
      {tab === "users" ? (
        <Users authConfig={authConfig} />
      ) : tab === "projects" ? (
        <AdminProjects onRefresh={onRefresh} />
      ) : (
        <EnvironmentSettings onSaved={onSettings} />
      )}
    </section>
  );
}
function Users({ authConfig }: { authConfig: AuthConfig | null }) {
  const resource = useResource<{ users: User[]; next_cursor?: string | null }>(
      "/api/admin/users",
    ),
    [extra, setExtra] = useState<User[]>([]),
    [cursor, setCursor] = useState<string | null>(null),
    [selected, setSelected] = useState<User | "new" | null>(null),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false);
  useEffect(() => {
    setExtra([]);
    setCursor(resource.data?.next_cursor ?? null);
  }, [resource.data]);
  const columns = useMemo<DataTableColumn<User>[]>(
    () => [
      {
        id: "name",
        label: "User",
        value: (row) => row.display_name,
        cell: (row) => (
          <button
            type="button"
            className="catalog-name"
            onClick={() => setSelected(row)}
          >
            {row.display_name}
          </button>
        ),
      },
      {
        id: "identity",
        label: "Login identities",
        value: (row) => row.identities?.map((id) => id.label).join(" ") ?? "",
        cell: (row) => (
          <span>
            {row.identities?.map((id) => id.label).join(" · ") ||
              "Provisioned identity"}
          </span>
        ),
      },
      {
        id: "active",
        label: "Status",
        value: (row) => (row.active ? "Active" : "Disabled"),
        cell: (row) => (
          <span
            className={`status ${row.active ? "status-completed" : "status-cancelled"}`}
          >
            {row.active ? "Active" : "Disabled"}
          </span>
        ),
      },
      {
        id: "role",
        label: "Administration",
        value: (row) => (row.is_admin ? "Administrator" : "Member"),
        cell: (row) => <span>{row.is_admin ? "Administrator" : "Member"}</span>,
      },
    ],
    [],
  );
  return (
    <>
      <header className="catalog-heading">
        <div>
          <h2>Users</h2>
          <p>
            Provision login identities and control access to the Control Plane.
          </p>
        </div>
        <Button variant="primary" onClick={() => setSelected("new")}>
          <IconPlus size={16} aria-hidden="true" />
          Create user
        </Button>
      </header>
      <LoadState {...resource} retry={resource.refresh} />
      {error ? (
        <div className="alert" role="alert">
          {error}
        </div>
      ) : null}
      {selected ? (
        <UserEditor
          key={
            selected === "new" ? "new" : `${selected.id}:${selected.revision}`
          }
          user={selected === "new" ? undefined : selected}
          authConfig={authConfig}
          onCancel={() => setSelected(null)}
          onSaved={() => {
            setSelected(null);
            resource.refresh();
          }}
        />
      ) : null}
      <DataTable
        data={[...(resource.data?.users ?? []), ...extra]}
        columns={columns}
        getRowId={(row) => row.id}
        search={{ columnId: "name", label: "Search loaded users" }}
        label="Control Plane users"
        loading={resource.loading}
        empty="No users found."
      />
      {cursor ? (
        <div className="load-more">
          <p className="field-help">
            Search and sorting apply to loaded users.
          </p>
          <Button
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              try {
                const data = await request<{
                  users: User[];
                  next_cursor?: string | null;
                }>(`/api/admin/users?after=${encodeURIComponent(cursor)}`);
                setExtra((value) => [...value, ...data.users]);
                setCursor(data.next_cursor ?? null);
              } catch (e) {
                setError(errorMessage(e));
              } finally {
                setBusy(false);
              }
            }}
          >
            Load more users
          </Button>
        </div>
      ) : null}
    </>
  );
}
function UserEditor({
  user,
  authConfig,
  onCancel,
  onSaved,
}: {
  user?: User | undefined;
  authConfig: AuthConfig | null;
  onCancel: () => void;
  onSaved: () => void;
}) {
  const [name, setName] = useState(user?.display_name ?? ""),
    [email, setEmail] = useState(user?.email ?? ""),
    [username, setUsername] = useState(""),
    [password, setPassword] = useState(""),
    [subject, setSubject] = useState(""),
    [active, setActive] = useState(user?.active ?? true),
    [admin, setAdmin] = useState(user?.is_admin ?? false),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false),
    [reset, setReset] = useState(false);
  const localIdentity = user?.identities?.some((item) => item.kind === "local");
  async function save() {
    setBusy(true);
    setError("");
    try {
      if (reset && user)
        await request(
          `/api/admin/users/${encodeURIComponent(user.id)}/password`,
          { revision: user.revision, new_password: password },
        );
      else
        await request(
          user
            ? `/api/admin/users/${encodeURIComponent(user.id)}`
            : "/api/admin/users",
          user
            ? {
                revision: user.revision,
                display_name: name.trim(),
                active,
                is_admin: admin,
              }
            : {
                display_name: name.trim(),
                email: email.trim() || null,
                is_admin: admin,
                ...(username.trim()
                  ? { username: username.trim(), password }
                  : {}),
                ...(subject.trim() ? { oidc_subject: subject.trim() } : {}),
              },
          undefined,
          user && !reset ? "PATCH" : undefined,
        );
      onSaved();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
      setPassword("");
    }
  }
  return (
    <form
      className="management-form control-card"
      onSubmit={(e) => {
        e.preventDefault();
        void save();
      }}
    >
      <header>
        <h3>
          {reset
            ? "Reset local password"
            : user
              ? `Edit ${user.display_name}`
              : "Create user"}
        </h3>
        <Button onClick={onCancel}>Close</Button>
      </header>
      {error ? (
        <div className="alert" role="alert">
          {error}
        </div>
      ) : null}
      {reset ? (
        <>
          <p className="field-help">
            Resetting the password invalidates this user’s existing sessions.
          </p>
          <label>
            <span>New password</span>
            <TextInput
              type="password"
              autoComplete="new-password"
              required
              minLength={15}
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
          </label>
        </>
      ) : (
        <>
          <label>
            <span>Display name</span>
            <TextInput
              required
              maxLength={200}
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </label>
          {!user ? (
            <>
              <label>
                <span>Email · optional</span>
                <TextInput
                  type="email"
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                  autoComplete="off"
                />
              </label>
              {authConfig?.local_enabled ? (
                <div className="form-pair">
                  <label>
                    <span>Local username</span>
                    <TextInput
                      value={username}
                      onChange={(e) => setUsername(e.target.value)}
                      autoComplete="off"
                    />
                  </label>
                  <label>
                    <span>Local password</span>
                    <TextInput
                      type="password"
                      minLength={15}
                      required={Boolean(username.trim())}
                      value={password}
                      onChange={(e) => setPassword(e.target.value)}
                      autoComplete="new-password"
                    />
                  </label>
                </div>
              ) : null}
              {authConfig?.oidc ? (
                <label>
                  <span>OIDC subject</span>
                  <TextInput
                    value={subject}
                    onChange={(e) => setSubject(e.target.value)}
                    autoComplete="off"
                  />
                  <small>
                    Exact subject from {authConfig.oidc.label}. Emails do not
                    automatically link identities.
                  </small>
                </label>
              ) : null}
              <p className="field-help">
                Provide at least one enabled login identity.
              </p>
            </>
          ) : (
            <label className="checkbox-field">
              <input
                type="checkbox"
                checked={active}
                onChange={(e) => setActive(e.target.checked)}
              />
              Active user<small>Changing this retires existing sessions.</small>
            </label>
          )}
          <label className="checkbox-field">
            <input
              type="checkbox"
              checked={admin}
              onChange={(e) => setAdmin(e.target.checked)}
            />
            Control Plane administrator
          </label>
          <p className="field-help">
            Administrators can read and manage projects. Running tasks and
            approving actions still require an explicit project role.
          </p>
        </>
      )}
      <div className="form-actions">
        <Button
          variant="primary"
          type="submit"
          disabled={
            busy ||
            (!reset && !name.trim()) ||
            (!user && !username.trim() && !subject.trim())
          }
        >
          {busy ? "Saving…" : reset ? "Reset password" : "Save user"}
        </Button>
        {user && localIdentity && authConfig?.local_enabled && !reset ? (
          <Button onClick={() => setReset(true)}>Reset local password</Button>
        ) : null}
        <Button onClick={onCancel}>Cancel</Button>
      </div>
    </form>
  );
}
function AdminProjects({ onRefresh }: { onRefresh: () => void }) {
  const resource = useResource<{
      projects: Project[];
      next_cursor?: string | null;
    }>("/api/admin/projects"),
    [selected, setSelected] = useState<Project | "new" | null>(null),
    [extra, setExtra] = useState<Project[]>([]),
    [cursor, setCursor] = useState<string | null>(null),
    [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  useEffect(() => {
    setExtra([]);
    setCursor(resource.data?.next_cursor ?? null);
  }, [resource.data]);
  const projects = [...(resource.data?.projects ?? []), ...extra];
  return (
    <>
      <header className="catalog-heading">
        <div>
          <h2>Project hierarchy</h2>
          <p>Organizational nesting does not grant inherited membership.</p>
        </div>
        <Button variant="primary" onClick={() => setSelected("new")}>
          <IconPlus size={16} aria-hidden="true" />
          Create project
        </Button>
      </header>
      <LoadState {...resource} retry={resource.refresh} />
      {error ? (
        <div className="alert" role="alert">
          {error}
        </div>
      ) : null}
      {selected ? (
        <ProjectEditor
          key={
            selected === "new" ? "new" : `${selected.id}:${selected.revision}`
          }
          {...(selected === "new" ? {} : { project: selected })}
          projects={projects}
          onCancel={() => setSelected(null)}
          onSaved={() => {
            setSelected(null);
            resource.refresh();
            onRefresh();
          }}
        />
      ) : null}
      <div className="access-list">
        {projects.map((project) => (
          <div key={project.id}>
            <span>
              <strong>{project.name}</strong>
              <small>
                {project.parent_project_id
                  ? `Within ${projects.find((item) => item.id === project.parent_project_id)?.name ?? project.parent_project_id}`
                  : "Top-level project"}
                {project.archived ? " · Archived" : ""}
              </small>
            </span>
            <Button onClick={() => setSelected(project)}>Edit project</Button>
          </div>
        ))}
      </div>
      {cursor ? (
        <Button
          disabled={busy}
          onClick={async () => {
            setBusy(true);
            try {
              const data = await request<{
                projects: Project[];
                next_cursor?: string | null;
              }>(`/api/admin/projects?after=${encodeURIComponent(cursor)}`);
              setExtra((value) => [...value, ...data.projects]);
              setCursor(data.next_cursor ?? null);
            } catch (e) {
              setError(errorMessage(e));
            } finally {
              setBusy(false);
            }
          }}
        >
          Load more projects
        </Button>
      ) : null}
    </>
  );
}
function EnvironmentSettings({
  onSaved,
}: {
  onSaved: (settings: AdminSettings) => void;
}) {
  const resource = useResource<AdminSettings>("/api/admin/settings"),
    [draft, setDraft] = useState<Classification | null>(null),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  useEffect(() => {
    if (resource.data) setDraft(resource.data.classification);
  }, [resource.data]);
  return (
    <>
      <LoadState {...resource} retry={resource.refresh} />
      {resource.data ? (
        <section className="control-card">
          <header>
            <h3>
              <IconShieldLock size={17} aria-hidden="true" />
              Authentication
            </h3>
          </header>
          <dl className="stat-list">
            <div>
              <dt>Local login</dt>
              <dd>
                {resource.data.auth.local_enabled ? "Enabled" : "Disabled"}
              </dd>
            </div>
            <div>
              <dt>OIDC provider</dt>
              <dd>{resource.data.auth.oidc_label ?? "Not configured"}</dd>
            </div>
          </dl>
          <p className="field-help">
            Authentication providers are configured by the deployment operator.
            Credentials and client secrets stay outside this interface.
          </p>
        </section>
      ) : null}
      {draft && resource.data ? (
        <section className="control-card">
          <header>
            <h3>
              <IconSettings size={17} aria-hidden="true" />
              Classification banner
            </h3>
          </header>
          <form
            className="management-form"
            onSubmit={async (event) => {
              event.preventDefault();
              setBusy(true);
              setError("");
              setNotice("");
              try {
                const result = await request<AdminSettings>(
                  "/api/admin/settings",
                  { revision: resource.data!.revision, classification: draft },
                  undefined,
                  "PATCH",
                );
                onSaved(result);
                resource.refresh();
                setNotice("Environment settings saved.");
              } catch (e) {
                setError(errorMessage(e));
              } finally {
                setBusy(false);
              }
            }}
          >
            {error ? (
              <div className="alert" role="alert">
                {error}
              </div>
            ) : null}
            {notice ? (
              <p role="status" className="notice">
                {notice}
              </p>
            ) : null}
            <label className="checkbox-field">
              <input
                type="checkbox"
                checked={draft.enabled}
                onChange={(e) =>
                  setDraft({ ...draft, enabled: e.target.checked })
                }
              />
              Display classification banner
            </label>
            <label>
              <span>Classification text</span>
              <TextInput
                required={draft.enabled}
                maxLength={160}
                value={draft.text}
                onChange={(e) => setDraft({ ...draft, text: e.target.value })}
              />
            </label>
            <div className="form-pair">
              <label>
                <span>Tone</span>
                <DropdownSelect
                  value={draft.tone}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      tone: e.target.value as Classification["tone"],
                    })
                  }
                >
                  <option value="neutral">Neutral</option>
                  <option value="info">Information</option>
                  <option value="warning">Warning</option>
                  <option value="danger">Danger</option>
                </DropdownSelect>
              </label>
              <label>
                <span>Placement</span>
                <DropdownSelect
                  value={draft.position}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      position: e.target.value as Classification["position"],
                    })
                  }
                >
                  <option value="top">Top</option>
                  <option value="top_and_bottom">Top and bottom</option>
                </DropdownSelect>
              </label>
            </div>
            {draft.enabled ? (
              <div
                className={`control-classification classification-${draft.tone}`}
                aria-label="Banner preview"
              >
                {draft.text || "Classification preview"}
              </div>
            ) : null}
            <Button variant="primary" type="submit" disabled={busy}>
              {busy ? "Saving…" : "Save environment settings"}
            </Button>
          </form>
        </section>
      ) : null}
    </>
  );
}
