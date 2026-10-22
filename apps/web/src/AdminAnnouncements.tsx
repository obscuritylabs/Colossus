import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "@colossus/ui";
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
import { IconPlus } from "@tabler/icons-react";
import { request } from "./api";
import { useResource, errorMessage } from "./resources";
import { LoadState } from "./Home";
import { AnnouncementEditor } from "./AnnouncementEditor";
import { announcementPath } from "./announcement-mutation";
import type { Announcement } from "./announcements";

interface Inventory {
  announcements: Announcement[];
  next_cursor: string | null;
}
export function AdminAnnouncements() {
  const resource = useResource<Inventory>("/api/admin/announcements?limit=50");
  const [extra, setExtra] = useState<Announcement[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [selected, setSelected] = useState<Announcement | "new" | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const alive = useRef(true);
  const operation = useRef<AbortController | null>(null);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      operation.current?.abort();
    };
  }, []);
  useEffect(() => {
    setExtra([]);
    setCursor(resource.data?.next_cursor ?? null);
  }, [resource.data]);
  const selectEditor = useCallback((value: Announcement | "new" | null) => {
    operation.current?.abort();
    operation.current = null;
    setBusy(false);
    setError("");
    setSelected(value);
  }, []);
  const rows = useMemo(() => {
    const byId = new Map<string, Announcement>();
    for (const row of [...(resource.data?.announcements ?? []), ...extra]) {
      const previous = byId.get(row.id);
      if (!previous || row.revision > previous.revision) byId.set(row.id, row);
    }
    return [...byId.values()];
  }, [resource.data, extra]);
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
            onClick={() => selectEditor(row)}
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
    [selectEditor],
  );
  async function reload(id: string) {
    if (operation.current) return;
    const abort = new AbortController();
    operation.current = abort;
    setBusy(true);
    setError("");
    try {
      const result = await request<{ announcement: Announcement }>(
        announcementPath(id),
        undefined,
        abort.signal,
      );
      if (alive.current && !abort.signal.aborted)
        setSelected(result.announcement);
    } catch (cause) {
      if (alive.current && !abort.signal.aborted) setError(errorMessage(cause));
    } finally {
      if (operation.current === abort) {
        operation.current = null;
        if (alive.current) setBusy(false);
      }
    }
  }
  return (
    <>
      <header className="catalog-heading">
        <div>
          <h2>Announcements</h2>
          <p>
            Publish Markdown updates and notices to everyone in this instance.
          </p>
        </div>
        {!selected ? (
          <Button
            variant="primary"
            onClick={() => {
              setNotice("");
              selectEditor("new");
            }}
          >
            <IconPlus size={16} aria-hidden="true" />
            New announcement
          </Button>
        ) : null}
      </header>
      <LoadState {...resource} retry={resource.refresh} />
      {error ? (
        <p role="alert" className="alert">
          {error}
        </p>
      ) : null}
      {notice ? (
        <p role="status" className="field-help">
          {notice}
        </p>
      ) : null}
      {selected ? (
        <AnnouncementEditor
          key={
            selected === "new" ? "new" : `${selected.id}:${selected.revision}`
          }
          announcement={selected === "new" ? null : selected}
          onCancel={() => selectEditor(null)}
          onReload={(id) => void reload(id)}
          onSaved={(value) => {
            selectEditor(null);
            setNotice(
              value.status === "published"
                ? "Announcement published to everyone in this instance."
                : value.status === "archived"
                  ? "Announcement archived."
                  : "Draft saved.",
            );
            resource.refresh();
            window.dispatchEvent(
              new Event("colossus:web:announcements-changed"),
            );
          }}
        />
      ) : null}
      {!selected ? (
        <>
          <DataTable
            data={rows}
            columns={columns}
            getRowId={(row) => row.id}
            label="Instance announcements"
            search={{ columnId: "title", label: "Search announcements" }}
            loading={resource.loading}
            empty="No announcements yet. Create a draft to get started."
          />
          {cursor ? (
            <div className="load-more">
              <p className="field-help">
                Search and sorting apply to the announcements shown.
              </p>
              <Button
                disabled={busy}
                onClick={async () => {
                  if (operation.current) return;
                  const abort = new AbortController();
                  operation.current = abort;
                  setBusy(true);
                  setError("");
                  try {
                    const result = await request<Inventory>(
                      `/api/admin/announcements?limit=50&after=${encodeURIComponent(cursor)}`,
                      undefined,
                      abort.signal,
                    );
                    if (alive.current && !abort.signal.aborted) {
                      setExtra((old) => [
                        ...new Map(
                          [...old, ...result.announcements].map((row) => [
                            row.id,
                            row,
                          ]),
                        ).values(),
                      ]);
                      setCursor(result.next_cursor);
                    }
                  } catch (cause) {
                    if (alive.current && !abort.signal.aborted)
                      setError(errorMessage(cause));
                  } finally {
                    if (operation.current === abort) {
                      operation.current = null;
                      if (alive.current) setBusy(false);
                    }
                  }
                }}
              >
                Load more announcements
              </Button>
            </div>
          ) : null}
        </>
      ) : null}
    </>
  );
}
