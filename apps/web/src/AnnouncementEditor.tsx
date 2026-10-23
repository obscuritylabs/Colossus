import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { Button, DropdownSelect, TextInput } from "@colossus/ui";
import { AnnouncementBanner } from "@colossus/ui/components/Announcements";
import {
  ANNOUNCEMENT_BODY_BYTES,
  type Announcement,
  type AnnouncementTone,
} from "./announcements";
import { useAnnouncementEditor } from "./useAnnouncementEditor";
import { SectionTabs } from "./Projects";
import { WebLink } from "./WebLink";
import { useSettingsNavigationGuard } from "./navigation";

const MarkdownPreview = lazy(() =>
  import("@colossus/ui/components/MarkdownContent").then((module) => ({
    default: module.MarkdownContent,
  })),
);

function localDateTime(value: string | null): string {
  if (!value) return "";
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return "";
  return new Date(date.getTime() - date.getTimezoneOffset() * 60_000)
    .toISOString()
    .slice(0, 16);
}

function limitCharacters(value: string, limit: number): string {
  let result = "";
  let count = 0;
  for (const character of value) {
    if (count++ === limit) break;
    result += character;
  }
  return result;
}

export function AnnouncementEditor({
  announcement,
  onSaved,
  onCancel,
  onReload,
  showHeading = true,
  onBusyChange,
  onDirtyChange,
}: {
  announcement: Announcement | null;
  showHeading?: boolean;
  onBusyChange?: ((busy: boolean) => void) | undefined;
  onDirtyChange?: ((dirty: boolean) => void) | undefined;
  onSaved: (value: Announcement) => void;
  onCancel: () => void;
  onReload: (id: string) => void;
}) {
  const localDirty = useSettingsNavigationGuard();
  const dirty = onDirtyChange ?? localDirty;
  const changed = useRef(false);
  const editor = useAnnouncementEditor(announcement, (value) => {
    changed.current = false;
    dirty(false);
    onSaved(value);
  });
  useEffect(() => {
    onBusyChange?.(editor.busy);
    dirty(changed.current || editor.busy || Boolean(editor.uncertain));
    return () => {
      onBusyChange?.(false);
      dirty(false);
    };
  }, [editor.busy, editor.uncertain, dirty, onBusyChange]);
  const { content } = editor;
  const setContent: typeof editor.setContent = (value) => {
    changed.current = true;
    dirty(true);
    editor.setContent(value);
  };
  const [mode, setMode] = useState("write");
  const bytes = new TextEncoder().encode(content.body).length;
  const expired =
    content.expires_at !== null && Date.parse(content.expires_at) <= Date.now();
  return (
    <section
      className="control-card announcement-editor"
      aria-label="Announcement editor"
    >
      {showHeading ? (
        <header>
          <h2>{announcement ? "Edit announcement" : "New announcement"}</h2>
        </header>
      ) : null}
      <p className="field-help">
        Published announcements are visible to everyone signed in to this
        Control Plane. Drafts stay private to administrators.
      </p>
      {editor.error ? (
        <p className="alert" role="alert">
          {editor.error}
        </p>
      ) : null}
      <form
        className="management-form"
        onChange={() => {
          changed.current = true;
          dirty(true);
        }}
        onSubmit={(event) => {
          event.preventDefault();
          editor.save("publish");
        }}
      >
        <fieldset className="announcement-fields" disabled={editor.locked}>
          <label>
            <span>Title</span>
            <TextInput
              value={content.title}
              required
              onChange={(event) =>
                setContent({
                  ...content,
                  title: limitCharacters(event.target.value, 120),
                })
              }
            />
            <small>{[...content.title].length} / 120 characters</small>
          </label>
          <label>
            <span>Banner summary</span>
            <TextInput
              value={content.summary}
              onChange={(event) =>
                setContent({
                  ...content,
                  summary: limitCharacters(event.target.value, 240),
                })
              }
            />
            <small>
              {[...content.summary].length} / 240 characters · A short
              introduction for the compact announcement banner.
            </small>
          </label>
          <div className="form-pair">
            <label>
              <span>Style</span>
              <DropdownSelect
                value={content.tone}
                onChange={(event) =>
                  setContent({
                    ...content,
                    tone: event.target.value as AnnouncementTone,
                  })
                }
              >
                <option value="info">Information</option>
                <option value="success">New feature</option>
                <option value="warning">Warning</option>
                <option value="danger">Important</option>
              </DropdownSelect>
            </label>
            <label>
              <span>Priority</span>
              <DropdownSelect
                value={String(content.priority)}
                onChange={(event) =>
                  setContent({
                    ...content,
                    priority: Number(event.target.value),
                  })
                }
              >
                <option value="0">Low</option>
                <option value="50">Normal</option>
                <option value="100">High</option>
                {![0, 50, 100].includes(content.priority) ? (
                  <option value={String(content.priority)}>
                    Custom ({content.priority})
                  </option>
                ) : null}
              </DropdownSelect>
            </label>
          </div>
          <label className="checkbox-field">
            <input
              type="checkbox"
              checked={content.banner}
              onChange={(event) =>
                setContent({ ...content, banner: event.target.checked })
              }
            />
            <span>Show a dismissible banner</span>
          </label>
          <label>
            <span>Expires at (optional)</span>
            <input
              className="ui-input"
              type="datetime-local"
              value={localDateTime(content.expires_at)}
              onChange={(event) => {
                const date = event.target.value
                  ? new Date(event.target.value)
                  : null;
                setContent({
                  ...content,
                  expires_at:
                    date && Number.isFinite(date.getTime())
                      ? date.toISOString()
                      : null,
                });
              }}
            />
            <small>
              Uses your local time. Leave empty to keep the announcement
              available until archived.
            </small>
          </label>
          {expired ? (
            <p className="field-help" role="status">
              This expiration has passed. Choose a future time before
              publishing.
            </p>
          ) : null}
        </fieldset>
        <div>
          <SectionTabs
            items={[
              { id: "write", label: "Write" },
              { id: "preview", label: "Preview" },
            ]}
            current={mode}
            onChange={setMode}
          />
          {mode === "write" ? (
            <label>
              <span>Message (Markdown)</span>
              <textarea
                className="ui-input announcement-markdown-input"
                rows={12}
                value={content.body}
                disabled={editor.locked}
                onChange={(event) =>
                  setContent({ ...content, body: event.target.value })
                }
                required
              />
              <small>
                {bytes.toLocaleString()} /{" "}
                {ANNOUNCEMENT_BODY_BYTES.toLocaleString()} bytes · Links, lists,
                and code are supported.
              </small>
            </label>
          ) : (
            <div
              className="announcement-editor-preview"
              aria-label="Announcement preview"
            >
              {content.banner ? (
                <AnnouncementBanner
                  announcement={{
                    id: editor.id,
                    publicationRevision:
                      announcement?.publication_revision ?? 0,
                    title: content.title || "Announcement title",
                    summary: content.summary,
                    body: content.body,
                    tone: content.tone,
                    publishedAt: announcement?.published_at ?? "",
                    read: false,
                    dismissed: false,
                  }}
                  onRead={() =>
                    document
                      .getElementById("announcement-preview-body")
                      ?.focus()
                  }
                />
              ) : null}
              <div id="announcement-preview-body" tabIndex={-1}>
                <h3>{content.title || "Announcement title"}</h3>
                <Suspense fallback={<p role="status">Preparing preview…</p>}>
                  <MarkdownPreview
                    content={
                      content.body ||
                      "Your announcement preview will appear here."
                    }
                    linkComponent={WebLink}
                  />
                </Suspense>
              </div>
            </div>
          )}
        </div>
        {editor.uncertain ? (
          <div className="form-actions">
            <Button
              onClick={() => void editor.checkStatus()}
              disabled={editor.busy}
            >
              Check save status
            </Button>
            <Button onClick={editor.retry} disabled={editor.busy}>
              Retry this exact save
            </Button>
          </div>
        ) : (
          <div className="form-actions">
            <Button
              type="submit"
              variant="primary"
              disabled={
                editor.locked || bytes > ANNOUNCEMENT_BODY_BYTES || expired
              }
            >
              {announcement?.status === "published"
                ? "Publish changes"
                : "Publish to everyone"}
            </Button>
            {announcement?.status !== "published" ? (
              <Button
                onClick={() => editor.save("draft")}
                disabled={editor.locked}
              >
                Save draft
              </Button>
            ) : null}
            {announcement && announcement.status !== "archived" ? (
              <Button
                variant="danger"
                onClick={() => editor.save("archive")}
                disabled={editor.locked}
              >
                Archive announcement
              </Button>
            ) : null}
          </div>
        )}
        <div className="form-actions">
          {editor.conflict || editor.uncertain ? (
            <Button disabled={editor.busy} onClick={() => onReload(editor.id)}>
              Reload current version
            </Button>
          ) : null}
          <Button onClick={onCancel} disabled={editor.busy}>
            Back to announcements
          </Button>
        </div>
      </form>
    </section>
  );
}
