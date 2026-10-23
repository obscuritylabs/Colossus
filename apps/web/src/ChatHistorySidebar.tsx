import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
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
  IconChevronDown,
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
import { useChatPins } from "./chat-pins";
import { usePinnedChats } from "./usePinnedChats";
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
  const pins = useChatPins(identity, project);
  const [searchDisclosure, setSearchDisclosure] = useState<{
    query: string;
    collapsed: boolean;
  } | null>(null);
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
  const pinnedHistory = usePinnedChats(
    identity,
    project,
    archive === "false" ? pins.ids : [],
    refreshKey,
  );
  const refresh = () => {
    history.refresh();
    pinnedHistory.refresh();
  };
  const previousRefresh = useRef(refreshKey);
  useEffect(() => {
    if (previousRefresh.current !== refreshKey) {
      previousRefresh.current = refreshKey;
      history.refresh();
    }
  }, [refreshKey, history.refresh]);
  const menu = useSidebarDropdown();
  const sidebarRef = useRef<HTMLElement>(null);
  const pinnedHeadingRef = useRef<HTMLButtonElement>(null);
  const pinFocus = useRef<string | null>(null);
  const [pinFocusVersion, setPinFocusVersion] = useState(0);
  const changePin = (id: string, pinned: boolean) => {
    pinFocus.current = id;
    if (pinned) {
      pins.setCollapsed(false);
      setSearchDisclosure(null);
    }
    pins.setPinned(id, pinned);
    setPinFocusVersion((value) => value + 1);
  };
  useLayoutEffect(() => {
    if (!pinFocus.current) return;
    const row = [
      ...(sidebarRef.current?.querySelectorAll<HTMLElement>(
        "[data-chat-row]",
      ) ?? []),
    ].find((row) => row.dataset.chatRow === pinFocus.current);
    pinFocus.current = null;
    const target =
      row?.querySelector<HTMLButtonElement>(
        '[aria-label^="Chat actions for"]',
      ) ??
      sidebarRef.current?.querySelector<HTMLAnchorElement>(
        '[aria-label="New chat"]',
      );
    target?.focus();
  }, [pins.ids, pinFocusVersion]);
  const metadataId = useId();
  const pinnedListId = metadataId + "-pinned";
  const pinnedExpanded = search
    ? searchDisclosure?.query === search
      ? !searchDisclosure.collapsed
      : true
    : !pins.collapsed;
  const now = new Date();
  const pinIds = new Set(pins.ids);
  const releasedPins = new Map(
    pinnedHistory.threads.map((thread) => [thread.thread_id, thread]),
  );
  for (const thread of history.data?.threads ?? []) {
    if (
      pinIds.has(thread.thread_id) &&
      thread.project_id === project &&
      thread.owner_user_id === identity &&
      thread.source === "cloud" &&
      (releasedPins.get(thread.thread_id)?.revision ?? -1) <= thread.revision
    )
      releasedPins.set(thread.thread_id, thread);
  }
  const pinnedThreads =
    archive === "false"
      ? pins.ids.flatMap((id) => {
          const thread = releasedPins.get(id);
          return thread &&
            !thread.archived &&
            thread.title.toLowerCase().includes(search.toLowerCase())
            ? [thread]
            : [];
        })
      : [];
  const groups = groupChatHistory(
    (history.data?.threads ?? []).filter(
      (thread) => archive !== "false" || !pinIds.has(thread.thread_id),
    ),
    now,
  );
  if (pinnedThreads.length)
    groups.unshift({ label: "Pinned", threads: pinnedThreads });
  const unavailablePins = pinnedHistory.unavailable.filter(
    (id) => !releasedPins.has(id),
  );
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
                onSelect={refresh}
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
          onChange={(value) => {
            setQuery(value);
            setSearchDisclosure(null);
          }}
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
      {pins.notice ? (
        <p className="chat-history-pin-notice" role="status">
          {pins.notice}
        </p>
      ) : null}
      <div className="chat-history-scroll">
        <LoadState {...history} retry={history.refresh} />
        <nav className="chat-history-list" aria-label="Your chat history">
          {groups.map((group) => (
            <section
              key={
                group.label === "Pinned"
                  ? "Pinned"
                  : group.label + ":" + group.threads[0]?.thread_id
              }
              className="chat-history-group"
              aria-label={group.label}
            >
              <h3
                className={`chat-history-group-heading${group.label === "Pinned" ? " is-collapsible" : ""}`}
              >
                {group.label === "Pinned" ? (
                  <FoundationButton
                    ref={pinnedHeadingRef}
                    variant="ghost"
                    className="chat-history-group-toggle"
                    aria-label="Pinned"
                    aria-expanded={pinnedExpanded}
                    aria-controls={pinnedListId}
                    onClick={() => {
                      if (search)
                        setSearchDisclosure({
                          query: search,
                          collapsed: pinnedExpanded,
                        });
                      else pins.setCollapsed(pinnedExpanded);
                    }}
                  >
                    Pinned
                    <IconChevronDown size={14} aria-hidden="true" />
                  </FoundationButton>
                ) : (
                  group.label
                )}
              </h3>
              <div
                className="chat-history-group-items"
                id={group.label === "Pinned" ? pinnedListId : undefined}
                hidden={group.label === "Pinned" && !pinnedExpanded}
              >
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
                    pinned={pinIds.has(thread.thread_id)}
                    visible={group.label !== "Pinned" || pinnedExpanded}
                    onHiddenFocus={() => pinnedHeadingRef.current?.focus()}
                    onPin={(pinned) => {
                      if (
                        thread.project_id === project &&
                        thread.owner_user_id === identity
                      ) {
                        changePin(thread.thread_id, pinned);
                      }
                    }}
                    canControl={canControl}
                    onFork={onFork}
                    onRefresh={refresh}
                    onChanged={(value) => {
                      refresh();
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
              </div>
            </section>
          ))}
        </nav>
        {!search &&
        archive === "false" &&
        pinnedHistory.loading &&
        pins.ids.length ? (
          <p className="chat-history-pin-notice" role="status">
            Loading pinned chats…
          </p>
        ) : null}
        {!search && archive === "false"
          ? unavailablePins.map((id) => (
              <div className="chat-history-pin-unavailable" key={id}>
                Pinned chat unavailable.
                <Button variant="tertiary" onClick={() => changePin(id, false)}>
                  Remove pin
                </Button>
              </div>
            ))
          : null}
        {!history.loading &&
        !history.error &&
        !groups.length &&
        !(
          archive === "false" &&
          !search &&
          (pinnedHistory.loading || unavailablePins.length)
        ) ? (
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
