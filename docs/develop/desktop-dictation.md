---
title: Evaluate offline Desktop dictation
description: Run the native Whisper probe and collect the evidence required before enabling Desktop microphone dictation.
audience: developer
type: how-to
---

# Evaluate offline Desktop dictation

`apps/desktop/native-dictation` is an opt-in feasibility probe for
[issue 212](https://github.com/obscuritylabs/Colossus/issues/212). It is not linked
into the production Desktop application. Run it on representative macOS and Windows
devices before selecting a production model, inference backend, or delivery method.
The existing composer and agent protocol are unchanged.

## Build and install a candidate

Use the [source toolchain](setup-testing.md), a C++ compiler, CMake, and libclang.
Windows needs the MSVC C++ build tools; macOS needs Xcode command-line tools. Linux
also needs ALSA development headers for the capture adapter. Dependencies come from
the locked Desktop Cargo graph. Build a portable CPU baseline from the repository root:

```bash
GGML_NATIVE=OFF cargo build --locked --release \
  --manifest-path apps/desktop/src-tauri/Cargo.toml \
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
file. Maintainers must own a reviewed, pinned model manifest and its updates before
adding a production installer; bundling versus explicit download is still undecided.

## Exercise offline inference and capture

The release executable is under `apps/desktop/src-tauri/target/release/` unless
`CARGO_TARGET_DIR` overrides it. Windows adds `.exe`. With the model installed,
disconnect networking and run:

```bash
apps/desktop/src-tauri/target/release/dictation-probe \
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
at that instant belongs to subsequent updates. Production send semantics must bind
the capture boundary to a composer draft before wiring this to `create_run`.

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
Forced parent termination still requires platform process-lifetime containment
before production integration.

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
cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml \
  --package colossus-native-dictation --lib
cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml \
  --package colossus-native-dictation --features probe --lib
cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml \
  --package colossus-native-dictation --features probe --all-targets -- -D warnings
```

The ordinary Desktop gate runs the first suite without audio/model build dependencies.
The feature-enabled suite additionally tests capture bounds and resampler tail handling.
Production enablement remains blocked on macOS/Windows measurements, native permission
states, process containment, model delivery, silence/endpoint behavior, and composer
edit/send integration. No supported-device or packaging decision has been established
by a Linux replay alone.
