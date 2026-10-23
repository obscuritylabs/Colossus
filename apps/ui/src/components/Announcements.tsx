import { lazy, Suspense, useEffect, useRef } from "react";
import {
  IconArrowLeft,
  IconBell,
  IconCircleCheck,
  IconExclamationCircle,
  IconInfoCircle,
  IconRestore,
  IconSpeakerphone,
  IconX,
} from "@tabler/icons-react";
import type { MarkdownLink } from "./MarkdownContent.js";
import { Button } from "./ui/button.js";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from "./ui/sheet.js";

// Keep Markdown and its optional syntax/diagram renderers out of the bell's
// initial presentation path. Hosts retain navigation and released-content policy.
const AnnouncementBody = lazy(() =>
  import("./MarkdownContent.js").then(({ MarkdownContent }) => ({
    default: MarkdownContent,
  })),
);

export type AnnouncementTone = "info" | "success" | "warning" | "danger";

export interface AnnouncementItem {
  id: string;
  publicationRevision: number;
  title: string;
  summary: string;
  body: string;
  tone: AnnouncementTone;
  publishedAt: string;
  expiresAt?: string | null;
  read: boolean;
  dismissed: boolean;
}

const TONES = {
  info: { icon: IconInfoCircle, label: "Information" },
  success: { icon: IconCircleCheck, label: "Success" },
  warning: { icon: IconExclamationCircle, label: "Warning" },
  danger: { icon: IconExclamationCircle, label: "Important" },
} as const;

function AnnouncementToneIcon({ tone }: { tone: AnnouncementTone }) {
  const { icon: Icon, label } = TONES[tone];
  return (
    <span className="announcement-tone-icon">
      <Icon size={18} aria-hidden="true" />
      <span className="shared-sr-only">{label}: </span>
    </span>
  );
}

function PublicationTime({ value }: { value: string }) {
  const date = new Date(value);
  return Number.isFinite(date.getTime()) ? (
    <time dateTime={value}>{date.toLocaleDateString()}</time>
  ) : (
    <span>Publication time unavailable</span>
  );
}

export interface AnnouncementBannerProps {
  /** The host selects the active, permitted, undismissed announcement. */
  announcement: AnnouncementItem;
  busy?: boolean;
  onRead: () => void;
  onDismiss?: (() => void) | undefined;
}

/** Controlled presentation only: dismissal never writes browser storage. */
export function AnnouncementBanner({
  announcement,
  busy = false,
  onRead,
  onDismiss,
}: AnnouncementBannerProps) {
  return (
    <section
      className="announcement-banner"
      data-tone={announcement.tone}
      aria-label="Instance announcement"
      aria-busy={busy}
    >
      <AnnouncementToneIcon tone={announcement.tone} />
      <div className="announcement-banner-copy">
        <strong>{announcement.title}</strong>
        {announcement.summary ? <span>{announcement.summary}</span> : null}
      </div>
      <div className="announcement-banner-actions">
        <Button variant="link" onClick={onRead} disabled={busy}>
          Read update
        </Button>
        {onDismiss ? (
          <Button
            variant="ghost"
            size="icon"
            onClick={onDismiss}
            disabled={busy}
            aria-label={`Dismiss announcement: ${announcement.title}`}
          >
            <IconX size={16} aria-hidden="true" />
          </Button>
        ) : null}
      </div>
    </section>
  );
}

