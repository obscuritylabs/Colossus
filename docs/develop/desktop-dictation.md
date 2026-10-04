---
title: Use and evaluate offline dictation
description: Configure Desktop and TUI dictation, bundle reviewed models, and evaluate native accuracy and latency.
audience: developer
type: how-to
---

# Use and evaluate offline dictation

Desktop includes local Whisper dictation. Open **Settings → Global → Dictation**, enable
it, and choose the microphone, speech model, and spoken punctuation preference. The
composer microphone starts recording; Pause permits edits; Stop releases capture.
The Settings microphone test discards its text when you leave. Recording requires an
explicit action and OS microphone permission. Audio stays on the machine and is
discarded after transcription. Recognized text remains a draft until Send.

Tiny English (~78 MB) ships with Desktop for offline first use. Base English (~148 MB)
is optional: download it from Settings or choose a verified local model file. Transfers
use fixed URLs, sizes, and SHA-256 checks; partial files are never published as models.
An unplugged selected microphone produces an error instead of switching inputs.

Preferences and models live in `COLOSSUS_HOME/dictation` (normally `~/.colossus/dictation`).
Desktop and the local TUI share these device-local choices. They apply to the next
recording, independently of workspaces and external agent targets. Model paths and
audio never enter the renderer or agent protocol. The shared native engine lives in
`crates/colossus-native-dictation`.

## Dictate in the local TUI

The Desktop-bundled CLI includes dictation. Enable it explicitly for source builds:

```bash
GGML_NATIVE=OFF cargo build --locked -p colossus-cli --features dictation
./target/debug/colossus tui
```

Use `/dictate on`, then **F4** to start, pause, or resume. **Shift+F4** stops. **Enter**
finalizes pending speech before sending; multiline mode uses **Ctrl+D**. Active speech
updates cannot overwrite manual edits. Pause before editing. Escape or Ctrl+C cancels
recording; exit, session navigation, and operator decisions release the microphone.

Use `/dictate settings` for current choices and command help:

- `/dictate microphones` lists inputs; `/dictate microphone NUMBER` selects one.
- `/dictate microphone default` restores the OS default.
- `/dictate install tiny` or `/dictate install base` explicitly downloads a pinned model.
- `/dictate model tiny` or `/dictate model base` selects an installed model.
- `/dictate punctuation on` or `/dictate punctuation off` controls spoken commands.
- `/dictate off` disables future recording.

Headless CLI builds omit native audio dependencies. A CLI running over SSH captures
on the machine running the CLI. Microphone access is not forwarded through the worker
or agent connection. Native permissions, input devices, and latency still need live
acceptance on each supported platform.

## Package the included model

Desktop dev/build and macOS/Windows packaging stage Tiny English before native
assembly. `release/dictation/models.json` pins the immutable revision, size, and digest;
`release/dictation/LICENSE-MIT` retains the upstream license. Weights are ignored build
assets, never Git source or renderer assets. Stage a reviewed file without networking:

```bash
node scripts/stage-dictation-model.mjs --model-file /absolute/path/ggml-tiny.en.bin
```

Downloaded and cached bytes are verified. Native resources include the model, license,
and provenance. Cargo defaults to `GGML_NATIVE=OFF` for CPU portability; an explicit
environment value can override it for local profiling. macOS signing grants audio
input to the app and bundled CLI. Renderer
chunk and fixture checks remain active, with a 4,050,000-byte total budget including the
regular dictation controls and lazy Settings page.

## Build and install a candidate

Use the [source toolchain](setup-testing.md), a C++ compiler, CMake, and libclang.
Windows needs the MSVC C++ build tools; macOS needs Xcode command-line tools. Linux
also needs ALSA development headers for the capture adapter. Dependencies come from
the locked root Cargo graph. Build a portable CPU baseline from the repository root:

```bash
GGML_NATIVE=OFF cargo build --locked --release \
  --package colossus-native-dictation --features probe --bin dictation-probe
```

