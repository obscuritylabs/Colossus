import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { WorkspaceSidebarThreadContent } from "@colossus/ui";
import { Button } from "@colossus/ui/components/ui/button";
import { Input } from "@colossus/ui/components/ui/input";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@colossus/ui/components/ui/dropdown-menu";
import { useSidebarDropdown } from "@colossus/ui/hooks/useSidebarDropdown";
import {
  IconAlertCircle,
  IconArchive,
  IconArchiveOff,
  IconClock,
  IconDots,
  IconGitFork,
  IconLock,
  IconPencil,
  IconPin,
} from "@tabler/icons-react";
import {
  ApiFailure,
  projectPath,
  request,
  type Task,
  type Thread,
  type ThreadDetailResponse,
} from "./api";
import {
  chatTitleError,
  latestChatForkSource,
  sameChatBinding,
} from "./ChatHistorySidebar.actions";
import { chatUpdatedTime } from "./chat-history-presentation";
import { RouteLink } from "./navigation";
import { errorMessage } from "./resources";
import { chatThreadHref } from "./routes";

export function ChatHistorySidebarRow({
  thread,
  project,
  identity,
  selected,
  now,
  metadataId,
  canControl,
  onChanged,
  onRemovedFocus,
  onRefresh,
  onFork,
  pinned = false,
  onPin,
  visible = true,
  onHiddenFocus,
}: {
  thread: Thread;
  project: string;
  identity: string;
  selected: boolean;
  now: Date;
  metadataId: string;
  canControl: boolean;
  onChanged: (thread: Thread) => void;
  onRemovedFocus: () => void;
  onRefresh: () => void;
  onFork: ((thread: Thread, source: Task) => void) | undefined;
  pinned?: boolean;
  onPin?: ((pinned: boolean) => void) | undefined;
  visible?: boolean;
  onHiddenFocus?: (() => void) | undefined;
}) {
  const menu = useSidebarDropdown();
  const [bound, setBound] = useState(thread);
  const [editing, setEditing] = useState<Thread | null>(null);
  const [titleDraft, setTitleDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [fork, setFork] = useState<{
    loading: boolean;
    detail: ThreadDetailResponse | null;
    error: string;
  }>({ loading: false, detail: null, error: "" });
  const pending = useRef(false);
  const alive = useRef(true);
  const rowRef = useRef<HTMLDivElement>(null);
  const renameRef = useRef<HTMLFormElement>(null);
  const titleRef = useRef<HTMLInputElement>(null);
  const skipMenuFocus = useRef(false);
  const restoreTriggerFocus = useRef(false);
  const detailAbort = useRef<AbortController | null>(null);
  const visibleRef = useRef(visible);
  const hiddenHadFocus = useRef(false);
  const hiddenFocusPending = useRef(false);
  if (visibleRef.current && !visible) {
    // Capture before the parent's hidden attribute can move focus to the body.
    hiddenHadFocus.current = Boolean(
      rowRef.current?.contains(document.activeElement) ||
      menu.contentRef.current?.contains(document.activeElement),
    );
  }
  visibleRef.current = visible;
  const owned =
    thread.project_id === project && thread.owner_user_id === identity;
  const allowed = canControl && owned;
  const showActions =
    owned &&
    (canControl ||
      Boolean(onPin && (!thread.archived || pinned)) ||
      Boolean(onFork && !thread.archived && thread.can_continue));
  const title = thread.title || "Untitled chat";
  const updated = chatUpdatedTime(thread.updated_at, now);
  const state =
    thread.sync_status === "incomplete"
      ? "History incomplete"
      : thread.can_continue === false
        ? "Read only"
        : thread.active_task_id || thread.queued_task_ids?.length
          ? "Pending work"
          : "";
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      detailAbort.current?.abort();
    };
  }, []);
  useEffect(() => {
    if (!menu.open) detailAbort.current?.abort();
  }, [menu.open]);
  useLayoutEffect(() => {
    if (visible) {
      hiddenHadFocus.current = false;
      hiddenFocusPending.current = false;
      return;
    }
    const hadFocus = Boolean(
      rowRef.current?.contains(document.activeElement) ||
      menu.contentRef.current?.contains(document.activeElement) ||
      (hiddenHadFocus.current && document.activeElement === document.body),
    );
    hiddenHadFocus.current = false;
    restoreTriggerFocus.current = false;
    detailAbort.current?.abort();
    if (menu.open) {
      hiddenFocusPending.current = hadFocus;
      skipMenuFocus.current = true;
      menu.onOpenChange(false);
    } else if (hadFocus && !hiddenFocusPending.current) onHiddenFocus?.();
  }, [visible, menu.open, menu.onOpenChange, menu.contentRef, onHiddenFocus]);
  useEffect(() => {
    if (!editing && !busy && restoreTriggerFocus.current) {
      restoreTriggerFocus.current = false;
      if (visibleRef.current) menu.triggerRef.current?.focus();
    }
  }, [editing, busy, menu.triggerRef]);
  useEffect(() => {
    if (!editing || !visibleRef.current) return;
    titleRef.current?.focus();
    titleRef.current?.select();
    const escape = (event: KeyboardEvent) => {
      if (
        event.key === "Escape" &&
        visibleRef.current &&
        !event.isComposing &&
        event.target instanceof Node &&
        renameRef.current?.contains(event.target) &&
        !pending.current
      ) {
        event.preventDefault();
        event.stopPropagation();
        setEditing(null);
        setError("");
        restoreTriggerFocus.current = true;
      }
    };
    window.addEventListener("keydown", escape, true);
    return () => window.removeEventListener("keydown", escape, true);
  }, [editing, menu.triggerRef]);

  async function openMenu(open: boolean) {
    if (open && !visibleRef.current) return;
    menu.onOpenChange(open);
    detailAbort.current?.abort();
    if (!open) return;
    setBound(thread);
    setError("");
    setFork({ loading: false, detail: null, error: "" });
    if (!owned || !onFork || thread.archived || !thread.can_continue) return;
    const abort = new AbortController();
    detailAbort.current = abort;
    setFork({ loading: true, detail: null, error: "" });
    try {
      const detail = await request<ThreadDetailResponse>(
        `${projectPath(project)}/threads/${encodeURIComponent(thread.thread_id)}`,
        undefined,
        abort.signal,
      );
      if (!abort.signal.aborted && alive.current)
        setFork({ loading: false, detail, error: "" });
    } catch (error) {
      if (!abort.signal.aborted && alive.current)
        setFork({ loading: false, detail: null, error: errorMessage(error) });
    }
  }

  async function mutate(
    target: Thread,
    values: { title?: string; archived?: boolean },
  ) {
    if (!allowed || pending.current) return;
    const actionHadFocus = Boolean(
      rowRef.current?.contains(document.activeElement) ||
      menu.contentRef.current?.contains(document.activeElement),
    );
    const mayRestoreFocus = () =>
      visibleRef.current &&
      actionHadFocus &&
      (document.activeElement === document.body ||
        rowRef.current?.contains(document.activeElement) ||
        menu.contentRef.current?.contains(document.activeElement));
    pending.current = true;
    setBusy(true);
    setError("");
    try {
      const result = await request<{ thread: Thread }>(
        `${projectPath(project)}/threads/${encodeURIComponent(target.thread_id)}`,
        { revision: target.revision, ...values },
        undefined,
        "PATCH",
      );
      if (!sameChatBinding(target, result.thread))
        throw new Error(
          "The returned chat no longer matches this row. Refresh your chats.",
        );
      if (!alive.current) return;
      setEditing(null);
      if (mayRestoreFocus()) {
        if (values.archived !== undefined) onRemovedFocus();
        else restoreTriggerFocus.current = true;
      }
      onChanged(result.thread);
    } catch (error) {
      if (alive.current) {
        setError(errorMessage(error));
        // Conflicts require a fresh row and an explicit new action, never an automatic retry.
        if (error instanceof ApiFailure && error.status === 409) {
          setEditing(null);
          restoreTriggerFocus.current = Boolean(mayRestoreFocus());
          onRefresh();
        }
      }
    } finally {
      pending.current = false;
      if (alive.current) setBusy(false);
    }
  }

  const source = fork.detail
    ? latestChatForkSource(bound, fork.detail, identity)
    : null;
  const boundTitle = bound.title || "Untitled chat";
  const boundUpdated = chatUpdatedTime(bound.updated_at, now);
  const forkNotice = fork.loading
    ? "Checking fork availability…"
    : fork.error ||
      (!source ? "Fork is available after an eligible run finishes." : "");
  return (
    <div
      className="chat-history-entry"
      ref={rowRef}
      data-chat-row={thread.thread_id}
      aria-busy={busy || undefined}
    >
      {editing ? (
        <form
          ref={renameRef}
          className="chat-history-rename"
          aria-label={`Rename ${title}`}
          onSubmit={(event) => {
            event.preventDefault();
            const validation = chatTitleError(titleDraft);
            if (validation) setError(validation);
            else void mutate(editing, { title: titleDraft.trim() });
          }}
        >
          <Input
            ref={titleRef}
            aria-label="Chat title"
            aria-describedby={error ? `${metadataId}-error` : undefined}
            aria-invalid={Boolean(error)}
            value={titleDraft}
            disabled={busy}
            onChange={(event) => setTitleDraft(event.target.value)}
          />
          <div className="chat-history-rename-actions">
            <Button type="submit" variant="secondary" disabled={busy}>
              {busy ? "Saving…" : "Save"}
            </Button>
            <Button
              variant="ghost"
              disabled={busy}
              onClick={() => {
                setEditing(null);
                setError("");
                restoreTriggerFocus.current = true;
              }}
            >
              Cancel
            </Button>
          </div>
        </form>
      ) : (
        <RouteLink
          href={chatThreadHref(project, thread.thread_id)}
          className={`shared-workspace-thread-link is-compact chat-history-row${showActions ? " chat-history-row-with-actions" : ""}`}
          aria-label={title}
          aria-describedby={metadataId}
          aria-current={selected ? "page" : undefined}
          title={`${title}\n${updated.dateTime ? "Updated " : ""}${updated.exact}${state ? ` · ${state}` : ""}`}
        >
          <WorkspaceSidebarThreadContent
            compact
            title={title}
            metadata={
              <span id={metadataId} className="chat-history-meta">
                <time dateTime={updated.dateTime} title={updated.exact}>
                  {updated.dateTime ? "Updated " : ""}
                  {updated.label}
                </time>
                {pinned ? <span>Pinned</span> : null}
                {state ? (
                  <span className="chat-history-row-state">
                    {state === "Pending work" ? (
                      <IconClock size={12} aria-hidden="true" />
                    ) : null}
                    {state}
                  </span>
                ) : null}
              </span>
            }
            status={
              state || pinned ? (
                <span className="chat-history-row-indicator" aria-hidden="true">
                  {pinned ? <IconPin size={14} /> : null}
                  {state === "Pending work" ? (
                    <IconClock size={14} />
                  ) : state === "Read only" ? (
                    <IconLock size={14} />
                  ) : state ? (
                    <IconAlertCircle size={14} />
                  ) : null}
                </span>
              ) : undefined
            }
          />
        </RouteLink>
      )}
      {showActions ? (
        <DropdownMenu
          open={menu.open}
          onOpenChange={(open) => void openMenu(open)}
          modal={menu.modal}
        >
          <DropdownMenuTrigger asChild>
            <Button
              ref={menu.triggerRef}
              className="chat-history-row-actions"
              variant="ghost"
              size="icon"
              aria-label={`Chat actions for ${title}`}
              disabled={busy || Boolean(editing)}
            >
              <IconDots size={17} aria-hidden="true" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            ref={menu.contentRef}
            container={menu.portalContainer}
            className="control-sidebar-dropdown chat-history-row-menu"
            align="end"
            collisionPadding={8}
            aria-label={`Actions for ${title}`}
            onCloseAutoFocus={(event) => {
              if (!visibleRef.current || skipMenuFocus.current) {
                event.preventDefault();
                skipMenuFocus.current = false;
              }
              if (hiddenFocusPending.current) {
                hiddenFocusPending.current = false;
                // Restore after the modal menu's focus trap has been removed,
                // and leave any focus the user moved elsewhere untouched.
                if (
                  !visibleRef.current &&
                  (document.activeElement === document.body ||
                    rowRef.current?.contains(document.activeElement) ||
                    menu.contentRef.current?.contains(document.activeElement))
                )
                  onHiddenFocus?.();
              }
            }}
          >
            <DropdownMenuLabel className="chat-history-menu-context">
              <strong>{boundTitle}</strong>
              <time dateTime={boundUpdated.dateTime} title={boundUpdated.exact}>
                {boundUpdated.dateTime ? "Updated " : ""}
                {boundUpdated.label}
              </time>
            </DropdownMenuLabel>
            <hr className="chat-history-menu-separator" />
            <DropdownMenuGroup aria-label="Chat actions">
              {allowed ? (
                <DropdownMenuItem
                  className="control-sidebar-dropdown-item"
                  onSelect={() => {
                    skipMenuFocus.current = true;
                    setTitleDraft(bound.title);
                    setEditing(bound);
                  }}
                >
                  <IconPencil size={16} aria-hidden="true" /> Rename
                </DropdownMenuItem>
              ) : null}
              {onFork && !bound.archived ? (
                <>
                  <DropdownMenuItem
                    className="control-sidebar-dropdown-item"
                    disabled={!source}
                    aria-describedby={
                      !source ? `${metadataId}-fork` : undefined
                    }
                    onSelect={() => {
                      if (source && fork.detail) {
                        skipMenuFocus.current = true;
                        onFork(fork.detail.thread, source);
                      }
                    }}
                  >
                    <IconGitFork size={16} aria-hidden="true" /> Fork chat
                  </DropdownMenuItem>
                  {forkNotice ? (
                    <p
                      className="chat-history-menu-notice"
                      id={`${metadataId}-fork`}
                      role="status"
                    >
                      {forkNotice}
                    </p>
                  ) : null}
                </>
              ) : null}
              {onPin && (!bound.archived || pinned) ? (
                <DropdownMenuCheckboxItem
                  className="control-sidebar-dropdown-item"
                  checked={pinned}
                  aria-label={`${pinned ? "Unpin" : "Pin"} ${title}`}
                  onSelect={() => {
                    skipMenuFocus.current = true;
                    onPin(!pinned);
                  }}
                >
                  <IconPin size={16} aria-hidden="true" />
                  {pinned ? "Unpin" : "Pin"}
                </DropdownMenuCheckboxItem>
              ) : null}
            </DropdownMenuGroup>
            {allowed ? (
              <>
                <hr className="chat-history-menu-separator" />
                <DropdownMenuGroup aria-label="Chat history">
                  <DropdownMenuItem
                    className="control-sidebar-dropdown-item"
                    onSelect={() =>
                      void mutate(bound, { archived: !bound.archived })
                    }
                  >
                    {bound.archived ? (
                      <IconArchiveOff size={16} aria-hidden="true" />
                    ) : (
                      <IconArchive size={16} aria-hidden="true" />
                    )}
                    {bound.archived ? "Restore" : "Archive"}
                  </DropdownMenuItem>
                </DropdownMenuGroup>
              </>
            ) : null}
          </DropdownMenuContent>
        </DropdownMenu>
      ) : null}
      {error ? (
        <p
          id={`${metadataId}-error`}
          className="chat-history-row-error"
          role="alert"
        >
          {error}
        </p>
      ) : null}
    </div>
  );
}
