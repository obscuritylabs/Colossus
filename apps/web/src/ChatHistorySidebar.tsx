import { useEffect, useId, useRef, useState } from "react";
import { Button, RadioGroup, WorkspaceSidebarSearch } from "@colossus/ui";
import { Button as FoundationButton } from "@colossus/ui/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@colossus/ui/components/ui/dropdown-menu";
import { useSidebarDropdown } from "@colossus/ui/hooks/useSidebarDropdown";
import {
  IconDots,
  IconLock,
  IconMessageCircle,
  IconPlus,
  IconPlugConnected,
  IconRefresh,
  IconSettings,
} from "@tabler/icons-react";
import { projectPath, type Task, type Thread } from "./api";
import { RouteLink } from "./navigation";
import { globalHref } from "./routes";
import { useResource } from "./resources";
import { LoadState } from "./Home";
import { groupChatHistory } from "./chat-history-presentation";
import { ChatHistorySidebarRow } from "./ChatHistorySidebarRow";
import "./chat-workspace.css";
import "./chat-history.css";

/** Personal history; hosts retain project, identity, query and cursor authority. */
export function ChatHistorySidebar({
  project,
  identity,
  selected,
  refreshKey = 0,
  onConnections,
  onProjectSettings,
  canControl = false,
  onThreadChanged,
  onFork,
}: {
  project: string;
  identity: string;
  selected: Thread | null;
  refreshKey?: number;
  onConnections?: (() => void) | undefined;
  onProjectSettings?: (() => void) | undefined;
  canControl?: boolean;
  onThreadChanged?: ((thread: Thread) => void) | undefined;
  onFork?: ((thread: Thread, source: Task) => void) | undefined;
}) {
  const [query, setQuery] = useState("");
  const [{ search, archive, cursors }, setView] = useState({
    search: "",
    archive: "false",
    cursors: [""],
  });
  useEffect(() => {
    const timer = setTimeout(() => {
      const search = query.trim();
      setView((current) =>
        current.search === search
          ? current
          : { ...current, search, cursors: [""] },
      );
    }, 250);
    return () => clearTimeout(timer);
  }, [query]);
  const params = new URLSearchParams({ limit: "25", archived: archive });
  if (search) params.set("query", search);
  if (cursors.at(-1)) params.set("after", cursors.at(-1)!);
  const history = useResource<{
    threads: Thread[];
    next_cursor?: string | null;
  }>(projectPath(project) + "/chat/threads?" + params, 3000, identity);
  const previousRefresh = useRef(refreshKey);
  useEffect(() => {
    if (previousRefresh.current !== refreshKey) {
      previousRefresh.current = refreshKey;
      history.refresh();
    }
  }, [refreshKey, history.refresh]);
  const menu = useSidebarDropdown();
  const sidebarRef = useRef<HTMLElement>(null);
  const metadataId = useId();
  const now = new Date();
  const groups = groupChatHistory(history.data?.threads ?? [], now);
  const hasPrevious = cursors.length > 1;
  const hasNext = Boolean(history.data?.next_cursor);
  return (
    <section
      ref={sidebarRef}
      className="chat-history-sidebar"
      aria-label="Your chats"
    >
      <header className="chat-history-toolbar">
        <div className="chat-history-title">
          <h2>Chats</h2>
          <span
            className="chat-history-private"
            title="Only you can open these chats."
          >
            <IconLock size={12} aria-hidden="true" /> Private
          </span>
        </div>
        <div className="chat-history-actions">
          <FoundationButton asChild variant="ghost" size="icon">
            <RouteLink
              href={globalHref("chat", project)}
              aria-label="New chat"
              title="New chat"
            >
              <IconPlus size={18} aria-hidden="true" />
            </RouteLink>
          </FoundationButton>
          <DropdownMenu
            open={menu.open}
            onOpenChange={menu.onOpenChange}
            modal={menu.modal}
          >
            <DropdownMenuTrigger asChild>
              <FoundationButton
                ref={menu.triggerRef}
                variant="ghost"
                size="icon"
                aria-label="Chat options"
                title="Chat options"
              >
                <IconDots size={18} aria-hidden="true" />
              </FoundationButton>
            </DropdownMenuTrigger>
            <DropdownMenuContent
              ref={menu.contentRef}
              container={menu.portalContainer}
              className="control-sidebar-dropdown"
              align="end"
              collisionPadding={8}
            >
              <DropdownMenuItem
                className="control-sidebar-dropdown-item"
                onSelect={history.refresh}
              >
                <IconRefresh size={16} aria-hidden="true" /> Refresh chats
              </DropdownMenuItem>
              {onConnections ? (
                <DropdownMenuItem
                  className="control-sidebar-dropdown-item"
                  onSelect={onConnections}
                >
                  <IconPlugConnected size={16} aria-hidden="true" /> Connections
                </DropdownMenuItem>
              ) : null}
              {onProjectSettings ? (
                <DropdownMenuItem
                  className="control-sidebar-dropdown-item"
                  onSelect={onProjectSettings}
                >
                  <IconSettings size={16} aria-hidden="true" /> Project settings
                </DropdownMenuItem>
              ) : null}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </header>
      <div className="chat-history-controls">
        <WorkspaceSidebarSearch
          className="chat-history-search"
          label="Search your chats"
          placeholder="Search chats…"
          value={query}
          onChange={setQuery}
        />
        <div className="chat-history-filter">
          <RadioGroup
            aria-label="Chat history view"
            variant="compact"
            value={archive}
            onValueChange={(archive) =>
              setView((current) =>
                current.archive === archive
                  ? current
                  : { ...current, archive, cursors: [""] },
              )
            }
            options={[
              { value: "false", label: "Open" },
              { value: "true", label: "Archived" },
            ]}
          />
        </div>
      </div>
      <div className="chat-history-scroll">
        <LoadState {...history} retry={history.refresh} />
        <nav className="chat-history-list" aria-label="Your chat history">
          {groups.map((group) => (
            <section
              key={group.label + ":" + group.threads[0]?.thread_id}
              className="chat-history-group"
              aria-label={group.label}
            >
              <h3 className="chat-history-group-heading">{group.label}</h3>
              {group.threads.map((thread) => (
                <ChatHistorySidebarRow
                  key={JSON.stringify([
                    identity,
                    project,
                    thread.thread_id,
                    thread.node_id,
                    thread.owner_user_id,
                    thread.session_id,
                  ])}
                  thread={thread}
                  project={project}
                  selected={selected?.thread_id === thread.thread_id}
                  now={now}
                  metadataId={metadataId + "-" + thread.thread_id}
                  identity={identity}
                  canControl={canControl}
                  onFork={onFork}
                  onRefresh={history.refresh}
                  onChanged={(value) => {
                    history.refresh();
                    onThreadChanged?.(value);
                  }}
                  onRemovedFocus={() => {
                    sidebarRef.current
                      ?.querySelector<HTMLAnchorElement>(
                        '[aria-label="New chat"]',
                      )
                      ?.focus();
                  }}
                />
              ))}
            </section>
          ))}
        </nav>
        {!history.loading && !history.error && !groups.length ? (
          <div className="chat-history-empty">
            <IconMessageCircle size={24} aria-hidden="true" />
            <strong>
              {search
                ? "No matching chats"
                : archive === "true"
                  ? "No archived chats"
                  : "Start a conversation"}
            </strong>
            <p>
              {search
                ? "Try another chat title or clear your search."
                : archive === "true"
                  ? "Chats you archive will appear here."
                  : "Your personal chats will appear here."}
            </p>
          </div>
        ) : null}
      </div>
      {hasPrevious || hasNext ? (
        <footer className="chat-history-pages" aria-label="Chat history pages">
          {hasPrevious ? (
            <Button
              variant="tertiary"
              onClick={() =>
                setView((current) => ({
                  ...current,
                  cursors: current.cursors.slice(0, -1),
                }))
              }
            >
              Newer chats
            </Button>
          ) : null}
          {hasNext ? (
            <Button
              variant="tertiary"
              onClick={() =>
                setView((current) => ({
                  ...current,
                  cursors: [...current.cursors, history.data!.next_cursor!],
                }))
              }
            >
              Older chats
            </Button>
          ) : null}
        </footer>
      ) : null}
    </section>
  );
}
