import { useEffect, useMemo, useRef, useState } from "react";
import { Button, DropdownSelect, TextInput } from "@colossus/ui";
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
import {
  IconFolder,
  IconPlus,
  IconShield,
  IconUsers,
} from "@tabler/icons-react";
import { projectPath, request, type Permission } from "./api";
import {
  projectRoles,
  projectPermissions,
  type Member,
  type Me,
  type PolicyBaseline,
  type Project,
  type ProjectRole,
} from "./control-api";
import { errorMessage, useResource } from "./resources";
import { AnalyticsPanel, LoadState } from "./Home";
import { RouteLink, useSettingsNavigationGuard } from "./navigation";
import {
  globalHref,
  projectHref,
  projectCreateHref,
  type ProjectView,
} from "./routes";

/** Async completions belong to the mounted resource scope, including A → B → A. */
function useOperationScope(key: string) {
  const scope = useRef({ key });
  if (scope.current.key !== key) scope.current = { key };
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  return () => {
    const operation = scope.current;
    return () => alive.current && scope.current === operation;
  };
}
export function SectionTabs({
  items,
  current,
  onChange,
  hrefFor,
}: {
  items: { id: string; label: string }[];
  current: string;
  onChange: (value: string) => void;
  hrefFor?: ((value: string) => string) | undefined;
}) {
  return (
    <nav className="section-tabs" aria-label="Section views">
      {items.map((item) =>
        hrefFor ? (
          <RouteLink
            key={item.id}
            className="ui-button ui-button--tertiary"
            href={hrefFor(item.id)}
            aria-current={current === item.id ? "page" : undefined}
            onNavigate={() => onChange(item.id)}
          >
            {item.label}
          </RouteLink>
        ) : (
          <Button
            key={item.id}
            variant="tertiary"
            aria-current={current === item.id ? "page" : undefined}
            onClick={() => onChange(item.id)}
          >
            {item.label}
          </Button>
        ),
      )}
    </nav>
  );
}
export function Projects({
  me,
  selected,
  onSelect,
  permissions,
  onRefresh,
  tasks,
  view,
  onView,
}: {
  me: Me;
  selected: string;
  onSelect: (id: string) => void;
  permissions: Permission[];
  onRefresh: () => void;
  tasks: React.ReactNode;
  view?: ProjectView;
  onView?: (value: ProjectView) => void;
}) {
  const [localTab, setTab] = useState("overview"),
    [visibility, setVisibility] = useState("active");
  const tab = view ?? localTab;
  const project = me.projects.find((item) => item.id === selected);
  return (
    <section className="control-page">
      {project ? (
        <RouteLink
          href={globalHref("projects")}
          className="ui-button ui-button--tertiary back"
        >
          All projects
        </RouteLink>
      ) : null}
      <header className="page-heading">
        <div>
          <span className="eyebrow">Projects</span>
          <h1>{project?.name ?? "Projects"}</h1>
          <p>
            {project
              ? project.description ||
                "Review activity, work, and access for this project."
              : "Find your projects and open their work, activity, and access settings."}
          </p>
        </div>
        <div className="project-page-actions">
          {!project ? (
            <label className="project-status-filter">
              <span>Show</span>
              <DropdownSelect
                aria-label="Project status"
                value={visibility}
                onChange={(event) => setVisibility(event.target.value)}
              >
                <option value="active">Active projects</option>
                <option value="archived">Archived projects</option>
                <option value="all">All projects</option>
              </DropdownSelect>
            </label>
          ) : null}
          {me.user.is_admin ? (
            <RouteLink
              href={projectCreateHref()}
              className="ui-button ui-button--primary"
            >
              <IconPlus size={16} aria-hidden="true" />
              New project
            </RouteLink>
          ) : null}
        </div>
      </header>
      {project ? (
        <>
          <div className="project-context">
            <span
              className={`status ${project.archived ? "status-cancelled" : "status-completed"}`}
            >
              {project.archived ? "Archived" : "Active"}
            </span>
            {project.parent_project_id ? (
              <span>
                Within{" "}
                {me.projects.find(
                  (item) => item.id === project.parent_project_id,
                )?.name ?? "parent project"}
              </span>
            ) : (
              <span>Top-level project</span>
            )}
          </div>
          <SectionTabs
            current={tab}
            onChange={(value) =>
              onView ? onView(value as ProjectView) : setTab(value)
            }
            hrefFor={(value) => projectHref(selected, value as ProjectView)}
            items={[
              { id: "overview", label: "Overview" },
              { id: "analytics", label: "Analytics" },
              { id: "tasks", label: "Tasks" },
              { id: "access", label: "Access" },
              { id: "settings", label: "Settings" },
            ]}
          />
          {tab === "overview" || tab === "analytics" ? (
            <>
              <AnalyticsPanel path={`${projectPath(selected)}/analytics`} />
            </>
          ) : tab === "tasks" ? (
            tasks
          ) : tab === "access" ? (
            <ProjectAccess
              key={selected}
              project={selected}
              canManage={permissions.includes("administer")}
              onChanged={onRefresh}
            />
          ) : (
            <>
              <section className="control-card">
                <header>
                  <h2>Project details</h2>
                </header>
                {permissions.includes("administer") ? (
                  <ProjectEditor
                    key={project.id}
                    project={project}
                    projects={me.projects}
                    onSaved={onRefresh}
                    allowHierarchy={me.user.is_admin}
                    showHeading={false}
                  />
                ) : (
                  <>
                    <dl className="stat-list">
                      <div>
                        <dt>Name</dt>
                        <dd>{project.name}</dd>
                      </div>
                      <div>
                        <dt>Parent</dt>
                        <dd>
                          {me.projects.find(
                            (item) => item.id === project.parent_project_id,
                          )?.name ?? "None"}
                        </dd>
                      </div>
                    </dl>
                    <p className="field-help">
                      A Control Plane administrator manages project details.
                    </p>
                  </>
                )}
              </section>
              <PolicyEditor
                key={selected}
                project={selected}
                canManage={permissions.includes("administer")}
              />
            </>
          )}
        </>
      ) : (
        <ProjectDirectory me={me} visibility={visibility} onSelect={onSelect} />
      )}
    </section>
  );
}

