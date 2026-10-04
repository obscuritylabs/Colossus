import {
  IconLoader2,
  IconMicrophone,
  IconPlayerPause,
} from "@tabler/icons-react";
import { useEffect, useRef, useState } from "react";
import type { ReactNode } from "react";
import type { DictationController, DictationSnapshot } from "../dictation";
import { DictationWaveform } from "./DictationWaveform";

export function DictationRecordingBar({
  controller,
  state,
  children,
}: {
  controller: DictationController;
  state: DictationSnapshot;
  children: ReactNode;
}) {
  const [seconds, setSeconds] = useState(0);
  const elapsed = useRef(0);
  useEffect(() => {
    if (state.phase !== "recording") return;
    const started = performance.now();
    const timer = setInterval(() => {
      setSeconds(
        Math.floor((elapsed.current + performance.now() - started) / 1000),
      );
    }, 1000);
    return () => {
      elapsed.current += performance.now() - started;
      clearInterval(timer);
    };
  }, [state.phase]);
  const recording = state.phase === "recording";
  const paused = state.phase === "paused";
  const label = recording ? "Listening" : paused ? "Paused" : "Starting mic";
  const Icon = recording
    ? IconMicrophone
    : paused
      ? IconPlayerPause
      : IconLoader2;
  const clock = `${Math.floor(seconds / 60)
    .toString()
    .padStart(2, "0")}:${(seconds % 60).toString().padStart(2, "0")}`;
  return (
    <div className={`dictation-recording is-${state.phase}`}>
      <div
        className="dictation-recording-strip"
        role="group"
        aria-label="Dictation recording"
      >
        <div className="dictation-recording-label">
          <Icon
            size={18}
            stroke={1.7}
            aria-hidden="true"
            className={
              state.phase === "starting" ? "dictation-loading" : undefined
            }
          />
          <div>
            <strong role="status">{label}</strong>
            <span
              className="dictation-recording-clock"
              aria-label={`${seconds} seconds recorded`}
            >
              {clock}
              <span aria-hidden="true"> · </span>On device
            </span>
          </div>
        </div>
        <DictationWaveform controller={controller} phase={state.phase} />
        {children}
      </div>
      <p className="dictation-recording-hint">
        {state.busy
          ? "Updating recording…"
          : state.phase === "starting"
            ? "Loading the local speech model…"
            : recording
              ? state.sending
                ? "Sending this message · Keep speaking for the next draft"
                : "Recording locally · Pause to edit · Send keeps the microphone on"
              : "Dictation paused · You can edit the draft"}
      </p>
    </div>
  );
}
