import { useState, type ReactNode } from "react";
import { Button } from "@colossus/ui";
import type { FleetNode, Thread } from "./api";
import { projectPath } from "./api";
import { useResource } from "./resources";
import { ThreadBranchDraft, type AsideDraftSource } from "./ThreadBranchDraft";

/** The parent owns the selected child; every child keeps its own feed and composer. */
export function ThreadAsidePanel({
  project,
  parent,
  nodes,
  draft,
  selected,
  disabled,
  onSelect,
  onDiscard,
  onChanged,
  renderChild,
}: {
  project: string;
  parent: Thread;
  nodes: FleetNode[];
  draft: AsideDraftSource | null;
  selected: Thread | null;
  disabled: boolean;
  onSelect: (thread: Thread | null) => void;
  onDiscard: () => void;
  onChanged: () => void;
  renderChild: (thread: Thread) => ReactNode;
}) {
  const [archived, setArchived] = useState(false);
  const [cursor, setCursor] = useState("");
  const query = new URLSearchParams({
    kind: "aside",
    archived: String(archived),
  });
  if (cursor) query.set("after", cursor);
  const history = useResource<{
    threads: Thread[];
    next_cursor?: string | null;
  }>(
    `${projectPath(project)}/threads/${encodeURIComponent(parent.thread_id)}/branches?${query}`,
    3000,
    `${project}:${parent.thread_id}:${parent.owner_user_id ?? ""}`,
  );
  return (
    <section className="thread-aside-panel" aria-label="Side chats">
      <div className="thread-aside-history">
        <div className="thread-aside-toolbar">
          <strong>Your side chats</strong>
          <Button
            variant="tertiary"
            onClick={() => {
              setArchived((value) => !value);
              setCursor("");
            }}
          >
            {archived ? "Show open" : "Show archived"}
          </Button>
        </div>
        {history.error ? (
          <p className="alert" role="alert">
            {history.error}
            <Button onClick={history.refresh}>Retry</Button>
          </p>
        ) : null}
        {history.loading ? <p role="status">Loading side chats…</p> : null}
        <nav aria-label={archived ? "Archived side chats" : "Open side chats"}>
          {history.data?.threads.map((child) => (
            <Button
              key={child.thread_id}
              variant="tertiary"
              aria-current={
                selected?.thread_id === child.thread_id ? "page" : undefined
              }
              onClick={() => onSelect(child)}
            >
              {child.title || "Untitled side chat"}
            </Button>
          ))}
        </nav>
        {!history.loading && !history.error && !history.data?.threads.length ? (
          <p className="field-help">
            {archived
              ? "No archived side chats."
              : "Select text in a message to start a side chat."}
          </p>
        ) : null}
        {history.data?.next_cursor ? (
          <Button onClick={() => setCursor(history.data!.next_cursor!)}>
            Older side chats
          </Button>
        ) : null}
        {cursor ? (
          <Button onClick={() => setCursor("")}>Back to newest</Button>
        ) : null}
        {draft && selected ? (
          <Button onClick={() => onSelect(null)}>
            Back to side-chat draft
          </Button>
        ) : null}
      </div>
      {selected ? (
        <div className="thread-aside-conversation" key={selected.thread_id}>
          {renderChild(selected)}
        </div>
      ) : null}
      {draft ? (
        <div
          hidden={Boolean(selected)}
          style={{ display: selected ? "none" : "contents" }}
        >
          <ThreadBranchDraft
            key={draft.key}
            kind="aside"
            draft={draft}
            project={project}
            nodes={nodes}
            disabled={disabled}
            onCreated={(child) => {
              onDiscard();
              onSelect(child);
              history.refresh();
            }}
            onCancel={onDiscard}
            onChanged={() => {
              history.refresh();
              onChanged();
            }}
          />
        </div>
      ) : null}
    </section>
  );
}
