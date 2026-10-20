import { useEffect, useRef, useState } from "react";
import { Button, DropdownSelect, TextInput } from "@colossus/ui";
import {
  IconLock,
  IconPlus,
  IconPlugConnected,
  IconSettings,
} from "@tabler/icons-react";
import { projectPath, type Thread } from "./api";
import { RouteLink } from "./navigation";
import { chatThreadHref, globalHref } from "./routes";
import { useResource } from "./resources";
import { LoadState } from "./Home";
import "./chat-workspace.css";

/** Personal history occupies the conversation pane beside application navigation. */
export function ChatHistorySidebar({
  project,
  identity,
  selected,
  refreshKey = 0,
  onConnections,
  onProjectSettings,
}: {
  project: string;
  identity: string;
  selected: Thread | null;
  refreshKey?: number;
  onConnections?: (() => void) | undefined;
  onProjectSettings?: (() => void) | undefined;
}) {
  const [query, setQuery] = useState("");
  const [search, setSearch] = useState("");
  const [archive, setArchive] = useState("false");
  const [cursors, setCursors] = useState([""]);
  useEffect(() => {
    const timer = setTimeout(() => setSearch(query.trim()), 250);
    return () => clearTimeout(timer);
  }, [query]);
  useEffect(() => setCursors([""]), [search, archive]);
  const params = new URLSearchParams({ limit: "25", archived: archive });
  if (search) params.set("query", search);
  if (cursors.at(-1)) params.set("after", cursors.at(-1)!);
  const history = useResource<{
    threads: Thread[];
    next_cursor?: string | null;
  }>(`${projectPath(project)}/chat/threads?${params}`, 3000, identity);
  const previousRefresh = useRef(refreshKey);
  useEffect(() => {
    if (previousRefresh.current !== refreshKey) {
      previousRefresh.current = refreshKey;
      history.refresh();
    }
  }, [refreshKey, history.refresh]);
  return (
    <section className="chat-history-sidebar" aria-label="Your chats">
      <div className="chat-history-content">
        <h2 className="chat-history-heading">Your chats</h2>
        <RouteLink
          className="ui-button ui-button--secondary chat-new-link"
          href={globalHref("chat", project)}
        >
          <IconPlus size={16} aria-hidden="true" /> New chat
        </RouteLink>
        <label>
          <span className="sr-only">Search your chats</span>
          <TextInput
            aria-label="Search your chats"
            type="search"
            placeholder="Search chats…"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>
        <DropdownSelect
          aria-label="Chat archive filter"
          value={archive}
          onChange={(event) => setArchive(event.target.value)}
        >
          <option value="false">Open chats</option>
          <option value="true">Archived chats</option>
        </DropdownSelect>
        <LoadState {...history} retry={history.refresh} />
        <nav aria-label="Your chat history">
          {history.data?.threads.map((thread) => (
            <RouteLink
              key={thread.thread_id}
              href={chatThreadHref(project, thread.thread_id)}
              className="chat-history-item"
              aria-current={
                selected?.thread_id === thread.thread_id ? "page" : undefined
              }
            >
              {thread.title || "Untitled chat"}
            </RouteLink>
          ))}
        </nav>
        {!history.loading && !history.error && !history.data?.threads.length ? (
          <p className="field-help">
            {search
              ? "No chats match your search."
              : "Your conversations will appear here."}
          </p>
        ) : null}
        <div className="chat-history-pages">
          {cursors.length > 1 ? (
            <Button onClick={() => setCursors((value) => value.slice(0, -1))}>
              Newer chats
            </Button>
          ) : null}
          {history.data?.next_cursor ? (
            <Button
              onClick={() =>
                setCursors((value) => [...value, history.data!.next_cursor!])
              }
            >
              Older chats
            </Button>
          ) : null}
        </div>
        <p className="chat-private-cue">
          <IconLock size={14} aria-hidden="true" /> Only you can open these
          chats.
        </p>
        {onConnections || onProjectSettings ? (
          <div className="chat-sidebar-management">
            {onConnections ? (
              <Button variant="tertiary" onClick={onConnections}>
                <IconPlugConnected size={16} aria-hidden="true" /> Connections
              </Button>
            ) : null}
            {onProjectSettings ? (
              <Button variant="tertiary" onClick={onProjectSettings}>
                <IconSettings size={16} aria-hidden="true" /> Project settings
              </Button>
            ) : null}
          </div>
        ) : null}
      </div>
    </section>
  );
}
