import { IconClock } from "@tabler/icons-react";
import { useCallback, useEffect, useId, useRef, useState } from "react";
import { Button } from "@colossus/ui/components/ui/button";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@colossus/ui/components/ui/popover";
import "@colossus/ui/styles/model-picker.css";
import {
  alwaysOnStatus,
  CommandFailure,
  enableAlwaysOn,
  pauseAlwaysOn,
  wakeAlwaysOn,
} from "../api";
import { shortDateLabel } from "../presenters";
import type { AlwaysOnProfile, AlwaysOnStatus } from "../types";
import "./always-on-control.css";

const STATUS_REFRESH_MS = 5_000;

export function AlwaysOnControl({
  targetId,
  sessionId,
  canEnroll,
  isCurrent,
  onOpenConversation,
}: {
  targetId: string;
  sessionId: string | null;
  canEnroll: boolean;
  isCurrent: () => boolean;
  onOpenConversation: (sessionId: string) => Promise<boolean>;
}) {
  const headingId = useId();
  const [open, setOpen] = useState(false);
  const [status, setStatus] = useState<AlwaysOnStatus | null>(null);
  const [readError, setReadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const read = useRef<symbol | null>(null);
  const action = useRef<symbol | null>(null);

  const refresh = useCallback(async () => {
    if (!isCurrent() || read.current !== null || action.current !== null) {
      return;
    }
    const request = Symbol();
    read.current = request;
    try {
      const next = await alwaysOnStatus(targetId);
      if (read.current === request && isCurrent()) {
        setStatus(next);
        setReadError(null);
      }
    } catch (error: unknown) {
      if (read.current === request && isCurrent()) {
        setStatus(null);
        setReadError(
          error instanceof CommandFailure
            ? error.detail.message
            : "Always On status is unavailable.",
        );
      }
    } finally {
      if (read.current === request) read.current = null;
    }
  }, [isCurrent, targetId]);

  useEffect(() => {
    if (!open) return;
    void refresh();
    const timer = window.setInterval(() => void refresh(), STATUS_REFRESH_MS);
    return () => {
      window.clearInterval(timer);
      read.current = null;
    };
  }, [open, refresh]);

  useEffect(
    () => () => {
      action.current = null;
    },
    [],
  );

  const profile = status?.profile ?? null;
  const linkedHere = profile?.session_id === sessionId;
  const enrollmentAction =
    profile === null
      ? "Use this conversation"
      : !profile.enabled && linkedHere
        ? "Resume"
        : !profile.enabled && profile.pending === null && canEnroll
          ? "Move to this conversation"
          : null;

  async function change(
    mutate: (
      selectionEpoch: number,
      revision: number,
    ) => Promise<AlwaysOnProfile>,
  ) {
    if (status === null || action.current !== null || !isCurrent()) return;
    const request = Symbol();
    action.current = request;
    read.current = null;
    setBusy(true);
    setActionError(null);
    const { selectionEpoch } = status;
    const revision = profile?.revision ?? 0;
    try {
      const next = await mutate(selectionEpoch, revision);
      if (action.current === request && isCurrent()) {
        setStatus({ selectionEpoch, profile: next });
      }
    } catch (error: unknown) {
      if (action.current === request && isCurrent()) {
        setStatus(null);
        setActionError(
          error instanceof CommandFailure
            ? error.detail.outcomeUnknown
              ? "The request outcome is unknown. Check the current status."
              : error.detail.message
            : "Could not update Always On.",
        );
      }
    } finally {
      if (action.current === request) {
        action.current = null;
        if (isCurrent()) {
          setBusy(false);
          void refresh();
        }
      }
    }
  }

  async function openConversation(linkedSessionId: string) {
    if (action.current !== null || !isCurrent()) return;
    const opened = await onOpenConversation(linkedSessionId);
    if (isCurrent()) {
      if (opened) setOpen(false);
      else setActionError("Conversation unavailable.");
    }
  }

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        if (next && action.current === null) {
          setStatus(null);
          setReadError(null);
          setActionError(null);
        }
        setOpen(next);
      }}
    >
      <PopoverTrigger asChild>
        <Button variant="secondary" aria-label="Always On Agent">
          <IconClock size={16} aria-hidden="true" />
          <span className="compact-action-copy">Always On</span>
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        aria-labelledby={headingId}
        className="always-on-popover"
      >
        <h3 id={headingId}>Always On Agent</h3>
        {status === null ? (
          readError === null ? (
            <p role="status">Checking status…</p>
          ) : (
            <>
              <p role="alert">{readError}</p>
              <Button variant="secondary" onClick={() => void refresh()}>
                Refresh
              </Button>
            </>
          )
        ) : (
          <>
            <p>Check-ins require a running Workspace worker.</p>
            {!profile && !canEnroll ? (
              <p>Finish an Execute conversation first.</p>
            ) : null}
            {profile && (
              <dl className="always-on-details">
                <dt>Status</dt>
                <dd>{profile.enabled ? "Enabled" : "Paused"}</dd>
                <dt>Conversation</dt>
                <dd>
                  {linkedHere ? "This conversation" : "Another conversation"}
                </dd>
                {profile.enabled ? (
                  <>
                    <dt>Next wake</dt>
                    <dd>
                      <time dateTime={profile.next_wake_at}>
                        {shortDateLabel(profile.next_wake_at)}
                      </time>
                    </dd>
                  </>
                ) : null}
                {profile.pending !== null ? (
                  <>
                    <dt>Pending</dt>
                    <dd>
                      {profile.pending.trigger === "manual"
                        ? "Manual wake"
                        : "Scheduled wake"}
                    </dd>
                  </>
                ) : null}
                {profile.last_wake !== null ? (
                  <>
                    <dt>Last wake</dt>
                    <dd>
                      <span className="always-on-wake-status">
                        {profile.last_wake.status.replaceAll("_", " ")}
                      </span>
                      {" · "}
                      <time dateTime={profile.last_wake.completed_at}>
                        {shortDateLabel(profile.last_wake.completed_at)}
                      </time>
                    </dd>
                  </>
                ) : null}
              </dl>
            )}
            {profile?.last_wake?.status === "outcome_unknown" &&
            !profile.enabled ? (
              <p>Review the enrolled conversation before continuing.</p>
            ) : null}
            <div className="always-on-actions">
              {enrollmentAction !== null ? (
                <Button
                  disabled={
                    busy ||
                    (profile === null ? !canEnroll : profile.pending !== null)
                  }
                  onClick={() => {
                    if (sessionId !== null) {
                      void change((epoch, revision) =>
                        enableAlwaysOn(targetId, epoch, sessionId, revision),
                      );
                    }
                  }}
                >
                  {enrollmentAction}
                </Button>
              ) : null}
              {profile?.enabled ? (
                <>
                  <Button
                    disabled={busy || profile.pending !== null}
                    onClick={() =>
                      void change((epoch, revision) =>
                        wakeAlwaysOn(targetId, epoch, revision),
                      )
                    }
                  >
                    Wake now
                  </Button>
                  <Button
                    variant="secondary"
                    disabled={busy}
                    onClick={() =>
                      void change((epoch, revision) =>
                        pauseAlwaysOn(targetId, epoch, revision),
                      )
                    }
                  >
                    Pause
                  </Button>
                </>
              ) : null}
              {profile !== null && !linkedHere ? (
                <Button
                  variant="secondary"
                  disabled={busy}
                  onClick={() => void openConversation(profile.session_id)}
                >
                  Open conversation
                </Button>
              ) : null}
            </div>
          </>
        )}
        {actionError !== null ? <p role="alert">{actionError}</p> : null}
      </PopoverContent>
    </Popover>
  );
}