On PowerShell, set `$env:GGML_NATIVE = 'OFF'` before the same Cargo command.
On macOS, evaluate `--features metal` separately after the CPU baseline. The
`metal_requested` output identifies the requested backend; confirm actual GPU use
with platform profiling. It does not establish accelerator availability.

The candidate is `whisper-rs` 0.16.0 / `whisper-rs-sys` 0.15.0, containing
whisper.cpp 1.8.3. The bindings are Unlicense, whisper.cpp is MIT, CPAL 0.18.2 is
Apache-2.0, and Rubato 0.16.2 is MIT. The
[converted OpenAI Whisper weights](https://huggingface.co/ggerganov/whisper.cpp/blob/5359861c739e955e79d9a303bcbc70fb988958b1/README.md)
declare MIT. These GGML conversions are third-party assets; retain both their
conversion provenance and the original Whisper license when distributing them.

Download a model explicitly from the pinned conversion revision
`5359861c739e955e79d9a303bcbc70fb988958b1`. The probe contains no downloader, network
client, API credential handling, automatic updates, or cloud fallback.

| Candidate | File | Bytes | SHA-256 from the pinned LFS metadata |
| --- | --- | --- | --- |
| English tiny | `ggml-tiny.en.bin` | 77,704,715 | `921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f` |
| English base | `ggml-base.en.bin` | 147,964,211 | `a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002` |

Choose an ordinary local file outside the checkout. The probe rejects missing,
empty, symlinked, non-regular, oversized, and hash-mismatched models. It loads the
exact verified bytes, without reopening the pathname. An operator-supplied hash
checks integrity; authenticity depends on reviewing the pinned source above. Hashing
an arbitrary downloaded file and supplying that result does not establish provenance.
The probe creates no model store. Remove its model by deleting that exact installed
file. Desktop and TUI use the reviewed catalog in `release/dictation/models.json`:
Tiny English is bundled with Desktop, and Base English is an explicit download.

## Open Desktop with a microphone control

From the repository root on macOS or Linux:

```bash
GGML_NATIVE=OFF ./scripts/desktop-dev
```

On Windows, set `$env:GGML_NATIVE = 'OFF'`, run `npm ci --ignore-scripts` in
`apps/desktop`, then `npm run tauri:dev`. The historical `--dictation` and
`tauri:dev:dictation` commands remain aliases. Building the probe alone does not launch
Desktop. Close another Desktop instance before launching the development app.

Enable dictation in **Settings → Global → Dictation**, use **Test dictation** to check
your input, then return to the thread and click the composer microphone.

**Spoken punctuation** is enabled by default in Dictation Settings. Say
“period” or “full stop” for `.`, “comma” for `,`, “question mark” for `?`,
“exclamation mark” or “exclamation point” for `!`, “colon” for `:`, “semicolon”
for `;`, and “new line” or “new paragraph” for line breaks. For example,
“Check the build period Is it ready question mark” becomes
“Check the build. Is it ready?”. The formatter works on recognized speech only;
typed text and pasted content keep their wording.

This mode treats punctuation names as commands, including in ordinary phrases.
Say “a literal period of time” to keep “a period of time”, or turn **Spoken
punctuation** off before starting recording to retain all punctuation names as
words. The setting stays fixed during a recording session. Multiword commands
can span transcript segments; editing a settled draft discards that suffix when
its text or position changes. Recognition mistakes can still require Pause and
an edit. This is formatting after local transcription, so it cannot recover a
spoken command that the model did not recognize.

The microphone control pauses and resumes recording. Pause finalizes captured speech
and releases the input device before the draft becomes editable; the composer protects
an active partial from competing edits. Resume continues with the edited draft. The
adjacent stop control finalizes speech and closes the recording session. A clear
recording strip remains visible while capture is active. Its cyan waveform shows
recent microphone loudness, with a glow that follows the input level and a clock
counting active recording time. Silence settles into dots; Pause freezes and dims
the waveform and stops the clock. Starting shows a loading indicator. Reduced
motion keeps a stationary input meter and disables decorative animation. The display
estimates a quiet background from recent input levels and suppresses activity near
that level, so steady room noise usually settles into dots after a short warmup.
This affects only the visualization; it does not gate recording or recognition.
Loud or changing background noise can still move the meter.

The native callback coalesces RMS loudness into one byte from 0 to 255. The existing
session poll delivers at most one level update alongside its bounded transcript
events, so metering stays responsive while inference is running without queuing
telemetry. The renderer retains 96 loudness values for the display, independently
of composer updates. Samples and model data stay in native code. The level history
is not a recording or a frequency spectrum; it measures input strength, including
background noise. Stale levels decay visually, and Pause, Stop, navigation, or a
failure clears the input meter. Stopping during model initialization cancels it.
Native transcript updates remove Whisper's exact `[BLANK_AUDIO]` silence marker,
including empty final revisions that replace an earlier partial.

**Send** settles a FIFO audio boundary before using the existing prompt/run or Next up
path. The same microphone stream stays open. Audio queued after the boundary goes into
the next draft, including speech recognized while the previous message is submitting.
A failed submission keeps the previous draft and appends subsequent speech for review.
Native replies are serialized, and thread/workspace navigation cancels recording and
ignores late updates from the old session. Opening another Desktop surface also stops
capture, as does switching to Topology or Activity where the composer is hidden.
Speech never submits a turn without an explicit Send or Redirect action.

Check pause/edit/resume, successive sends, failed submission, permission denial and
recovery, device removal, model failure, renderer reload, and application close on each
native platform. If capture is unavailable, check the system's default input device and
microphone privacy settings for Colossus or the launching terminal. The app includes
a macOS microphone usage description and an audio-input entitlement configuration;
actual platform permission prompts and hardened packaging still need acceptance.

## Exercise offline inference and capture

The release executable is under `target/release/` unless
`CARGO_TARGET_DIR` overrides it. Windows adds `.exe`. With the model installed,
disconnect networking and run:

```bash
target/release/dictation-probe \
  /path/to/ggml-tiny.en.bin \
  921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f
```

Model verification and initialization happen before microphone access. Capture starts
only for this explicit live invocation. Permission/device failure produces a
categorical error with OS-settings recovery guidance. This console probe does not yet
distinguish every native permission state or provide the production consent UI.

Speak continuously, then enter these commands on stdin:

- `pause` releases the microphone, drains captured audio, and finalizes the tail.
- `resume` opens a new stream while preserving monotonic segment IDs.
- `flush` settles audio already consumed by the pipeline while capture continues.
- `stop` releases capture, finalizes the tail, and terminates the local helper.
  EOF also stops; Ctrl+C terminates the console process.

`flush` is a probe segmentation control, not an agent submission. Audio still queued
at that instant belongs to subsequent updates. Desktop instead inserts a
FIFO capture marker and settles the current draft before using the existing Send path.

Stdout contains JSON cold-start and per-revision metrics. It omits transcript text by
default; add `--show-text` only when deliberately inspecting recognition. Do not
redirect private transcripts into durable logs. Inference time excludes capture,
queue wait, and endpoint delay; measure total visible latency separately.

For reproducible inference without a microphone, add `--wav /path/to/fixture.wav`.
The fixture must already be a mono, 16 kHz, signed 16-bit WAV of at most 60 seconds.
The probe reads it in bounded chunks and creates no recording or temporary audio file.
Use the public
[JFK sample at whisper.cpp v1.8.3](https://github.com/ggml-org/whisper.cpp/blob/v1.8.3/samples/jfk.wav)
for a credential-free smoke check; its SHA-256 is
`59dfb9a4acb36fe2a2affc14bacbee2920ff435cb13cc314a08c13f66ba7860e`.

## Interpret the native boundary

Capture downmixes supported input formats and uses an FFT resampler to produce
16 kHz mono audio. Common rates from 8 to 96 kHz and at most eight channels are
accepted. Each callback is limited to 8,192 mono frames, the capture queue to 256
blocks (at most 8 MiB), and the active segment to six seconds. Partials are recomputed
at two and four seconds; the six-second update finalizes that segment. Pause/stop
also settle short tails, with padding excluded from the reported audio duration.

Each update replaces the complete text for one `segment_id` and increasing
`revision`. Append a segment to a settled draft exactly once when `is_final` is true;
an empty final replaces an earlier partial. Text is bounded to 8 KiB per segment.
The probe retains no cumulative transcript. Its owned audio buffers are zeroized
on release, with no durable recording or transcript logging by default. The native
inference library may retain internal process memory until its helper exits.

The helper is another invocation of the same probe executable, with bounded private
stdin/stdout framing. Audio and model bytes stay within native code. The renderer is
not involved. The parent kills and reaps the helper on normal shutdown or failure,
with a 30-second initialization timeout and a ten-second response timeout per decode.
This avoids the unsafe closure ownership in whisper-rs 0.16.0's cancellation adapter.
Capture overload, malformed input, oversized output, and inference failures stop the
session rather than silently discarding speech or changing to a hosted service.
Desktop window close, renderer reload, and normal application exit independently
cancel a decode; the supervisor checks cancellation every 20 ms. Forced parent
termination still requires platform process-lifetime containment before production
enablement.

## Recorded Linux smoke evidence

On 2026-10-03, the portable CPU release probe transcribed the public 11-second JFK
fixture in a container with `--network none`, using the pinned tiny English model
and no API key. It emitted six revisions and two settled segments, with transcript
text absent from default output. Initialization took 503 ms; individual decodes
took 1,404–2,149 ms on a five-vCPU Intel Xeon Platinum 8573C cloud host while other
repository validation was running. A separate less-contended replay took
1,020–1,469 ms per decode. Process RSS was unavailable in this sandbox, so no memory
claim is made.

This establishes offline execution and the transcript protocol, not microphone
acceptance or supported-device performance. Both runs exceed the provisional
one-second decode budget below. macOS and Windows capture, accuracy, memory,
accelerator behavior, packaging, and long-session stability remain unmeasured.

## Collect supported-device evidence

Record OS/version, CPU, RAM, microphone and sample rate, requested/observed backend,
model digest, executable size, installed model size, cold-start time, p50/p95 decode
time, visible partial/final latency, parent-plus-helper peak RSS, CPU/GPU use, and
technical-term error rate. Include quiet speech, accents, silence/background noise,
words spanning segment boundaries, a 30-minute session, and at least ten successive
draft boundaries. Fixed non-overlapping windows can cut words; byte continuity tests
do not prove recognition accuracy or word-level deduplication.

Use these provisional review thresholds, then explicitly agree the supported hardware
floor: cold start at most five seconds, p95 decode at most one second for six seconds
of audio, first visible partial at most three seconds, combined peak RSS at most
512 MiB, no capture overruns or growing memory during 30 minutes, and at most 10%
word error on a reviewed technical dictation corpus. The fixed-window probe does not
establish an utterance-end finalization target; production endpointing needs its own
latency and accuracy evidence.

On both platforms, verify first-use consent, denial and recovery, absent/unplugged
devices, invalid models, inference timeout/crash, pause/resume, stopping during speech,
normal exit, and forced exit. Confirm the OS microphone indicator clears and repeat
with networking disconnected and no OpenAI API key. Synthetic queue tests cannot
substitute for native permission and shutdown acceptance.

Run deterministic checks with:

```bash
cargo test --locked --package colossus-native-dictation --lib
cargo test --locked --package colossus-native-dictation --features probe --lib
cargo clippy --locked --package colossus-native-dictation --features probe --all-targets -- -D warnings
```

The root Rust gate runs the first suite without audio/model build dependencies.
The feature-enabled suite additionally tests capture bounds, resampler tails, continuous
send boundaries, and cancellation. Renderer tests cover ordered revisions, settled
drafts, failed sends, bounded output, and stale-session replies. Production enablement
remains blocked on macOS/Windows measurements, native permission acceptance, forced-exit
containment, model delivery, silence/endpoint behavior, and release packaging. No
supported-device or packaging decision has been established by a Linux replay alone.