const projectRowId = (project: Project) => project.id;
const projectSorting = [{ id: "project", desc: false }];

function ProjectDirectory({
  me,
  visibility,
  onSelect,
}: {
  me: Me;
  visibility: string;
  onSelect: (id: string) => void;
}) {
  const projects = useMemo(
    () =>
      me.projects.filter(
        (project) =>
          visibility === "all" ||
          project.archived === (visibility === "archived"),
      ),
    [me.projects, visibility],
  );
  const columns = useMemo<DataTableColumn<Project>[]>(() => {
    const names = new Map(
        me.projects.map((project) => [project.id, project.name]),
      ),
      memberships = new Map(
        me.memberships.map((member) => [member.project_id, member]),
      );
    const role = (project: Project) => {
      const membership = memberships.get(project.id);
      return membership
        ? (projectRoles.find((entry) => entry.value === membership.role)
            ?.label ?? "Project member")
        : me.user.is_admin
          ? "Administrator visibility"
          : "Project access";
    };
    return [
      {
        id: "project",
        label: "Project",
        value: (project) => `${project.name} ${project.description}`,
        rowHeader: true,
        hideable: false,
        cell: (project) => (
          <div className="catalog-identity">
            <span className="resource-icon">
              <IconFolder size={17} aria-hidden="true" />
            </span>
            <div className="project-table-copy">
              <RouteLink
                className="catalog-name"
                href={projectHref(project.id)}
                onNavigate={() => onSelect(project.id)}
              >
                {project.name}
              </RouteLink>
              {project.description ? (
                <small>{project.description}</small>
              ) : null}
            </div>
          </div>
        ),
      },
      {
        id: "parent",
        label: "Parent",
        value: (project) => names.get(project.parent_project_id ?? "") ?? "",
        cell: (project) =>
          project.parent_project_id ? (
            names.has(project.parent_project_id) ? (
              <RouteLink
                href={projectHref(project.parent_project_id)}
                className="project-parent-link"
                onNavigate={() => onSelect(project.parent_project_id!)}
              >
                {names.get(project.parent_project_id)}
              </RouteLink>
            ) : (
              <span className="muted">Parent not visible</span>
            )
          ) : (
            <span className="muted">Top-level</span>
          ),
      },
      {
        id: "status",
        label: "Status",
        value: (project) => (project.archived ? "Archived" : "Active"),
        cell: (project) => (
          <span
            className={`status ${project.archived ? "status-cancelled" : "status-completed"}`}
          >
            {project.archived ? "Archived" : "Active"}
          </span>
        ),
      },
      {
        id: "access",
        label: "Your access",
        value: role,
        cell: (project) => (
          <div className="project-table-copy">
            <strong>{role(project)}</strong>
            <small>
              {projectPermissions(me, project.id)
                .map((permission) =>
                  permission === "administer"
                    ? "Manage"
                    : permission[0]!.toUpperCase() + permission.slice(1),
                )
                .join(" · ") || "No project permissions"}
            </small>
          </div>
        ),
      },
    ];
  }, [me, onSelect]);
  return (
    <section className="project-table-directory" aria-label="Project directory">
      <DataTable
        data={projects}
        columns={columns}
        getRowId={projectRowId}
        label="Projects"
        itemLabel="projects"
        search={{ columnId: "project", label: "Search projects" }}
        initialSorting={projectSorting}
        empty={
          <div className="project-table-empty">
            <IconFolder size={24} aria-hidden="true" />
            <strong>
              {me.projects.length
                ? "No projects match this view"
                : "No visible projects"}
            </strong>
            <span>
              {me.projects.length
                ? "Change the status filter or search to find another project."
                : me.user.is_admin
                  ? "Create a project to organize agent work."
                  : "Ask an administrator for project access."}
            </span>
          </div>
        }
      />
    </section>
  );
}
export function CreateProject({
  me,
  onSaved,
  onCancel,
}: {
  me: Me;
  onSaved: () => void;
  onCancel: () => void;
}) {
  return (
    <section className="control-page">
      <RouteLink
        href={globalHref("projects")}
        className="ui-button ui-button--tertiary back"
      >
        All projects
      </RouteLink>
      <header className="page-heading">
        <div>
          <span className="eyebrow">Projects</span>
          <h1>Create project</h1>
          <p>Name the project and choose where it belongs.</p>
        </div>
      </header>
      {me.user.is_admin ? (
        <ProjectEditor
          projects={me.projects}
          onSaved={onSaved}
          onCancel={onCancel}
          showHeading={false}
        />
      ) : (
        <p role="alert">
          Administrator access is required to create a project.
        </p>
      )}
    </section>
  );
}
export function ProjectEditor({
  project,
  projects,
  onSaved,
  onCancel,
  allowHierarchy = true,
  showHeading = true,
}: {
  project?: Project;
  projects: Project[];
  onSaved: () => void;
  onCancel?: () => void;
  allowHierarchy?: boolean;
  showHeading?: boolean;
}) {
  const [name, setName] = useState(project?.name ?? ""),
    [description, setDescription] = useState(project?.description ?? ""),
    [parent, setParent] = useState(project?.parent_project_id ?? ""),
    [archived, setArchived] = useState(project?.archived ?? false),
    [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const creationId = useRef(crypto.randomUUID().replaceAll("-", ""));
  const pending = useRef(false);
  const dirty = useRef(false);
  const revision = useRef(project?.revision);
  const draftScope = useRef(project?.id);
  const markDirty = useSettingsNavigationGuard();
  const captureScope = useOperationScope(project?.id ?? "new-project");
  const changed = () => {
    dirty.current = true;
    markDirty(true);
  };
  useEffect(() => {
    const scopeChanged = draftScope.current !== project?.id;
    if (scopeChanged || (!dirty.current && !pending.current)) {
      setName(project?.name ?? "");
      setDescription(project?.description ?? "");
      setParent(project?.parent_project_id ?? "");
      setArchived(project?.archived ?? false);
      revision.current = project?.revision;
      if (scopeChanged) {
        pending.current = false;
        dirty.current = false;
        markDirty(false);
        setBusy(false);
        setError("");
      }
    }
    draftScope.current = project?.id;
  }, [project, markDirty]);
  const candidates = projects.filter((item) => {
    let cursor: Project | undefined = item;
    const seen = new Set<string>();
    while (cursor) {
      if (cursor.id === project?.id || seen.has(cursor.id)) return false;
      seen.add(cursor.id);
      cursor = projects.find((p) => p.id === cursor?.parent_project_id);
    }
    return true;
  });
  async function save() {
    if (pending.current || !name.trim()) return;
    const isCurrent = captureScope();
    pending.current = true;
    setBusy(true);
    setError("");
    try {
      const saved = await request<{ project: Project }>(
        project ? projectPath(project.id) : "/api/projects",
        {
          name: name.trim(),
          description: description.trim(),
          parent_project_id: parent || null,
          ...(project
            ? { revision: revision.current, archived }
            : { id: creationId.current }),
        },
        undefined,
        project ? "PATCH" : undefined,
      );
      if (isCurrent()) {
        dirty.current = false;
        markDirty(false);
        revision.current = saved.project.revision;
        onSaved();
      }
    } catch (e) {
      if (isCurrent()) setError(errorMessage(e));
    } finally {
      if (isCurrent()) {
        pending.current = false;
        setBusy(false);
      }
    }
  }
  return (
    <form
      className="management-form"
      aria-label={project ? "Edit project" : "Create project"}
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      {showHeading ? (
        <h3>{project ? "Edit project" : "Create project"}</h3>
      ) : null}
      {error ? (
        <div className="alert" role="alert">
          {error}
        </div>
      ) : null}
      <label>
        <span>Name</span>
        <TextInput
          required
          disabled={busy}
          maxLength={200}
          value={name}
          onChange={(e) => {
            setName(e.target.value);
            changed();
          }}
        />
      </label>
      <label>
        <span>Description</span>
        <textarea
          className="ui-input"
          rows={3}
          disabled={busy}
          value={description}
          onChange={(e) => {
            setDescription(e.target.value);
            changed();
          }}
          maxLength={2000}
        />
      </label>
      <label>
        <span>Parent project</span>
        <DropdownSelect
          disabled={busy || !allowHierarchy}
          value={parent}
          onChange={(e) => {
            setParent(e.target.value);
            changed();
          }}
        >
          <option value="">None · top-level project</option>
          {candidates.map((item) => (
            <option value={item.id} key={item.id}>
              {item.name}
            </option>
          ))}
        </DropdownSelect>
      </label>
      <p className="field-help">
        Nesting organizes projects. Members of a parent receive no automatic
        access to its children.
      </p>
      {project ? (
        <label className="checkbox-field">
          <input
            type="checkbox"
            disabled={busy}
            checked={archived}
            onChange={(e) => {
              setArchived(e.target.checked);
              changed();
            }}
          />
          Archived
        </label>
      ) : null}
      <div className="form-actions">
        <Button type="submit" variant="primary" disabled={busy || !name.trim()}>
          {busy ? "Saving…" : "Save project"}
        </Button>
        {onCancel ? (
          <Button
            type="button"
            disabled={busy}
            onClick={() => {
              if (
                dirty.current &&
                !window.confirm("Discard unsaved settings changes?")
              )
                return;
              dirty.current = false;
              markDirty(false);
              onCancel();
            }}
          >
            Cancel
          </Button>
        ) : null}
      </div>
    </form>
  );
}
export function ProjectAccess({
  project,
  canManage,
  onChanged,
}: {
  project: string;
  canManage: boolean;
  onChanged: () => void;
}) {
  const resource = useResource<{
      members: Member[];
      next_cursor?: string | null;
    }>(`${projectPath(project)}/members`),
    [more, setMore] = useState<Member[]>([]),
    [cursor, setCursor] = useState<string | null>(null),
    [query, setQuery] = useState(""),
    [search, setSearch] = useState(""),
    [candidate, setCandidate] = useState(""),
    [role, setRole] = useState<ProjectRole>("viewer"),
    [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const pending = useRef(false);
  const captureScope = useOperationScope(project);
  const markDirty = useSettingsNavigationGuard();
  useEffect(() => {
    pending.current = false;
    setBusy(false);
    setError("");
    setCandidate("");
    setQuery("");
    setSearch("");
    setRole("viewer");
    setMore([]);
    setCursor(null);
    markDirty(false);
  }, [project, markDirty]);
  useEffect(() => {
    const timer = setTimeout(() => setSearch(query.trim()), 250);
    return () => clearTimeout(timer);
  }, [query]);
  const candidates = useResource<{
    users: { id: string; display_name: string }[];
  }>(
    canManage && search.length >= 2
      ? `${projectPath(project)}/member-candidates?query=${encodeURIComponent(search)}`
      : null,
  );
  useEffect(() => {
    setMore([]);
    setCursor(resource.data?.next_cursor ?? null);
  }, [resource.data]);
  async function mutate(body: unknown, userId?: string, remove = false) {
    if (!canManage || pending.current || (!userId && !candidate)) return;
    const isCurrent = captureScope();
    pending.current = true;
    setBusy(true);
    setError("");
    try {
      await request(
        `${projectPath(project)}/members${userId ? `/${encodeURIComponent(userId)}${remove ? `?revision=${(body as { revision: number }).revision}` : ""}` : ""}`,
        remove
          ? undefined
          : userId
            ? { ...(body as object), user_id: userId }
            : body,
        undefined,
        remove ? "DELETE" : userId ? "PATCH" : undefined,
      );
      if (isCurrent()) {
        markDirty(false);
        resource.refresh();
        onChanged();
        setCandidate("");
        setQuery("");
      }
    } catch (e) {
      if (isCurrent()) setError(errorMessage(e));
    } finally {
      if (isCurrent()) {
        pending.current = false;
        setBusy(false);
      }
    }
  }
  return (
    <section className="control-card">
      <header>
        <h2>
          <IconUsers size={17} aria-hidden="true" />
          Project access
        </h2>
      </header>
      <p className="field-help">
        Roles grant explicit project permissions. Agent tools and approvals
        remain constrained by the host’s native grant.
      </p>
      <LoadState {...resource} retry={resource.refresh} />
      {error ? (
        <div className="alert" role="alert">
          {error}
        </div>
      ) : null}
      {canManage ? (
        <form
          className="member-invite"
          onSubmit={(e) => {
            e.preventDefault();
            void mutate({ user_id: candidate, role });
          }}
        >
          <label>
            <span>Find a user</span>
            <TextInput
              disabled={busy}
              value={query}
              onChange={(e) => {
                setQuery(e.target.value);
                setCandidate("");
                markDirty(false);
              }}
              placeholder="Type at least two characters"
            />
          </label>
          <label>
            <span>User</span>
            <DropdownSelect
              value={candidate}
              onChange={(e) => {
                setCandidate(e.target.value);
                markDirty(Boolean(e.target.value));
              }}
              disabled={busy || !candidates.data?.users.length}
            >
              <option value="">Select a user</option>
              {candidates.data?.users.map((user) => (
                <option key={user.id} value={user.id}>
                  {user.display_name}
                </option>
              ))}
            </DropdownSelect>
          </label>
          <label>
            <span>Role</span>
            <DropdownSelect
              disabled={busy}
              value={role}
              onChange={(e) => {
                setRole(e.target.value as ProjectRole);
                if (candidate) markDirty(true);
              }}
            >
              {projectRoles.map((item) => (
                <option key={item.value} value={item.value}>
                  {item.label}
                </option>
              ))}
            </DropdownSelect>
          </label>
          <Button type="submit" disabled={busy || !candidate} variant="primary">
            Add member
          </Button>
          {candidates.error ? (
            <p className="alert" role="alert">
              {candidates.error}
            </p>
          ) : null}
        </form>
      ) : null}
      <div className="access-list">
        {[...(resource.data?.members ?? []), ...more].map((member) => (
          <div key={member.user_id}>
            <span>
              <strong>{member.display_name || member.user_id}</strong>
              <small>
                {
                  projectRoles.find((item) => item.value === member.role)
                    ?.description
                }
              </small>
            </span>
            {canManage ? (
              <>
                <DropdownSelect
                  aria-label={`Role for ${member.display_name || member.user_id}`}
                  value={member.role}
                  disabled={busy}
                  onChange={(e) =>
                    void mutate(
                      { role: e.target.value, revision: member.revision },
                      member.user_id,
                    )
                  }
                >
                  {projectRoles.map((item) => (
                    <option key={item.value} value={item.value}>
                      {item.label}
                    </option>
                  ))}
                </DropdownSelect>
                <Button
                  disabled={busy}
                  onClick={() => {
                    if (
                      window.confirm(
                        `Remove ${member.display_name || member.user_id} from this project?`,
                      )
                    )
                      void mutate(
                        { revision: member.revision },
                        member.user_id,
                        true,
                      );
                  }}
                >
                  Remove
                </Button>
              </>
            ) : (
              <span>
                {projectRoles.find((item) => item.value === member.role)
                  ?.label ?? member.role}
              </span>
            )}
          </div>
        ))}
      </div>
      {!resource.loading && !resource.data?.members.length ? (
        <p className="empty-copy">No explicit project memberships.</p>
      ) : null}
      {cursor ? (
        <Button
          disabled={busy}
          onClick={async () => {
            if (pending.current) return;
            const isCurrent = captureScope();
            pending.current = true;
            setBusy(true);
            setError("");
            try {
              const value = await request<{
                members: Member[];
                next_cursor?: string | null;
              }>(
                `${projectPath(project)}/members?after=${encodeURIComponent(cursor)}`,
              );
              if (isCurrent()) {
                setMore((current) => [...current, ...value.members]);
                setCursor(value.next_cursor ?? null);
              }
            } catch (e) {
              if (isCurrent()) setError(errorMessage(e));
            } finally {
              if (isCurrent()) {
                pending.current = false;
                setBusy(false);
              }
            }
          }}
        >
          Load more members
        </Button>
      ) : null}
    </section>
  );
}
function lines(value: string) {
  return value
    .split(/[,\n]/u)
    .map((item) => item.trim())
    .filter(Boolean);
}
export function PolicyEditor({
  project,
  canManage,
}: {
  project: string;
  canManage: boolean;
}) {
  const resource = useResource<PolicyBaseline>(
      `${projectPath(project)}/policy`,
    ),
    [profile, setProfile] = useState(""),
    [approvals, setApprovals] = useState(""),
    [tools, setTools] = useState(""),
    [toolLimit, setToolLimit] = useState(false),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [notice, setNotice] = useState("");
  const pending = useRef(false);
  const dirty = useRef(false);
  const revision = useRef<number | null>(null);
  const markDirty = useSettingsNavigationGuard();
  const captureScope = useOperationScope(project);
  const changed = () => {
    dirty.current = true;
    markDirty(true);
    setNotice("");
  };
  const applyBaseline = (baseline: PolicyBaseline) => {
    revision.current = baseline.revision;
    setProfile(baseline.required_sandbox_profile ?? "");
    setApprovals(baseline.allowed_approval_modes.join("\n"));
    setTools(baseline.allowed_tools?.join("\n") ?? "");
    setToolLimit(baseline.allowed_tools !== null);
  };
  useEffect(() => {
    pending.current = false;
    dirty.current = false;
    revision.current = null;
    markDirty(false);
    setBusy(false);
    setError("");
    setNotice("");
  }, [project, markDirty]);
  useEffect(() => {
    if (resource.data && !dirty.current && !pending.current)
      applyBaseline(resource.data);
  }, [resource.data]);
  return (
    <section className="control-card">
      <header>
        <h2>
          <IconShield size={17} aria-hidden="true" />
          Policy monitoring baseline
        </h2>
      </header>
      <p className="field-help">
        Compare runtime-reported posture with this project’s expectations.
        Saving a baseline does not change agent policy or establish independent
        attestation.
      </p>
      <LoadState {...resource} retry={resource.refresh} />
      {resource.data ? (
        <form
          className="management-form"
          onSubmit={async (e) => {
            e.preventDefault();
            if (!canManage || pending.current || revision.current === null)
              return;
            const isCurrent = captureScope();
            pending.current = true;
            setBusy(true);
            setError("");
            setNotice("");
            try {
              const saved = await request<PolicyBaseline>(
                `${projectPath(project)}/policy`,
                {
                  revision: revision.current,
                  required_sandbox_profile: profile.trim() || null,
                  allowed_approval_modes: lines(approvals),
                  allowed_tools: toolLimit ? lines(tools) : null,
                },
                undefined,
                "PATCH",
              );
              if (isCurrent()) {
                dirty.current = false;
                markDirty(false);
                applyBaseline(saved);
                resource.refresh();
                setNotice("Monitoring baseline saved.");
              }
            } catch (error) {
              if (isCurrent()) setError(errorMessage(error));
            } finally {
              if (isCurrent()) {
                pending.current = false;
                setBusy(false);
              }
            }
          }}
        >
          {error ? (
            <div className="alert" role="alert">
              {error}
              <Button
                type="button"
                disabled={busy}
                onClick={() => {
                  if (
                    dirty.current &&
                    !window.confirm("Discard unsaved settings changes?")
                  )
                    return;
                  dirty.current = false;
                  markDirty(false);
                  resource.refresh();
                }}
              >
                Reload baseline
              </Button>
            </div>
          ) : null}
          {notice ? (
            <p role="status" className="notice">
              {notice}
            </p>
          ) : null}
          <label>
            <span>Required sandbox profile</span>
            <TextInput
              value={profile}
              onChange={(e) => {
                setProfile(e.target.value);
                changed();
              }}
              disabled={busy || !canManage}
              placeholder="Any profile"
              maxLength={128}
            />
          </label>
          <fieldset className="approval-options">
            <legend>Allowed approval modes</legend>
            <p className="field-help">
              Leave all unchecked to accept any reported mode.
            </p>
            {[
              { value: "ask", label: "Ask for approval" },
              { value: "deny", label: "Deny approval obligations" },
              { value: "risk_auto", label: "Risk-based automatic approval" },
              { value: "danger_auto", label: "Automatic approval" },
              { value: "unknown", label: "Unknown / unreported" },
            ].map((item) => (
              <label className="checkbox-field" key={item.value}>
                <input
                  type="checkbox"
                  disabled={busy || !canManage}
                  checked={lines(approvals).includes(item.value)}
                  onChange={(event) => {
                    setApprovals(
                      (event.target.checked
                        ? [...lines(approvals), item.value]
                        : lines(approvals).filter(
                            (value) => value !== item.value,
                          )
                      ).join("\n"),
                    );
                    changed();
                  }}
                />
                {item.label}
              </label>
            ))}
          </fieldset>
          <label className="checkbox-field">
            <input
              type="checkbox"
              checked={toolLimit}
              onChange={(e) => {
                setToolLimit(e.target.checked);
                changed();
              }}
              disabled={busy || !canManage}
            />
            Monitor a tool ceiling
          </label>
          {toolLimit ? (
            <label>
              <span>Allowed tools · one per line</span>
              <textarea
                className="ui-input"
                rows={4}
                value={tools}
                onChange={(e) => {
                  setTools(e.target.value);
                  changed();
                }}
                disabled={busy || !canManage}
              />
            </label>
          ) : null}
          {canManage ? (
            <Button type="submit" variant="primary" disabled={busy}>
              {busy ? "Saving…" : "Save baseline"}
            </Button>
          ) : (
            <p className="field-help">
              A project administrator manages this baseline.
            </p>
          )}
        </form>
      ) : null}
    </section>
  );
}
