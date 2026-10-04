import {
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { IconMicrophone, IconCpu, IconRefresh } from "@tabler/icons-react";
import { DropdownSelect } from "./DropdownSelect";
import { DictationRecordingBar } from "./DictationRecordingBar";
import { DictationControl } from "./DictationControl";
import { DictationController, nativeDictationApi } from "../dictation";
import { composerDraft, expandComposerDraft } from "../composer-paste";
import {
  getDictationSettings,
  saveDictationSettings,
  downloadDictationModel,
  cancelDictationDownload,
  dictationError,
} from "../dictation-settings";
import type {
  DictationSettingsDraft,
  DictationSettingsSnapshot,
  DictationModelId,
} from "../dictation-settings";

export function DictationSettings() {
  const [settings, setSettings] = useState<DictationSettingsSnapshot | null>(
    null,
  );
  const [pending, setPending] = useState(false);
  const [downloading, setDownloading] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const cancelRequested = useRef(false);
  const draft = useRef(composerDraft());
  const [transcript, setTranscript] = useState("");
  const [controller] = useState(
    () =>
      new DictationController(nativeDictationApi, {
        read: () => draft.current,
        write: (next) => {
          draft.current = next;
          setTranscript(expandComposerDraft(next));
        },
      }),
  );
  const recording = useSyncExternalStore(
    controller.subscribe,
    controller.getSnapshot,
    controller.getSnapshot,
  );
  const testing = !!recording.sessionId || recording.phase === "starting";
  const disabled =
    pending ||
    testing ||
    settings?.active === true ||
    settings?.downloadActive === true;
  const transferring = downloading || settings?.downloadActive === true;
  const wasTesting = useRef(false);
  const refresh = useCallback(async () => {
    try {
      setSettings(await getDictationSettings());
      setError("");
    } catch (failure) {
      setError(dictationError(failure));
    }
  }, []);
  useEffect(() => {
    void refresh();
    window.addEventListener("focus", refresh);
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      await controller.poll();
      if (active) timer = setTimeout(poll, 150);
    };
    void poll();
    return () => {
      active = false;
      clearTimeout(timer);
      window.removeEventListener("focus", refresh);
      controller.reset();
    };
  }, [controller, refresh]);
  useEffect(() => {
    if (wasTesting.current && !testing) void refresh();
    wasTesting.current = testing;
  }, [testing, refresh]);
  useEffect(() => {
    if (!settings?.downloadActive) return;
    const timer = setInterval(() => void refresh(), 1500);
    return () => clearInterval(timer);
  }, [settings?.downloadActive, refresh]);

  async function save(change: Partial<DictationSettingsDraft>) {
    if (!settings) return;
    setPending(true);
    setError("");
    setNotice("");
    try {
      setSettings(
        await saveDictationSettings({
          enabled: settings.enabled,
          modelId: settings.modelId,
          microphoneId: settings.microphoneId,
          spokenPunctuation: settings.spokenPunctuation,
          ...change,
        }),
      );
      setNotice("Saved");
    } catch (failure) {
      setError(dictationError(failure));
    } finally {
      setPending(false);
    }
  }
  async function install(modelId: DictationModelId) {
    cancelRequested.current = false;
    setPending(true);
    setDownloading(true);
    setError("");
    setNotice("");
    try {
      setSettings(await downloadDictationModel(modelId));
      setNotice("Model installed");
    } catch (failure) {
      if (cancelRequested.current) setNotice("Download cancelled");
      else setError(dictationError(failure));
    } finally {
      setPending(false);
      setDownloading(false);
    }
  }
  const model = settings?.models.find((item) => item.id === settings.modelId);
  return (
    <section
      className="managed-settings-body desktop-settings dictation-settings-page"
      aria-labelledby="dictation-settings-heading"
    >
      <div className="managed-section-heading">
        <div>
          <h3 id="dictation-settings-heading">Dictation</h3>
          <p className="managed-heading-copy">
            Turn speech into text on this device.
          </p>
        </div>
      </div>
      {error ? (
        <p className="inline-error" role="alert">
          {error}
        </p>
      ) : null}
      {!settings && !error ? (
        <p role="status">Loading dictation settings…</p>
      ) : null}
      {settings ? (
        <>
          <div className="appearance-settings-card desktop-preferences-card">
            <label className="compact-switch desktop-preference-toggle">
              <input
                className="switch-input"
                type="checkbox"
                role="switch"
                checked={settings.enabled}
                disabled={disabled}
                onChange={(event) =>
                  void save({ enabled: event.target.checked })
                }
              />
              <span>
                <strong>Enable dictation</strong>
                <small>
                  Recording starts only when you click the microphone or start a
                  test.
                </small>
              </span>
            </label>
            <p className="desktop-preference-note">
              Audio stays on this device and is discarded after transcription.
              Recognized text goes into your draft; only Send submits it.
            </p>
            <label
              className="desktop-preference-select"
              htmlFor="dictation-input"
            >
              <IconMicrophone size={18} aria-hidden="true" />
              <span>
                <strong>Microphone</strong>
                <small>
                  Choose the input used by Desktop and the local TUI.
                </small>
              </span>
              <DropdownSelect
                id="dictation-input"
                value={settings.microphoneId ?? ""}
                disabled={disabled}
                onChange={(event) =>
                  void save({ microphoneId: event.target.value || null })
                }
              >
                <option value="">System default</option>
                {settings.microphoneMissing ? (
                  <option value={settings.microphoneId!}>
                    Unavailable microphone
                  </option>
                ) : null}
                {settings.microphones.map((input) => (
                  <option key={input.id} value={input.id}>
                    {input.name}
                  </option>
                ))}
              </DropdownSelect>
            </label>
            <button
              className="button secondary compact"
              type="button"
              disabled={disabled}
              onClick={() => void refresh()}
            >
              <IconRefresh size={16} aria-hidden="true" />
              Refresh microphones
            </button>
            {settings.microphoneMissing ? (
              <p role="alert">
                The selected microphone is disconnected. Connect it or choose
                another input.
              </p>
            ) : null}
            <label
              className="desktop-preference-select"
              htmlFor="dictation-model"
            >
              <IconCpu size={18} aria-hidden="true" />
              <span>
                <strong>Speech model</strong>
                <small>
                  Tiny English is fast and included. Base English may improve
                  accuracy but uses more memory and time.
                </small>
              </span>
              <DropdownSelect
                id="dictation-model"
                value={settings.modelId}
                disabled={disabled}
                onChange={(event) =>
                  void save({ modelId: event.target.value as DictationModelId })
                }
              >
                {settings.models.map((item) => (
                  <option
                    key={item.id}
                    value={item.id}
                    disabled={!item.installed}
                  >
                    {item.name}
                    {item.installed ? "" : " · Not installed"}
                  </option>
                ))}
              </DropdownSelect>
            </label>
            <div className="settings-action-row">
              <button
                className="button secondary compact"
                type="button"
                disabled={disabled}
                onClick={() =>
                  void (async () => {
                    setPending(true);
                    try {
                      await controller.choose();
                      await refresh();
                    } finally {
                      setPending(false);
                    }
                  })()
                }
              >
                Choose local model…
              </button>
              {!settings.models.find((item) => item.id === "base_english")
                ?.installed ? (
                <button
                  className="button secondary compact"
                  type="button"
                  disabled={disabled}
                  onClick={() => void install("base_english")}
                >
                  Download Base English · 148 MB
                </button>
              ) : null}
              {transferring ? (
                <button
                  className="button secondary compact"
                  type="button"
                  onClick={() =>
                    void (async () => {
                      cancelRequested.current = true;
                      setNotice("Cancelling download…");
                      try {
                        await cancelDictationDownload();
                      } catch (failure) {
                        setError(dictationError(failure));
                      }
                    })()
                  }
                >
                  Cancel download
                </button>
              ) : null}
            </div>
            <label className="compact-switch desktop-preference-toggle">
              <input
                className="switch-input"
                type="checkbox"
                role="switch"
                checked={settings.spokenPunctuation}
                disabled={disabled}
                onChange={(event) =>
                  void save({ spokenPunctuation: event.target.checked })
                }
              />
              <span>
                <strong>Spoken punctuation</strong>
                <small>
                  Say “period”, “comma”, or “question mark” to add punctuation.
                  Say “literal period” to keep the word.
                </small>
              </span>
            </label>
            <p className="desktop-preference-note">
              These choices are saved on this device and apply to your next
              recording.
            </p>
            <p role="status">
              {transferring
                ? cancelRequested.current
                  ? "Cancelling download…"
                  : "Downloading and verifying the model…"
                : notice}
            </p>
          </div>
          <div className="appearance-settings-card desktop-preferences-card">
            <strong>Try your microphone</strong>
            <p className="desktop-preference-note">
              Speak a short sentence to check the input and transcription. Test
              text is discarded when you leave this page.
            </p>
            {testing ? (
              <DictationRecordingBar controller={controller} state={recording}>
                <DictationControl
                  controller={controller}
                  state={recording}
                  disabled={false}
                />
              </DictationRecordingBar>
            ) : (
              <button
                type="button"
                className="button secondary"
                disabled={
                  disabled ||
                  !settings.enabled ||
                  !model?.installed ||
                  settings.microphoneMissing
                }
                onClick={() =>
                  void (async () => {
                    draft.current = composerDraft();
                    setTranscript("");
                    if (!(await controller.inspect())) return;
                    await controller.start();
                  })()
                }
              >
                Test dictation
              </button>
            )}
            {recording.error ? (
              <p className="inline-error" role="alert">
                {recording.error}
              </p>
            ) : null}
            {transcript ? (
              <p aria-label="Test transcript">{transcript}</p>
            ) : null}
          </div>
        </>
      ) : null}
    </section>
  );
}
