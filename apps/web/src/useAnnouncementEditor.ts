import { useEffect, useRef, useState } from "react";
import { ApiFailure, request } from "./api";
import { errorMessage } from "./resources";
import {
  announcementPath,
  announcementWriteMatches,
  saveAnnouncement,
  type AnnouncementWrite,
} from "./announcement-mutation";
import {
  announcementContent,
  announcementValidation,
  type Announcement,
  type AnnouncementAction,
  type AnnouncementContent,
} from "./announcements";

const emptyContent = (): AnnouncementContent => ({
  title: "",
  summary: "",
  body: "",
  tone: "info",
  banner: true,
  priority: 50,
  expires_at: null,
});

/** Capture one reviewed write; uncertain outcomes retain its exact ID and body. */
export function useAnnouncementEditor(
  initial: Announcement | null,
  onSaved: (value: Announcement) => void,
) {
  const [id] = useState(() => initial?.id ?? crypto.randomUUID());
  const [content, setContent] = useState<AnnouncementContent>(() =>
    initial ? announcementContent(initial) : emptyContent(),
  );
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [uncertain, setUncertain] = useState<AnnouncementWrite | null>(null);
  const [conflict, setConflict] = useState(false);
  const operation = useRef<AbortController | null>(null);
  const alive = useRef(true);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      operation.current?.abort();
    };
  }, []);
  async function perform(write: AnnouncementWrite) {
    if (operation.current) return;
    const abort = new AbortController();
    operation.current = abort;
    setBusy(true);
    setError("");
    try {
      const saved = await saveAnnouncement(write, abort.signal);
      if (alive.current && !abort.signal.aborted) onSaved(saved);
    } catch (cause) {
      if (alive.current && !abort.signal.aborted) {
        if (!(cause instanceof ApiFailure) || cause.status >= 500) {
          setUncertain(write);
          setError(
            "The save could not be confirmed. Check its status before making another change.",
          );
        } else {
          setUncertain(null);
          setConflict(cause.status === 409);
          setError(
            cause.status === 409
              ? "Another administrator changed this announcement. Reload its current version before saving."
              : errorMessage(cause),
          );
        }
      }
    } finally {
      if (operation.current === abort) operation.current = null;
      if (alive.current) setBusy(false);
    }
  }
  function save(action: AnnouncementAction) {
    if (uncertain || conflict || operation.current) return;
    const validation = announcementValidation(content, action === "publish");
    if (validation) {
      setError(validation);
      return;
    }
    const snapshot: AnnouncementWrite = {
      id,
      action,
      content: structuredClone(announcementContent(content)),
      ...(initial ? { revision: initial.revision } : {}),
    };
    void perform(snapshot);
  }
  async function checkStatus() {
    if (!uncertain || operation.current) return;
    const abort = new AbortController();
    operation.current = abort;
    setBusy(true);
    try {
      const result = await request<{ announcement: Announcement }>(
        announcementPath(id),
        undefined,
        abort.signal,
      );
      if (!alive.current || abort.signal.aborted) return;
      if (announcementWriteMatches(uncertain, result.announcement))
        onSaved(result.announcement);
      else
        setError(
          "The saved version differs from this proposal. Reload the current version to review it, or retry this exact save.",
        );
    } catch (cause) {
      if (alive.current && !abort.signal.aborted)
        setError(
          cause instanceof ApiFailure && cause.status === 404
            ? "This announcement is not available yet. Check again or retry this exact save."
            : "The save status could not be checked. Your proposal is still retained here.",
        );
    } finally {
      if (operation.current === abort) operation.current = null;
      if (alive.current) setBusy(false);
    }
  }
  return {
    id,
    content,
    setContent,
    error,
    busy,
    uncertain,
    conflict,
    locked: busy || uncertain !== null || conflict,
    save,
    checkStatus,
    retry: () => {
      if (uncertain) void perform(uncertain);
    },
  };
}