export interface AnnouncementCenterProps {
  /** Host-filtered records; dismissed announcements remain available to read. */
  announcements: readonly AnnouncementItem[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  selectedId?: string | null;
  /** Selection is explicit. The host decides whether to persist a read receipt. */
  onSelect: (announcement: AnnouncementItem) => void;
  onBack?: (() => void) | undefined;
  onDismiss?: ((announcement: AnnouncementItem) => void) | undefined;
  onRestore?: ((announcement: AnnouncementItem) => void) | undefined;
  busyId?: string | null;
  busyIds?: ReadonlySet<string>;
  loading?: boolean;
  error?: string | null;
  onRetry?: (() => void) | undefined;
  hasMore?: boolean;
  onLoadMore?: (() => void) | undefined;
  loadingMore?: boolean;
  linkComponent?: MarkdownLink | undefined;
}

function AnnouncementActions({
  announcement,
  busy,
  onDismiss,
  onRestore,
}: {
  announcement: AnnouncementItem;
  busy: boolean;
  onDismiss: AnnouncementCenterProps["onDismiss"];
  onRestore: AnnouncementCenterProps["onRestore"];
}) {
  if (announcement.dismissed) {
    return onRestore ? (
      <Button
        variant="ghost"
        onClick={() => onRestore(announcement)}
        disabled={busy}
        aria-label={`Show announcement banner again: ${announcement.title}`}
      >
        <IconRestore size={16} aria-hidden="true" /> Show banner again
      </Button>
    ) : null;
  }
  return onDismiss ? (
    <Button
      variant="ghost"
      onClick={() => onDismiss(announcement)}
      disabled={busy}
      aria-label={`Dismiss announcement: ${announcement.title}`}
    >
      <IconX size={16} aria-hidden="true" /> Dismiss banner
    </Button>
  ) : null;
}

export function AnnouncementCenter({
  announcements,
  open,
  onOpenChange,
  selectedId,
  onSelect,
  onBack,
  onDismiss,
  onRestore,
  busyId,
  busyIds,
  loading = false,
  error,
  onRetry,
  hasMore = false,
  onLoadMore,
  loadingMore = false,
  linkComponent,
}: AnnouncementCenterProps) {
  const unreadCount = announcements.reduce(
    (count, announcement) => count + (announcement.read ? 0 : 1),
    0,
  );
  const unreadLabel = `${hasMore ? "At least " : ""}${unreadCount} unread update${unreadCount === 1 ? "" : "s"}`;
  const selected = announcements.find(({ id }) => id === selectedId);
  const isBusy = (id: string) => busyId === id || busyIds?.has(id) === true;
  const centerHeading = useRef<HTMLHeadingElement>(null);
  const detailHeading = useRef<HTMLHeadingElement>(null);
  const rowButtons = useRef(new Map<string, HTMLButtonElement>());
  const previousSelection = useRef<string | null>(null);
  const currentSelection = selected?.id ?? null;
  useEffect(() => {
    if (!open) return;
    if (currentSelection) {
      detailHeading.current?.focus();
    } else if (previousSelection.current) {
      const row = rowButtons.current.get(previousSelection.current);
      (row ?? centerHeading.current)?.focus();
    }
    previousSelection.current = currentSelection;
  }, [open, currentSelection]);
  return (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className="announcement-bell"
          aria-label={`Announcements, ${unreadLabel}`}
        >
          <IconBell size={18} aria-hidden="true" />
          {unreadCount > 0 ? (
            <span className="announcement-unread-count" aria-hidden="true">
              {unreadCount > 99 ? "99+" : `${unreadCount}${hasMore ? "+" : ""}`}
            </span>
          ) : null}
        </Button>
      </SheetTrigger>
      <SheetContent
        className="announcement-center"
        side="right"
        onOpenAutoFocus={(event) => {
          if (!selected || !detailHeading.current) return;
          event.preventDefault();
          detailHeading.current.focus();
        }}
      >
        <SheetHeader className="announcement-center-header">
          <SheetTitle
            ref={centerHeading}
            tabIndex={-1}
            className="announcement-center-title"
          >
            <IconSpeakerphone size={19} aria-hidden="true" /> Announcements
          </SheetTitle>
          <SheetDescription className="announcement-center-description">
            Updates published by your instance administrators.
          </SheetDescription>
          <p className="announcement-center-count">{unreadLabel}</p>
        </SheetHeader>
        <div className="announcement-center-scroll">
          {error ? (
            <div className="announcement-center-error" role="alert">
              <p>{error}</p>
              {onRetry ? (
                <Button variant="outline" onClick={onRetry} disabled={loading}>
                  Try again
                </Button>
              ) : null}
            </div>
          ) : null}
          {loading ? (
            <p className="announcement-center-status" role="status">
              Loading announcements…
            </p>
          ) : null}
          {selected ? (
            <article
              className="announcement-detail"
              aria-label={selected.title}
              aria-busy={isBusy(selected.id)}
            >
              {onBack ? (
                <Button
                  className="announcement-back"
                  variant="ghost"
                  onClick={onBack}
                >
                  <IconArrowLeft size={16} aria-hidden="true" /> All
                  announcements
                </Button>
              ) : null}
              <div
                className="announcement-detail-heading"
                data-tone={selected.tone}
              >
                <AnnouncementToneIcon tone={selected.tone} />
                <h3 ref={detailHeading} tabIndex={-1}>
                  {selected.title}
                </h3>
              </div>
              <div className="announcement-meta">
                <PublicationTime value={selected.publishedAt} />
                {selected.read ? <span>Read</span> : <span>Unread</span>}
                {selected.dismissed ? <span>Banner dismissed</span> : null}
              </div>
              <Suspense fallback={<p role="status">Loading update…</p>}>
                <AnnouncementBody
                  content={selected.body}
                  linkComponent={linkComponent}
                />
              </Suspense>
              <div className="announcement-detail-actions">
                <AnnouncementActions
                  announcement={selected}
                  busy={isBusy(selected.id)}
                  onDismiss={onDismiss}
                  onRestore={onRestore}
                />
              </div>
            </article>
          ) : null}
          {!selected || !onBack ? (
            <>
              {announcements.length > 0 ? (
                <ul
                  className="announcement-list"
                  aria-label="Published announcements"
                >
                  {announcements.map((announcement) => (
                    <li
                      key={`${announcement.id}:${announcement.publicationRevision}`}
                      className="announcement-list-item"
                      data-tone={announcement.tone}
                      data-unread={!announcement.read}
                      aria-busy={isBusy(announcement.id)}
                    >
                      <button
                        ref={(element) => {
                          if (element)
                            rowButtons.current.set(announcement.id, element);
                          else rowButtons.current.delete(announcement.id);
                        }}
                        type="button"
                        className="announcement-list-select"
                        onClick={() => onSelect(announcement)}
                        disabled={isBusy(announcement.id)}
                        aria-label={`Read announcement: ${announcement.title}${announcement.read ? "" : ", unread"}`}
                      >
                        <AnnouncementToneIcon tone={announcement.tone} />
                        <span className="announcement-list-copy">
                          <strong>{announcement.title}</strong>
                          {announcement.summary ? (
                            <span>{announcement.summary}</span>
                          ) : null}
                          <span className="announcement-meta">
                            <PublicationTime value={announcement.publishedAt} />
                            {!announcement.read ? <span>Unread</span> : null}
                            {announcement.dismissed ? (
                              <span>Banner dismissed</span>
                            ) : null}
                          </span>
                        </span>
                      </button>
                      <div className="announcement-list-actions">
                        <AnnouncementActions
                          announcement={announcement}
                          busy={isBusy(announcement.id)}
                          onDismiss={onDismiss}
                          onRestore={onRestore}
                        />
                      </div>
                    </li>
                  ))}
                </ul>
              ) : !loading && !error ? (
                <p className="announcement-center-status">
                  No announcements yet.
                </p>
              ) : null}
              {hasMore ? (
                <div className="announcement-pagination">
                  {onLoadMore ? (
                    <Button
                      variant="outline"
                      onClick={onLoadMore}
                      disabled={loading || loadingMore}
                    >
                      {loadingMore
                        ? "Loading more…"
                        : "Load older announcements"}
                    </Button>
                  ) : (
                    <p>Showing recent announcements.</p>
                  )}
                </div>
              ) : null}
            </>
          ) : null}
        </div>
      </SheetContent>
    </Sheet>
  );
}
