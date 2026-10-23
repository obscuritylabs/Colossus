import { useMemo } from "react";
import { Button } from "@colossus/ui";
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
import { IconLock } from "@tabler/icons-react";
import type { Thread } from "./api";
import { projectRoles } from "./control-api";
import { conversationHref } from "./conversation-route";
import { LoadState } from "./Home";
import { RouteLink } from "./navigation";
import { agentHref, projectHref } from "./routes";
import {
  useUserInventory,
  userApiPath,
  userDate,
  type UserProject,
  type UserWorkspace,
} from "./admin-user-data";

const roleLabel = (role: string) =>
  projectRoles.find((item) => item.value === role)?.label ?? role;
const projectColumns: DataTableColumn<UserProject>[] = [
  {
    id: "project",
    label: "Project",
    hideable: false,
    value: (row) => `${row.project.name} ${row.project.description}`,
    cell: ({ project }) => (
      <div className="user-table-name">
        <RouteLink className="catalog-name" href={projectHref(project.id)}>
          {project.name}
        </RouteLink>
        {project.description ? <small>{project.description}</small> : null}
      </div>
    ),
  },
  { id: "role", label: "Role", value: (row) => roleLabel(row.membership.role) },
  {
    id: "status",
    label: "Status",
    value: (row) => (row.project.archived ? "Archived" : "Active"),
    cell: (row) => (
      <span
        className={`status ${row.project.archived ? "status-cancelled" : "status-completed"}`}
      >
        {row.project.archived ? "Archived" : "Active"}
      </span>
    ),
    filter: {
      label: "Project status",
      options: [
        { value: "", label: "All statuses" },
        { value: "active", label: "Active" },
        { value: "archived", label: "Archived" },
      ],
      matches: (row, value) =>
        !value || row.project.archived === (value === "archived"),
    },
  },
  {
    id: "permissions",
    label: "Permissions",
    value: (row) => row.membership.permissions.join(", "),
    cell: (row) => (
      <span className="user-permissions">
        {row.membership.permissions
          .map(
            (permission) =>
              permission.charAt(0).toUpperCase() + permission.slice(1),
          )
          .join(" · ") || "None"}
      </span>
    ),
  },
  {
    id: "access",
    label: "Access",
    sortable: false,
    className: "catalog-row-actions",
    value: () => "",
    cell: (row) => (
      <RouteLink
        className="ui-button ui-button--tertiary"
        href={projectHref(row.project.id, "access")}
      >
        Manage access
      </RouteLink>
    ),
  },
];
export function UserProjectsTable({
  inventory,
}: {
  inventory: ReturnType<typeof useUserInventory<UserProject, "projects">>;
}) {
  return (
    <>
      <header className="catalog-heading">
        <div>
          <h2>Projects</h2>
          <p>
            Explicit project memberships and the permissions assigned to this
            user.
          </p>
        </div>
        <Button onClick={inventory.refresh} disabled={inventory.loading}>
          Refresh projects
        </Button>
      </header>
      <LoadState {...inventory} retry={inventory.refresh} />
      {!inventory.error ? (
        <DataTable
          data={inventory.rows}
          columns={projectColumns}
          getRowId={(row) => row.project.id}
          label="User projects"
          itemLabel="projects"
          search={{ columnId: "project", label: "Search loaded projects" }}
          loading={inventory.loading}
          empty="This user has no explicit project memberships."
        />
      ) : null}
      <InventoryContinuation inventory={inventory} label="projects" />
    </>
  );
}
export function UserResourceTable({
  userId,
  currentUserId,
  project,
  kind,
}: {
  userId: string;
  currentUserId: string;
  project: string;
  kind: "workspaces" | "chats";
}) {
  return kind === "chats" ? (
    <UserChats
      key={`${userId}:${project}`}
      userId={userId}
      currentUserId={currentUserId}
      project={project}
    />
  ) : (
    <UserWorkspaces
      key={`${userId}:${project}`}
      userId={userId}
      currentUserId={currentUserId}
      project={project}
    />
  );
}
function ResourcePrivacy({ kind }: { kind: "chats" | "workspaces" }) {
  return (
    <div className="user-privacy-note" role="note">
      <IconLock size={20} aria-hidden="true" />
      <div>
        <strong>Personal {kind} are private</strong>
        <p>
          This user’s personal {kind} are visible only to their owner. Control
          Plane administration does not grant access to another user’s personal
          resources.
        </p>
      </div>
    </div>
  );
}
function UserChats({
  userId,
  currentUserId,
  project,
}: {
  userId: string;
  currentUserId: string;
  project: string;
}) {
  const inventory = useUserInventory<Thread, "chats">(
    `${userApiPath(userId)}/projects/${encodeURIComponent(project)}/chats?limit=50`,
    "chats",
    currentUserId,
  );
  const columns = useMemo<DataTableColumn<Thread>[]>(
    () => [
      {
        id: "chat",
        label: "Chat",
        hideable: false,
        value: (row) => row.title || "Untitled chat",
        cell: (row) => (
          <RouteLink className="catalog-name" href={conversationHref(row)}>
            {row.title || "Untitled chat"}
          </RouteLink>
        ),
      },
      {
        id: "status",
        label: "Status",
        value: (row) => (row.archived ? "Archived" : "Active"),
        cell: (row) => (
          <span
            className={`status ${row.archived ? "status-cancelled" : "status-completed"}`}
          >
            {row.archived ? "Archived" : "Active"}
          </span>
        ),
        filter: {
          label: "Chat status",
          options: [
            { value: "", label: "All statuses" },
            { value: "active", label: "Active" },
            { value: "archived", label: "Archived" },
          ],
          matches: (row, value) =>
            !value || row.archived === (value === "archived"),
        },
      },
      {
        id: "workspace",
        label: "Workspace",
        value: (row) => row.workspace_id ?? "Hosted",
        cell: (row) => (
          <span className="user-resource-id">
            {row.workspace_id ?? "Hosted"}
          </span>
        ),
      },
      {
        id: "created",
        label: "Created",
        value: (row) => row.created_at,
        cell: (row) => (
          <time dateTime={row.created_at}>{userDate(row.created_at)}</time>
        ),
      },
      {
        id: "updated",
        label: "Updated",
        value: (row) => row.updated_at,
        cell: (row) => (
          <time dateTime={row.updated_at}>{userDate(row.updated_at)}</time>
        ),
      },
    ],
    [],
  );
  return (
    <>
      <LoadState {...inventory} retry={inventory.refresh} />
      {inventory.visibility === "personal_resources_private" ? (
        <ResourcePrivacy kind="chats" />
      ) : !inventory.error ? (
        <DataTable
          data={inventory.rows}
          columns={columns}
          getRowId={(row) => row.thread_id}
          label="User chats"
          itemLabel="chats"
          search={{ columnId: "chat", label: "Search loaded chats" }}
          loading={inventory.loading}
          initialSorting={[{ id: "updated", desc: true }]}
          empty="No personal chats found in this project."
        />
      ) : null}
      <InventoryContinuation inventory={inventory} label="chats" />
    </>
  );
}
const workspaceColumns: DataTableColumn<UserWorkspace>[] = [
  {
    id: "workspace",
    label: "Workspace",
    hideable: false,
    value: (row) => row.label || row.workspace_id,
    cell: (row) => (
      <div className="user-table-name">
        <RouteLink
          className="catalog-name"
          href={agentHref(row.project_id, row.node_id)}
        >
          {row.label || "Untitled workspace"}
        </RouteLink>
        <small className="user-resource-id">{row.workspace_id}</small>
      </div>
    ),
  },
  {
    id: "host",
    label: "Host",
    value: (row) => row.host_id,
    cell: (row) => <span className="user-resource-id">{row.host_id}</span>,
  },
  {
    id: "agent",
    label: "Agent",
    value: (row) => row.node_id,
    cell: (row) => <span className="user-resource-id">{row.node_id}</span>,
  },
  { id: "sharing", label: "Sharing", value: (row) => row.sharing },
];
function UserWorkspaces({
  userId,
  currentUserId,
  project,
}: {
  userId: string;
  currentUserId: string;
  project: string;
}) {
  const inventory = useUserInventory<UserWorkspace, "workspaces">(
    `${userApiPath(userId)}/projects/${encodeURIComponent(project)}/workspaces?limit=50`,
    "workspaces",
    currentUserId,
  );
  return (
    <>
      <LoadState {...inventory} retry={inventory.refresh} />
      {inventory.visibility === "personal_resources_private" ? (
        <ResourcePrivacy kind="workspaces" />
      ) : !inventory.error ? (
        <DataTable
          data={inventory.rows}
          columns={workspaceColumns}
          getRowId={(row) => row.workspace_id}
          label="User workspaces"
          itemLabel="workspaces"
          search={{ columnId: "workspace", label: "Search loaded workspaces" }}
          loading={inventory.loading}
          empty="No personal workspaces found in this project."
        />
      ) : null}
      <InventoryContinuation inventory={inventory} label="workspaces" />
    </>
  );
}
export function InventoryContinuation({
  inventory,
  label,
}: {
  inventory: {
    cursor: string | null;
    moreBusy: boolean;
    moreError: string;
    loadMore: () => Promise<void>;
  };
  label: string;
}) {
  return (
    <>
      {inventory.moreError ? (
        <p className="alert" role="alert">
          {inventory.moreError}
        </p>
      ) : null}
      {inventory.cursor ? (
        <div className="load-more">
          <p className="field-help">
            More {label} are available. Search, sorting, and row counts apply to
            loaded {label}.
          </p>
          <Button
            disabled={inventory.moreBusy}
            onClick={() => void inventory.loadMore()}
          >
            {inventory.moreBusy ? "Loading…" : `Load more ${label}`}
          </Button>
        </div>
      ) : null}
    </>
  );
}
