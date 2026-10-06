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
import { RouteLink } from "./navigation";
import { globalHref, projectHref, type ProjectView } from "./routes";
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
    [editing, setEditing] = useState(false),
    [visibility, setVisibility] = useState("active");
  const tab = view ?? localTab;
  const project = me.projects.find((item) => item.id === selected);
  useEffect(() => {
    setEditing(false);
  }, [selected]);
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
            <Button onClick={() => setEditing(true)}>
              <IconPlus size={16} aria-hidden="true" />
              New project
            </Button>
          ) : null}
        </div>
      </header>
      {editing ? (
        <ProjectEditor
          projects={me.projects}
          onSaved={() => {
            setEditing(false);
            onRefresh();
          }}
          onCancel={() => setEditing(false)}
        />
      ) : null}
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
                  <h3>Project details</h3>
                </header>
                {permissions.includes("administer") ? (
                  <ProjectEditor
                    key={`${project.id}:${project.revision}`}
                    project={project}
                    projects={me.projects}
                    onSaved={onRefresh}
                    allowHierarchy={me.user.is_admin}
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
export function ProjectEditor({
  project,
  projects,
  onSaved,
  onCancel,
  allowHierarchy = true,
}: {
  project?: Project;
  projects: Project[];
  onSaved: () => void;
  onCancel?: () => void;
  allowHierarchy?: boolean;
}) {
  const [name, setName] = useState(project?.name ?? ""),
    [description, setDescription] = useState(project?.description ?? ""),
    [parent, setParent] = useState(project?.parent_project_id ?? ""),
    [archived, setArchived] = useState(project?.archived ?? false),
    [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  const creationId = useRef(crypto.randomUUID().replaceAll("-", ""));
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
    setBusy(true);
    setError("");
    try {
      await request(
        project ? projectPath(project.id) : "/api/projects",
        {
          name: name.trim(),
          description: description.trim(),
          parent_project_id: parent || null,
          ...(project
            ? { revision: project.revision, archived }
            : { id: creationId.current }),
        },
        undefined,
        project ? "PATCH" : undefined,
      );
      onSaved();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <form
      className="management-form"
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <h3>{project ? "Edit project" : "Create project"}</h3>
      {error ? (
        <div className="alert" role="alert">
          {error}
        </div>
      ) : null}
      <label>
        <span>Name</span>
        <TextInput
          required
          maxLength={200}
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
      </label>
      <label>
        <span>Description</span>
        <textarea
          className="ui-input"
          rows={3}
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          maxLength={2000}
        />
      </label>
      <label>
        <span>Parent project</span>
        <DropdownSelect
          disabled={!allowHierarchy}
          value={parent}
          onChange={(e) => setParent(e.target.value)}
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
            checked={archived}
            onChange={(e) => setArchived(e.target.checked)}
          />
          Archived
        </label>
      ) : null}
      <div className="form-actions">
        <Button type="submit" variant="primary" disabled={busy || !name.trim()}>
          {busy ? "Saving…" : "Save project"}
        </Button>
        {onCancel ? <Button onClick={onCancel}>Cancel</Button> : null}
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
      resource.refresh();
      onChanged();
      setCandidate("");
      setQuery("");
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="control-card">
      <header>
        <h3>
          <IconUsers size={17} aria-hidden="true" />
          Project access
        </h3>
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
              value={query}
              onChange={(e) => {
                setQuery(e.target.value);
                setCandidate("");
              }}
              placeholder="Type at least two characters"
            />
          </label>
          <label>
            <span>User</span>
            <DropdownSelect
              value={candidate}
              onChange={(e) => setCandidate(e.target.value)}
              disabled={!candidates.data?.users.length}
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
              value={role}
              onChange={(e) => setRole(e.target.value as ProjectRole)}
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
            setBusy(true);
            try {
              const value = await request<{
                members: Member[];
                next_cursor?: string | null;
              }>(
                `${projectPath(project)}/members?after=${encodeURIComponent(cursor)}`,
              );
              setMore((current) => [...current, ...value.members]);
              setCursor(value.next_cursor ?? null);
            } catch (e) {
              setError(errorMessage(e));
            } finally {
              setBusy(false);
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
  useEffect(() => {
    if (resource.data) {
      setProfile(resource.data.required_sandbox_profile ?? "");
      setApprovals(resource.data.allowed_approval_modes.join("\n"));
      setTools(resource.data.allowed_tools?.join("\n") ?? "");
      setToolLimit(resource.data.allowed_tools !== null);
    }
  }, [resource.data]);
  return (
    <section className="control-card">
      <header>
        <h3>
          <IconShield size={17} aria-hidden="true" />
          Policy monitoring baseline
        </h3>
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
            setBusy(true);
            setError("");
            setNotice("");
            try {
              await request(
                `${projectPath(project)}/policy`,
                {
                  revision: resource.data!.revision,
                  required_sandbox_profile: profile.trim() || null,
                  allowed_approval_modes: lines(approvals),
                  allowed_tools: toolLimit ? lines(tools) : null,
                },
                undefined,
                "PATCH",
              );
              resource.refresh();
              setNotice("Monitoring baseline saved.");
            } catch (error) {
              setError(errorMessage(error));
            } finally {
              setBusy(false);
            }
          }}
        >
          {error ? (
            <div className="alert" role="alert">
              {error}
              <Button onClick={resource.refresh}>Reload baseline</Button>
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
              onChange={(e) => setProfile(e.target.value)}
              disabled={!canManage}
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
                  disabled={!canManage}
                  checked={lines(approvals).includes(item.value)}
                  onChange={(event) =>
                    setApprovals(
                      (event.target.checked
                        ? [...lines(approvals), item.value]
                        : lines(approvals).filter(
                            (value) => value !== item.value,
                          )
                      ).join("\n"),
                    )
                  }
                />
                {item.label}
              </label>
            ))}
          </fieldset>
          <label className="checkbox-field">
            <input
              type="checkbox"
              checked={toolLimit}
              onChange={(e) => setToolLimit(e.target.checked)}
              disabled={!canManage}
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
                onChange={(e) => setTools(e.target.value)}
                disabled={!canManage}
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
