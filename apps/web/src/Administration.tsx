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
  PublicSettings,
} from "./control-api";
import { errorMessage, useResource } from "./resources";
import { LoadState } from "./Home";
import { ProjectEditor } from "./Projects";
import {
  adminUserHref,
  connectionSettingsSections,
  type ConnectionSettingsSection,
  type AdminView,
  type AdminUserView,
} from "./routes";
import { AdminUserDetails } from "./AdminUserDetails";
import { UserEditor } from "./AdminUserEditor";
import { GlobalConfiguration } from "./GlobalConfiguration";
import { EnvironmentAppearance } from "./Appearance";
import { projectCreateHref } from "./routes";
import { RouteLink } from "./navigation";
import { AdminAnnouncements } from "./AdminAnnouncements";
import { classificationColorVariables } from "@colossus/ui/lib/classification-colors";
export function Administration({
  authConfig,
  onRefresh,
  onSettings,
  view,
  userId,
  userView,
  userProject,
  currentUserId,
  identity,
  projects,
  selectedProject,
}: {
  authConfig: AuthConfig | null;
  onRefresh: () => void;
  onSettings: (settings: PublicSettings) => void;
  userId?: string | undefined;
  userView?: AdminUserView | undefined;
  userProject?: string | undefined;
  currentUserId: string;
  identity: string;
  projects: Project[];
  selectedProject: string;
  view?: AdminView;
  onView?: (value: AdminView) => void;
}) {
  const tab = view ?? "users";
  if (userId)
    return (
      <AdminUserDetails
        key={`${currentUserId}:${userId}`}
        userId={userId}
        currentUserId={currentUserId}
        view={userView ?? "overview"}
        project={userProject}
        authConfig={authConfig}
        onChanged={onRefresh}
      />
    );
  return (
    <section className="control-page">
      <header
        className={
          (connectionSettingsSections as readonly string[]).includes(tab)
            ? "sr-only"
            : "page-heading"
        }
      >
        <div>
          <span className="eyebrow">Administration</span>
          <h1>Control Plane administration</h1>
          <p>
            Manage identities, projects, announcements, and environment
            settings.
          </p>
        </div>
      </header>
      {tab === "users" ? (
        <Users authConfig={authConfig} />
      ) : tab === "projects" ? (
        <AdminProjects onRefresh={onRefresh} />
      ) : tab === "announcements" ? (
        <AdminAnnouncements />
      ) : tab === "appearance" ? (
        <EnvironmentAppearance onSaved={onSettings} />
      ) : (connectionSettingsSections as readonly string[]).includes(tab) ? (
        <GlobalConfiguration
          section={tab as ConnectionSettingsSection}
          identity={identity}
          projects={projects}
          selectedProject={selectedProject}
          onSaved={onRefresh}
        />
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
    [creating, setCreating] = useState(false),
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
        hideable: false,
        value: (row) =>
          [
            row.display_name,
            row.email,
            ...(row.identities?.map(
              (identity) => identity.username ?? identity.label,
            ) ?? []),
          ]
            .filter(Boolean)
            .join(" "),
        cell: (row) => (
          <div className="user-table-name">
            <RouteLink className="catalog-name" href={adminUserHref(row.id)}>
              {row.display_name}
            </RouteLink>
            {row.email ? <small>{row.email}</small> : null}
          </div>
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
        <Button variant="primary" onClick={() => setCreating(true)}>
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
      {creating ? (
        <UserEditor
          authConfig={authConfig}
          onCancel={() => setCreating(false)}
          onSaved={() => {
            setCreating(false);
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
function AdminProjects({ onRefresh }: { onRefresh: () => void }) {
  const resource = useResource<{
      projects: Project[];
      next_cursor?: string | null;
    }>("/api/admin/projects"),
    [selected, setSelected] = useState<Project | null>(null),
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
        <RouteLink
          href={projectCreateHref()}
          className="ui-button ui-button--primary"
        >
          <IconPlus size={16} aria-hidden="true" />
          Create project
        </RouteLink>
      </header>
      <LoadState {...resource} retry={resource.refresh} />
      {error ? (
        <div className="alert" role="alert">
          {error}
        </div>
      ) : null}
      {selected ? (
        <ProjectEditor
          key={`${selected.id}:${selected.revision}`}
          project={selected}
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
                className={`control-classification classification-${draft.tone}${classificationColorVariables(draft.colors) ? " classification-custom-colors" : ""}`}
                style={classificationColorVariables(draft.colors)}
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
