import {
  IconMicrophone,
  IconPlayerPause,
  IconPlayerStop,
  IconX,
} from "@tabler/icons-react";
import { useState } from "react";
import type { DictationController, DictationSnapshot } from "../dictation";
import "./dictation.css";

export function DictationControl({
  controller,
  state,
  disabled,
}: {
  controller: DictationController;
  state: DictationSnapshot;
  disabled: boolean;
}) {
  const [open, setOpen] = useState(false);
  const recording = state.phase === "recording";
  const paused = state.phase === "paused";
  const active = state.sessionId !== null || state.phase === "starting";
  const label = recording
    ? "Pause dictation"
    : paused
      ? "Resume dictation"
      : "Start offline dictation";
  return (
    <div className="dictation-control">
      <button
        type="button"
        className={`icon-button${recording ? " is-recording" : ""}`}
        aria-label={label}
        title={label}
        aria-pressed={recording}
        aria-expanded={open}
        aria-controls="dictation-settings"
        disabled={disabled || state.busy || state.sending}
        onClick={() => {
          if (recording) void controller.control("pause");
          else if (paused) void controller.control("resume");
          else {
            setOpen(!open);
            if (!open) void controller.inspect();
          }
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
          disabled={state.busy}
          onClick={() => void controller.control("stop")}
        >
          <IconPlayerStop size={17} aria-hidden="true" />
        </button>
      ) : null}
      {open ? (
        <div
          className="dictation-settings"
          id="dictation-settings"
          role="region"
          aria-label="Offline dictation"
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              setOpen(false);
            }
          }}
        >
          <div className="dictation-settings-header">
            <strong>Offline dictation preview</strong>
            <button
              className="icon-button"
              type="button"
              aria-label="Close dictation settings"
              onClick={() => setOpen(false)}
            >
              <IconX size={16} aria-hidden="true" />
            </button>
          </div>
          <p>
            Speech is transcribed on this computer. Audio is not uploaded or
            saved.
          </p>
          {!state.enabled ? (
            <p>
              Dictation is not enabled in this build. Use the dictation preview
              build described in the testing guide.
            </p>
          ) : (
            <>
              <label className="dictation-punctuation-option">
                <input
                  type="checkbox"
                  checked={state.spokenPunctuation}
                  disabled={active || state.busy || state.sending}
                  onChange={(event) =>
                    controller.setSpokenPunctuation(event.target.checked)
                  }
                />
                Spoken punctuation
              </label>
              <p>
                Say “period”, “comma”, or “question mark” to add punctuation.
                Say “literal period” to keep the word. Turn this off before
                recording to keep punctuation names as words.
              </p>
              <p>
                {state.model
                  ? `Local model: ${state.model}`
                  : "Choose the pinned Whisper tiny.en or base.en model you downloaded."}
              </p>
              <div className="dictation-settings-actions">
                <button
                  className="button secondary compact"
                  type="button"
                  disabled={active || state.busy}
                  onClick={() => void controller.choose()}
                >
                  {state.busy ? "Working…" : "Choose model…"}
                </button>
                <button
                  className="button compact"
                  type="button"
                  disabled={!state.model || active || state.busy || disabled}
                  onClick={() => {
                    setOpen(false);
                    void controller.start();
                  }}
                >
                  Start recording
                </button>
              </div>
            </>
          )}
        </div>
      ) : null}
    </div>
  );
}
