import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  attachAppshot,
  discardAppshot,
  featureError,
  readAppshot,
} from "./desktop-extras";
import type { AppshotPreview } from "./desktop-extras";
import type { ArtifactReference } from "./types";

export function useAppshots(
  destinationKey: string,
  targetId: string | null,
  onAttach: (parts: ArtifactReference[]) => void,
) {
  const [preview, setPreview] = useState<AppshotPreview | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const current = useRef({ destinationKey, targetId, onAttach });
  current.current = { destinationKey, targetId, onAttach };
  const started = useRef<{ id: string; destination: string } | null>(null);
  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    let closed = false;
    const subscriptions = [
      listen<string>("appshot-started", (event) => {
        if (closed) return;
        if (started.current)
          void discardAppshot(started.current.id).catch(() => {});
        setPreview(null);
        started.current = {
          id: event.payload,
          destination: current.current.destinationKey,
        };
        setError("");
      }),
      listen<string>("appshot-ready", (event) => {
        const request = started.current;
        if (
          request?.id !== event.payload ||
          request.destination !== current.current.destinationKey
        ) {
          void discardAppshot(event.payload).catch(() => {});
          return;
        }
        void readAppshot(event.payload).then(
          (value) => {
            if (
              !closed &&
              started.current === request &&
              request.destination === current.current.destinationKey
            )
              setPreview(value);
            else void discardAppshot(value.captureId).catch(() => {});
          },
          (failure) => {
            if (!closed && started.current === request && request.destination === current.current.destinationKey) setError(featureError(failure));
          },
        );
      }),
      listen<{captureId:string;message:string}>("appshot-error", (event) => {
        const request=started.current;
        if (!closed && request?.id===event.payload.captureId && request.destination===current.current.destinationKey) {setPreview(null); setError(event.payload.message.slice(0, 512));}
      }),
    ];
    return () => {
      closed = true;
      if (started.current)
        void discardAppshot(started.current.id).catch(() => {});
      started.current = null;
      for (const subscription of subscriptions)
        void subscription.then(
          (unlisten) => unlisten(),
          () => {},
        );
    };
  }, []);
  useEffect(() => {
    if (
      started.current !== null &&
      started.current.destination !== destinationKey
    ) {
      const id = started.current.id;
      started.current = null;
      setPreview(null);
      setError("");
      void discardAppshot(id).catch(() => {});
    }
  }, [destinationKey]);
  function discard() {
    if (busy) return;
    if (preview) void discardAppshot(preview.captureId).catch(() => {});
    started.current = null;
    setPreview(null);
    setError("");
  }
  async function attach(includeText: boolean) {
    const request = started.current;
    const snapshot = current.current;
    if (
      preview === null ||
      request === null ||
      preview.captureId !== request.id ||
      snapshot.targetId === null ||
      request.destination !== snapshot.destinationKey ||
      busy
    )
      return;
    setBusy(true);
    setError("");
    try {
      const parts = await attachAppshot(
        preview.captureId,
        snapshot.targetId,
        includeText,
      );
      if (
        current.current.destinationKey === snapshot.destinationKey &&
        started.current === request
      ) {
        current.current.onAttach(parts);
        setPreview(null);
        started.current = null;
      }
    } catch (failure) {
      if (current.current.destinationKey === snapshot.destinationKey && started.current === request)
        setError(featureError(failure));
    } finally {
      setBusy(false);
    }
  }
  return { preview, error, busy, discard, attach };
}
