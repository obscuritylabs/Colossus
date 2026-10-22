import { useState } from "react";
import {
  AnnouncementBanner,
  AnnouncementCenter,
  type AnnouncementItem,
} from "@colossus/ui/components/Announcements";
import {
  activeBanner,
  mergeAnnouncementEntries,
  type AnnouncementEntry,
} from "./announcements";
import { useAnnouncements } from "./useAnnouncements";
import { WebLink } from "./WebLink";

function presentation(entry: AnnouncementEntry): AnnouncementItem {
  return {
    id: entry.announcement.id,
    publicationRevision: entry.announcement.publication_revision,
    title: entry.announcement.title,
    summary: entry.announcement.summary,
    body: entry.announcement.body,
    tone: entry.announcement.tone,
    publishedAt:
      entry.announcement.published_at ?? entry.announcement.created_at,
    expiresAt: entry.announcement.expires_at,
    read: entry.read,
    dismissed: entry.dismissed,
  };
}

export function useInstanceAnnouncements(identity: string) {
  const feed = useAnnouncements(identity);
  const [open, setOpen] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const entries = mergeAnnouncementEntries([
    ...(feed.active_banner ? [feed.active_banner] : []),
    ...feed.announcements,
  ]).filter(
    ({ announcement }) =>
      announcement.status === "published" &&
      (announcement.expires_at === null ||
        Date.parse(announcement.expires_at) > Date.now()),
  );
  const banner = activeBanner(feed.active_banner ? [feed.active_banner] : []);
  function read(entry: AnnouncementEntry) {
    setSelectedId(entry.announcement.id);
    setOpen(true);
    void feed.receipt(entry);
  }
  return {
    refresh: feed.refresh,
    notice: (
      <>
        {banner ? (
          <AnnouncementBanner
            announcement={presentation(banner)}
            busy={feed.busyIds.has(banner.announcement.id)}
            onRead={() => read(banner)}
            onDismiss={() => void feed.receipt(banner, true)}
          />
        ) : null}
        {feed.receiptError && !open ? (
          <p className="announcement-inline-error" role="alert">
            {feed.receiptError}
          </p>
        ) : null}
      </>
    ),
    control: (
      <AnnouncementCenter
        announcements={entries.map(presentation)}
        open={open}
        onOpenChange={setOpen}
        selectedId={selectedId}
        onBack={() => setSelectedId(null)}
        onSelect={(item) => {
          const entry = entries.find((row) => row.announcement.id === item.id);
          if (entry) read(entry);
        }}
        onDismiss={(item) => {
          const entry = entries.find((row) => row.announcement.id === item.id);
          if (entry) void feed.receipt(entry, true);
        }}
        busyIds={feed.busyIds}
        loading={feed.loading}
        error={feed.error}
        onRetry={feed.refresh}
        hasMore={feed.hasMore}
        onLoadMore={feed.loadMore}
        loadingMore={feed.loadingMore}
        linkComponent={WebLink}
      />
    ),
  };
}
