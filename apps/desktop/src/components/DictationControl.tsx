import {
  IconMicrophone,
  IconPlayerPause,
  IconPlayerStop,
  IconSettings,
} from "@tabler/icons-react";
import type { DictationController, DictationSnapshot } from "../dictation";
import "./dictation.css";

export function DictationControl({
  controller,
  state,
  disabled,
  onSettings,
}: {
  controller: DictationController;
  state: DictationSnapshot;
  disabled: boolean;
  onSettings?: (() => void) | undefined;
}) {
  const recording = state.phase === "recording";
  const paused = state.phase === "paused";
  const active = state.sessionId !== null || state.phase === "starting";
  const label = recording
    ? "Pause dictation"
    : paused
      ? "Resume dictation"
      : "Start dictation";
  return (
    <div className="dictation-control">
      <button
        type="button"
        className={`icon-button${recording ? " is-recording" : ""}`}
        aria-label={label}
        title={label}
        aria-pressed={recording}
        disabled={disabled || state.busy || state.sending}
        onClick={() => {
          if (recording) void controller.control("pause");
          else if (paused) void controller.control("resume");
          else
            void (async () => {
              if (!(await controller.inspect())) return;
              const current = controller.getSnapshot();
              if (current.enabled && current.model) await controller.start();
              else onSettings?.();
            })();
        }}
      >
        {recording ? (
          <IconPlayerPause size={19} aria-hidden="true" />
        ) : (
          <IconMicrophone size={19} stroke={1.7} aria-hidden="true" />
        )}
      </button>
      {active ? (
        <button
          type="button"
          className="icon-button"
          aria-label="Stop dictation"
          title="Stop dictation and release the microphone"
          disabled={state.busy && state.phase !== "starting"}
          onClick={() => {
            if (state.phase === "starting") controller.reset();
            else void controller.control("stop");
          }}
        >
          <IconPlayerStop size={17} aria-hidden="true" />
        </button>
      ) : onSettings ? (
        <button
          type="button"
          className="icon-button"
          aria-label="Dictation settings"
          title="Dictation settings"
          disabled={state.busy}
          onClick={onSettings}
        >
          <IconSettings size={17} aria-hidden="true" />
        </button>
      ) : null}
    </div>
  );
}
