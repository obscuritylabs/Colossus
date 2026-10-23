import { useEffect, useMemo, useRef, useState } from "react";
import { Button, DropdownSelect, TextInput } from "@colossus/ui";
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
import { IconPlus, IconSettings, IconShieldLock } from "@tabler/icons-react";
import { ApiFailure, request } from "./api";
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
import {
  adminUserHref,
  projectHref,
  globalConnectionSettingsSections,
  connectionSettingsLabels,
  type ConnectionSettingsSection,
  type AdminView,
  type AdminUserView,
} from "./routes";
import { AdminUserDetails } from "./AdminUserDetails";
import { UserEditor } from "./AdminUserEditor";
import { useUserInventory } from "./admin-user-data";
import { GlobalConfiguration } from "./GlobalConfiguration";
import { EnvironmentAppearance } from "./Appearance";
import { projectCreateHref } from "./routes";
import {
  RouteLink,
  useNavigation,
  useSettingsNavigationGuard,
} from "./navigation";
import {
  SettingsEditorPage,
  type SettingsEditorNavigation,
} from "./settings-editor";
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
  onProjectChange,
  editorLocation,
  onOpenEditor,
  onCloseEditor,
  onDirtyChange,
}: SettingsEditorNavigation & {
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
  onProjectChange?: ((project: string) => void) | undefined;
  view?: AdminView;
  onView?: (value: AdminView) => void;
}) {
  const tab = view ?? "users";
  const editorNavigation = {
    editorLocation,
    onOpenEditor,
    onCloseEditor,
    onDirtyChange,
  };
  if (tab === "users" && editorLocation)
    return (
      <UserSettingsEditorRoute
        key={`${identity}:${editorLocation.action}:${editorLocation.id ?? ""}`}
        authConfig={authConfig}
        identity={identity}
        {...editorNavigation}
        onSaved={onRefresh}
      />
    );
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
      <header className="sr-only">
        <h1>
          {tab === "appearance"
            ? "Appearance"
            : tab === "settings"
              ? "Authentication and classification"
              : tab === "users"
                ? "Users"
                : tab === "projects"
                  ? "Project hierarchy"
                  : tab === "announcements"
                    ? "Announcements"
                    : `Control Plane ${connectionSettingsLabels[tab as ConnectionSettingsSection] ?? tab}`}
        </h1>
      </header>
      {tab === "users" ? (
        <Users
          identity={identity}
          onCreate={() => onOpenEditor?.({ action: "new" })}
        />
      ) : tab === "projects" ? (
        <AdminProjects key={identity} identity={identity} />
      ) : tab === "announcements" ? (
        <AdminAnnouncements {...editorNavigation} identity={identity} />
      ) : tab === "appearance" ? (
        <>
          <header className="catalog-heading">
            <div>
              <h2>Appearance</h2>
              <p>Publish classification banner colors for this instance.</p>
            </div>
          </header>
          <EnvironmentAppearance onSaved={onSettings} />
        </>
      ) : (globalConnectionSettingsSections as readonly string[]).includes(
          tab,
        ) ? (
        <GlobalConfiguration
          {...editorNavigation}
          section={tab as ConnectionSettingsSection}
          identity={identity}
          projects={projects}
          selectedProject={selectedProject}
          onProjectChange={onProjectChange}
          onSaved={onRefresh}
        />
      ) : (
        <EnvironmentSettings
          key={identity}
          identity={identity}
          onSaved={onSettings}
        />
      )}
    </section>
  );
}
function Users({
  identity,
  onCreate,
}: {
  identity: string;
  onCreate: () => void;
}) {
  const resource = useUserInventory<User, "users">(
    "/api/admin/users",
    "users",
    identity,
  );
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
        <Button variant="primary" onClick={onCreate}>
          <IconPlus size={16} aria-hidden="true" />
          Create user
        </Button>
      </header>
      <LoadState {...resource} retry={resource.refresh} />
      {resource.moreError ? (
        <p className="alert" role="alert">
          {resource.moreError}
        </p>
      ) : null}
      <DataTable
        data={resource.rows}
        columns={columns}
        getRowId={(row) => row.id}
        search={{ columnId: "name", label: "Search loaded users" }}
        label="Control Plane users"
        loading={resource.loading}
        empty="No users found."
      />
      {resource.cursor ? (
        <div className="load-more">
          <p className="field-help">
            Search and sorting apply to loaded users.
          </p>
          <Button
            disabled={resource.moreBusy}
            onClick={() => void resource.loadMore()}
          >
            Load more users
          </Button>
        </div>
      ) : null}
    </>
  );
}
function AdminProjects({ identity }: { identity: string }) {
  const resource = useUserInventory<Project, "projects">(
    "/api/admin/projects",
    "projects",
    identity,
  );
  const projects = resource.rows;
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
      {resource.moreError ? (
        <p className="alert" role="alert">
          {resource.moreError}
        </p>
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
            <RouteLink
              className="ui-button"
              href={projectHref(project.id, "settings")}
            >
              Edit project
            </RouteLink>
          </div>
        ))}
      </div>
      {resource.cursor ? (
        <div className="load-more">
          <p className="field-help">
            Showing loaded projects. Load more to review additional hierarchy
            entries.
          </p>
          <Button
            disabled={resource.moreBusy}
            onClick={() => void resource.loadMore()}
          >
            Load more projects
          </Button>
        </div>
      ) : null}
    </>
  );
}
function EnvironmentSettings({
  onSaved,
  identity,
}: {
  onSaved: (settings: PublicSettings) => void;
  identity: string;
}) {
  const resource = useResource<AdminSettings>(
    "/api/admin/settings",
    0,
    identity,
  );
  const [draft, setDraft] = useState<Classification | null>(null);
  const [busy, setBusy] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [needsReload, setNeedsReload] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const dirty = useRef(false),
    pending = useRef(false),
    alive = useRef(true);
  const reviewedRevision = useRef<number | null>(null);
  const reloadSource = useRef<AdminSettings | null>(null);
  const markDirty = useSettingsNavigationGuard();
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  useEffect(() => {
    markDirty(dirty.current || busy || needsReload);
  }, [busy, needsReload, markDirty]);
  useEffect(() => {
    if (
      resource.data &&
      resource.data.revision >= (reviewedRevision.current ?? 0) &&
      ((!dirty.current && !pending.current) ||
        (reloadSource.current && resource.data !== reloadSource.current))
    ) {
      setDraft(resource.data.classification);
      reviewedRevision.current = resource.data.revision;
      dirty.current = false;
      markDirty(false);
      setNeedsReload(false);
      setRefreshing(false);
      setError("");
      reloadSource.current = null;
    }
  }, [resource.data, markDirty]);
  useEffect(() => {
    if (resource.error) setRefreshing(false);
  }, [resource.error]);
  function changeDraft(value: Classification) {
    setDraft(value);
    dirty.current = true;
    markDirty(true);
  }
  async function save() {
    if (!draft || pending.current || needsReload || refreshing) return;
    if (draft.enabled && !draft.text.trim()) {
      setError("Enter classification text before enabling the banner.");
      return;
    }
    if (
      [...draft.text].length > 160 ||
      /[\u0000-\u001f\u007f-\u009f]/u.test(draft.text)
    ) {
      setError(
        "Use at most 160 classification characters without control characters.",
      );
      return;
    }
    pending.current = true;
    setBusy(true);
    markDirty(true);
    setError("");
    setNotice("");
    try {
      const result = await request<PublicSettings & { revision: number }>(
        "/api/admin/settings",
        { revision: reviewedRevision.current, classification: draft },
        undefined,
        "PATCH",
      );
      if (!alive.current) return;
      dirty.current = false;
      markDirty(false);
      reviewedRevision.current = result.revision;
      setDraft(result.classification);
      onSaved(result);
      resource.refresh();
      setNotice("Classification banner settings saved.");
    } catch (cause) {
      if (!alive.current) return;
      const uncertain = !(cause instanceof ApiFailure) || cause.status >= 500;
      setNeedsReload(
        uncertain || (cause instanceof ApiFailure && cause.status === 409),
      );
      setError(
        uncertain
          ? "The save could not be confirmed. Reload current settings before making another change."
          : cause instanceof ApiFailure && cause.status === 409
            ? "These settings changed. Reload the current version before saving again."
            : errorMessage(cause),
      );
    } finally {
      pending.current = false;
      if (alive.current) {
        setBusy(false);
      }
    }
  }
  function reload() {
    reloadSource.current = resource.data;
    setRefreshing(true);
    resource.refresh();
  }
  return (
    <>
      <header className="catalog-heading">
        <div>
          <h2>Authentication and classification</h2>
          <p>
            Review instance login methods and publish classification markings.
          </p>
        </div>
      </header>
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
            aria-busy={busy || refreshing}
            onSubmit={(event) => {
              event.preventDefault();
              void save();
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
            {needsReload ? (
              <Button disabled={busy || refreshing} onClick={reload}>
                Reload current settings
              </Button>
            ) : null}
            <fieldset
              className="settings-editor-fields"
              disabled={busy || refreshing || needsReload}
            >
              <label className="checkbox-field">
                <input
                  type="checkbox"
                  checked={draft.enabled}
                  onChange={(e) =>
                    changeDraft({ ...draft, enabled: e.target.checked })
                  }
                />
                Display classification banner
              </label>
              <label>
                <span>Classification text</span>
                <TextInput
                  required={draft.enabled}
                  maxLength={320}
                  value={draft.text}
                  onChange={(e) =>
                    changeDraft({ ...draft, text: e.target.value })
                  }
                />
              </label>
              <div className="form-pair">
                <label>
                  <span>Tone</span>
                  <DropdownSelect
                    value={draft.tone}
                    onChange={(e) =>
                      changeDraft({
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
                      changeDraft({
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
            </fieldset>
          </form>
        </section>
      ) : null}
    </>
  );
}

function UserSettingsEditorRoute({
  authConfig,
  identity,
  editorLocation,
  onCloseEditor,
  onSaved,
  onDirtyChange,
}: SettingsEditorNavigation & {
  authConfig: AuthConfig | null;
  identity: string;
  onSaved: () => void;
}) {
  const navigation = useNavigation();
  const resource = useResource<{ user: User }>(
    editorLocation?.action === "edit" && editorLocation.id
      ? `/api/admin/users/${encodeURIComponent(editorLocation.id)}`
      : null,
    0,
    identity,
  );
  const [busy, setBusy] = useState(false);
  const [snapshot, setSnapshot] = useState<{ user: User } | null>(null);
  const [generation, setGeneration] = useState(0);
  const reloadSource = useRef<{ user: User } | null>(null);
  useEffect(() => {
    if (
      resource.data &&
      (!snapshot ||
        (reloadSource.current && resource.data !== reloadSource.current))
    ) {
      setSnapshot(resource.data);
      setGeneration((value) => value + 1);
      reloadSource.current = null;
    }
  }, [resource.data, snapshot]);
  function reload() {
    reloadSource.current = resource.data;
    resource.refresh();
  }
  function close() {
    if (editorLocation?.action === "edit" && editorLocation.id)
      navigation.go(adminUserHref(editorLocation.id));
    else onCloseEditor?.();
  }
  return (
    <section className="control-page">
      <SettingsEditorPage
        title={editorLocation?.action === "new" ? "Create user" : "Edit user"}
        backLabel={editorLocation?.action === "edit" ? "user" : "users"}
        onBack={close}
        busy={busy}
      >
        <LoadState {...resource} retry={resource.refresh} />
        {editorLocation?.action === "new" || snapshot ? (
          <UserEditor
            key={generation}
            user={editorLocation?.action === "new" ? undefined : snapshot?.user}
            authConfig={authConfig}
            showHeading={false}
            onBusyChange={setBusy}
            onDirtyChange={onDirtyChange}
            onReload={reload}
            onCancel={close}
            onSaved={() => {
              onSaved();
              if (snapshot) navigation.go(adminUserHref(snapshot.user.id));
              else onCloseEditor?.();
            }}
          />
        ) : null}
      </SettingsEditorPage>
    </section>
  );
}
