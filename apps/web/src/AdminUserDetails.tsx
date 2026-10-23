import { useState } from "react";
import { Button, DropdownSelect } from "@colossus/ui";
import {
  IconArrowLeft,
  IconArrowRight,
  IconFolder,
  IconLayoutDashboard,
  IconMessageCircle,
  IconRefresh,
  IconUser,
} from "@tabler/icons-react";
import type { AuthConfig, User } from "./control-api";
import { LoadState } from "./Home";
import { useNavigation, RouteLink } from "./navigation";
import { SectionTabs } from "./Projects";
import { useResource } from "./resources";
import {
  adminUserHref,
  globalHref,
  settingsEditorHref,
  type AdminUserView,
} from "./routes";
import {
  InventoryContinuation,
  UserProjectsTable,
  UserResourceTable,
} from "./AdminUserInventories";
import {
  useUserInventory,
  userApiPath,
  userDate,
  type UserProject,
} from "./admin-user-data";
import "./admin-user.css";

export function AdminUserDetails({
  userId,
  currentUserId,
  view,
  project,
}: {
  userId: string;
  currentUserId: string;
  view: AdminUserView;
  project?: string | undefined;
  authConfig: AuthConfig | null;
  onChanged: () => void;
}) {
  const navigation = useNavigation();
  const profile = useResource<{ user: User }>(
    userApiPath(userId),
    0,
    currentUserId,
  );
  const projects = useUserInventory<UserProject, "projects">(
    `${userApiPath(userId)}/projects?limit=50`,
    "projects",
    currentUserId,
  );
  const [resourceGeneration, setResourceGeneration] = useState(0);
  const user = profile.error ? undefined : profile.data?.user;
  const selectedProject = project ?? projects.rows[0]?.project.id ?? "";
  function refresh() {
    setResourceGeneration((value) => value + 1);
    profile.refresh();
    projects.refresh();
  }
  return (
    <section className="control-page admin-user-page">
      <RouteLink
        className="ui-button ui-button--tertiary back"
        href={globalHref("admin", undefined, "users")}
      >
        <IconArrowLeft size={16} aria-hidden="true" />
        Back to users
      </RouteLink>
      <LoadState {...profile} retry={profile.refresh} />
      {user ? (
        <>
          <header className="page-heading user-profile-heading">
            <div className="user-profile-identity">
              <div className="user-profile-avatar" aria-hidden="true">
                <IconUser size={26} />
              </div>
              <div>
                <span className="eyebrow">
                  User details
                  {user.id === currentUserId ? " · Your account" : ""}
                </span>
                <h1>{user.display_name}</h1>
                <p>{user.email || "No email address provided"}</p>
                <div className="user-profile-badges">
                  <span
                    className={`status ${user.active ? "status-completed" : "status-cancelled"}`}
                  >
                    {user.active ? "Active" : "Disabled"}
                  </span>
                  <span className="status">
                    {user.is_admin ? "Administrator" : "Member"}
                  </span>
                </div>
              </div>
            </div>
            <div className="user-profile-actions">
              <Button
                onClick={refresh}
                disabled={profile.loading}
                aria-label="Refresh user details"
              >
                <IconRefresh size={16} aria-hidden="true" />
                Refresh
              </Button>
              <Button
                variant="primary"
                onClick={() =>
                  navigation.go(
                    settingsEditorHref(
                      globalHref("admin", undefined, "users"),
                      { action: "edit", id: user.id },
                    ),
                  )
                }
              >
                Edit user
              </Button>
            </div>
          </header>
          <SectionTabs
            current={view}
            onChange={(value) =>
              navigation.go(
                adminUserHref(user.id, value as AdminUserView, project),
              )
            }
            hrefFor={(value) =>
              adminUserHref(user.id, value as AdminUserView, project)
            }
            items={[
              { id: "overview", label: "Overview" },
              { id: "projects", label: "Projects" },
              { id: "workspaces", label: "Workspaces" },
              { id: "chats", label: "Chats" },
            ]}
          />
          {view === "overview" ? (
            <>
              <div className="user-overview-grid">
                <section className="control-card">
                  <header>
                    <h2>Account details</h2>
                  </header>
                  <dl className="stat-list">
                    <div>
                      <dt>Display name</dt>
                      <dd>{user.display_name}</dd>
                    </div>
                    <div>
                      <dt>Email</dt>
                      <dd>{user.email || "Not provided"}</dd>
                    </div>
                    <div>
                      <dt>User ID</dt>
                      <dd className="user-resource-id">{user.id}</dd>
                    </div>
                    <div>
                      <dt>Created</dt>
                      <dd>
                        <time dateTime={user.created_at}>
                          {userDate(user.created_at)}
                        </time>
                      </dd>
                    </div>
                    <div>
                      <dt>Last updated</dt>
                      <dd>
                        <time dateTime={user.updated_at}>
                          {userDate(user.updated_at)}
                        </time>
                      </dd>
                    </div>
                  </dl>
                </section>
                <section className="control-card">
                  <header>
                    <h2>Login identities</h2>
                  </header>
                  {user.identities?.length ? (
                    <ul className="user-identity-list">
                      {user.identities.map((identity, index) => (
                        <li key={`${identity.kind}:${index}`}>
                          <span className="status">
                            {identity.kind === "local" ? "Local" : "OIDC"}
                          </span>
                          <div>
                            <strong>{identity.label}</strong>
                            {identity.username ? (
                              <span>{identity.username}</span>
                            ) : null}
                            {identity.issuer ? (
                              <span>{identity.issuer}</span>
                            ) : null}
                            {identity.subject ? (
                              <span className="user-resource-id">
                                {identity.subject}
                              </span>
                            ) : null}
                          </div>
                        </li>
                      ))}
                    </ul>
                  ) : (
                    <p className="empty-copy">No login identities reported.</p>
                  )}
                  <p className="field-help">
                    {user.is_admin
                      ? "This user can administer the Control Plane and manage projects. Execution and approval permissions require an explicit project role."
                      : "This user’s access is assigned through explicit project memberships."}
                  </p>
                </section>
              </div>
              <div className="user-inventory-links" aria-label="User resources">
                <RouteLink href={adminUserHref(user.id, "projects", project)}>
                  <IconFolder size={22} aria-hidden="true" />
                  <div>
                    <strong>Projects</strong>
                    <span>
                      {projects.loading
                        ? "Loading memberships…"
                        : projects.error
                          ? "Review project access"
                          : `${projects.rows.length}${projects.cursor ? "+" : ""} explicit membership${projects.rows.length === 1 ? "" : "s"}`}
                    </span>
                  </div>
                  <IconArrowRight size={18} aria-hidden="true" />
                </RouteLink>
                <RouteLink href={adminUserHref(user.id, "workspaces", project)}>
                  <IconLayoutDashboard size={22} aria-hidden="true" />
                  <div>
                    <strong>Workspaces</strong>
                    <span>Browse personal workspaces by project</span>
                  </div>
                  <IconArrowRight size={18} aria-hidden="true" />
                </RouteLink>
                <RouteLink href={adminUserHref(user.id, "chats", project)}>
                  <IconMessageCircle size={22} aria-hidden="true" />
                  <div>
                    <strong>Chats</strong>
                    <span>Review authorized personal chat history</span>
                  </div>
                  <IconArrowRight size={18} aria-hidden="true" />
                </RouteLink>
              </div>
            </>
          ) : view === "projects" ? (
            <UserProjectsTable inventory={projects} />
          ) : (
            <>
              <header className="catalog-heading user-resource-heading">
                <div>
                  <h2>{view === "chats" ? "Chats" : "Workspaces"}</h2>
                  <p>
                    {view === "chats"
                      ? "Personal conversations owned by this user in the selected project."
                      : "Personal workspaces attached to this user’s agents in the selected project."}
                  </p>
                </div>
                {projects.rows.length ? (
                  <label className="user-project-select">
                    <span>Project</span>
                    <DropdownSelect
                      aria-label="Project for user resources"
                      value={selectedProject}
                      onChange={(event) =>
                        navigation.go(
                          adminUserHref(user.id, view, event.target.value),
                        )
                      }
                    >
                      {project &&
                      !projects.rows.some(
                        (row) => row.project.id === project,
                      ) ? (
                        <option value={project}>Selected project</option>
                      ) : null}
                      {projects.rows.map((row) => (
                        <option key={row.project.id} value={row.project.id}>
                          {row.project.name}
                          {row.project.archived ? " (Archived)" : ""}
                        </option>
                      ))}
                    </DropdownSelect>
                  </label>
                ) : null}
              </header>
              <LoadState {...projects} retry={projects.refresh} />
              {!projects.loading && !projects.error && !projects.rows.length ? (
                <div className="user-privacy-note">
                  <IconFolder size={22} aria-hidden="true" />
                  <div>
                    <strong>No project memberships</strong>
                    <p>
                      This user has no explicit project memberships to browse.
                    </p>
                  </div>
                </div>
              ) : null}
              {!projects.error && selectedProject ? (
                <UserResourceTable
                  key={resourceGeneration}
                  userId={userId}
                  currentUserId={currentUserId}
                  project={selectedProject}
                  kind={view}
                />
              ) : null}
              <InventoryContinuation inventory={projects} label="projects" />
            </>
          )}
        </>
      ) : null}
    </section>
  );
}
