import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "@colossus/ui";
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
import { IconPlus } from "@tabler/icons-react";
import { useResource } from "./resources";
import { LoadState } from "./Home";
import { AnnouncementEditor } from "./AnnouncementEditor";
import { announcementPath } from "./announcement-mutation";
import { useUserInventory } from "./admin-user-data";
import { navigate } from "./navigation";
import { globalHref, settingsEditorHref } from "./routes";
import {
  SettingsEditorPage,
  type SettingsEditorNavigation,
  type SettingsEditorLocation,
} from "./settings-editor";
import type { Announcement } from "./announcements";

export function AdminAnnouncements({
  editorLocation,
  onOpenEditor,
  onCloseEditor,
  onDirtyChange,
  identity = "",
}: SettingsEditorNavigation & { identity?: string } = {}) {
  const resource = useUserInventory<Announcement, "announcements">(
    editorLocation ? null : "/api/admin/announcements?limit=50",
    "announcements",
    identity,
  );
  const [notice, setNotice] = useState("");
  const openEditor = useCallback(
    (location: SettingsEditorLocation) => {
      setNotice("");
      if (onOpenEditor) onOpenEditor(location);
      else
        navigate(
          settingsEditorHref(
            globalHref("admin", undefined, "announcements"),
            location,
          ),
        );
    },
    [onOpenEditor],
  );
  const columns = useMemo<DataTableColumn<Announcement>[]>(
    () => [
      {
        id: "title",
        label: "Announcement",
        value: (row) => row.title,
        cell: (row) => (
          <button
            type="button"
            className="catalog-name"
            onClick={() => openEditor({ action: "edit", id: row.id })}
          >
            {row.title}
          </button>
        ),
      },
      {
        id: "status",
        label: "Status",
        value: (row) => row.status,
        cell: (row) => (
          <span>
            {row.status === "published" &&
            row.expires_at &&
            Date.parse(row.expires_at) <= Date.now()
              ? "Expired"
              : row.status === "published"
                ? "Published"
                : row.status === "archived"
                  ? "Archived"
                  : "Draft"}
          </span>
        ),
      },
      {
        id: "placement",
        label: "Display",
        value: (row) =>
          row.banner ? "Banner and notification center" : "Notification center",
      },
      {
        id: "updated",
        label: "Updated",
        value: (row) => row.updated_at,
        cell: (row) => (
          <time dateTime={row.updated_at}>
            {new Date(row.updated_at).toLocaleString()}
          </time>
        ),
      },
    ],
    [openEditor],
  );
  if (editorLocation)
    return (
      <AnnouncementSettingsEditorRoute
        key={`${identity}:${editorLocation.action}:${editorLocation.id ?? ""}`}
        identity={identity}
        editorLocation={editorLocation}
        onCloseEditor={onCloseEditor}
        onDirtyChange={onDirtyChange}
        onChanged={(value) => {
          setNotice(
            value.status === "published"
              ? "Announcement published to everyone in this instance."
              : value.status === "archived"
                ? "Announcement archived."
                : "Draft saved.",
          );
          resource.refresh();
        }}
      />
    );
  return (
    <>
      <header className="catalog-heading">
        <div>
          <h2>Announcements</h2>
          <p>
            Publish Markdown updates and notices to everyone in this instance.
          </p>
        </div>
        <Button variant="primary" onClick={() => openEditor({ action: "new" })}>
          <IconPlus size={16} aria-hidden="true" />
          New announcement
        </Button>
      </header>
      <LoadState {...resource} retry={resource.refresh} />
      {resource.moreError ? (
        <p role="alert" className="alert">
          {resource.moreError}
        </p>
      ) : null}
      {notice ? (
        <p role="status" className="field-help">
          {notice}
        </p>
      ) : null}
      <DataTable
        data={resource.rows}
        columns={columns}
        getRowId={(row) => row.id}
        label="Instance announcements"
        search={{ columnId: "title", label: "Search announcements" }}
        loading={resource.loading}
        empty="No announcements yet. Create a draft to get started."
      />
      {resource.cursor ? (
        <div className="load-more">
          <p className="field-help">
            Search and sorting apply to the announcements shown.
          </p>
          <Button
            disabled={resource.moreBusy}
            onClick={() => void resource.loadMore()}
          >
            Load more announcements
          </Button>
        </div>
      ) : null}
    </>
  );
}

function AnnouncementSettingsEditorRoute({
  editorLocation,
  onCloseEditor,
  onDirtyChange,
  onChanged,
  identity,
}: SettingsEditorNavigation & {
  onChanged: (value: Announcement) => void;
  identity: string;
}) {
  const resource = useResource<{ announcement: Announcement }>(
    editorLocation?.action === "edit" && editorLocation.id
      ? announcementPath(editorLocation.id)
      : null,
    0,
    identity,
  );
  const [busy, setBusy] = useState(false);
  const [snapshot, setSnapshot] = useState<{
    announcement: Announcement;
  } | null>(null);
  const [generation, setGeneration] = useState(0);
  const reloadSource = useRef<{ announcement: Announcement } | null>(null);
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
    if (onCloseEditor) onCloseEditor();
    else navigate(globalHref("admin", undefined, "announcements"));
  }
  return (
    <SettingsEditorPage
      title={
        editorLocation?.action === "new"
          ? "New announcement"
          : "Edit announcement"
      }
      backLabel="announcements"
      onBack={close}
      busy={busy}
    >
      <LoadState {...resource} retry={resource.refresh} />
      {editorLocation?.action === "new" || snapshot ? (
        <AnnouncementEditor
          key={generation}
          announcement={
            editorLocation?.action === "new"
              ? null
              : (snapshot?.announcement ?? null)
          }
          showHeading={false}
          onBusyChange={setBusy}
          onDirtyChange={onDirtyChange}
          onCancel={close}
          onReload={reload}
          onSaved={(value) => {
            onChanged(value);
            window.dispatchEvent(
              new Event("colossus:web:announcements-changed"),
            );
            close();
          }}
        />
      ) : null}
    </SettingsEditorPage>
  );
}
