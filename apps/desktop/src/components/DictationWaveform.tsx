import { useEffect, useRef, useSyncExternalStore } from "react";
import type { DictationController, DictationPhase } from "../dictation";
import { DictationLevelMeter } from "../dictation-level";

const HISTORY_LENGTH = 96;
const SAMPLE_MS = 100;

/** A rolling input-level envelope. Only bounded native loudness bytes reach this view. */
export function DictationWaveform({
  controller,
  phase,
}: {
  controller: DictationController;
  phase: DictationPhase;
}) {
  const input = useSyncExternalStore(
    controller.subscribeInputLevel,
    controller.getInputLevelSnapshot,
    controller.getInputLevelSnapshot,
  );
  const inputRef = useRef(input);
  inputRef.current = input;
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const history = useRef(new Array<number>(HISTORY_LENGTH).fill(0));
  const smoothed = useRef(0);
  const levelMeter = useRef(new DictationLevelMeter());
  const processed = useRef({ receivedAt: 0, level: 0 });

  useEffect(() => {
    const canvas = canvasRef.current;
    const context = canvas?.getContext("2d");
    if (!canvas || !context) return;
    const meter = canvas.parentElement!;
    const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
    let width = 0;
    let height = 40;
    let frame = 0;
    let previous = performance.now();
    let sampled = previous;
    let active = true;
    const paint = (now: number) => {
      if (!active) return;
      const recording = phase === "recording";
      const fresh = now - inputRef.current.receivedAt < 400;
      if (
        recording &&
        fresh &&
        processed.current.receivedAt !== inputRef.current.receivedAt
      ) {
        processed.current = {
          receivedAt: inputRef.current.receivedAt,
          level: levelMeter.current.update(inputRef.current.level),
        };
      }
      const target = recording && fresh ? processed.current.level : 0;
      const elapsed = Math.min(100, now - previous);
      previous = now;
      if (recording) {
        smoothed.current +=
          (target - smoothed.current) * (1 - Math.exp(-elapsed / 70));
        if (now - sampled >= SAMPLE_MS) {
          history.current.push(smoothed.current);
          history.current.shift();
          sampled = now;
        }
      }
      meter.style.setProperty(
        "--mic-strength",
        String(recording ? smoothed.current : 0),
      );
      meter.setAttribute("aria-valuenow", String(Math.round(target * 100)));
      meter.dataset.motion = motion.matches ? "reduced" : "full";
      context.clearRect(0, 0, width, height);
      context.strokeStyle = getComputedStyle(canvas).color;
      context.lineWidth = 3;
      context.lineCap = "round";
      const bars = Math.min(
        HISTORY_LENGTH - 1,
        Math.max(6, Math.floor(width / 7)),
      );
      const step = width / Math.max(1, bars);
      const shift =
        recording && !motion.matches ? (now - sampled) / SAMPLE_MS : 0;
      for (let index = 0; index < bars; index += 1) {
        const value =
          motion.matches && recording
            ? target
            : history.current[HISTORY_LENGTH - bars + index]!;
        const size = 1 + Math.pow(value, 0.8) * (height - 9);
        const x = (index + 0.5 - shift) * step;
        context.globalAlpha = 0.28 + (index / Math.max(1, bars - 1)) * 0.72;
        context.beginPath();
        context.moveTo(x, (height - size) / 2);
        context.lineTo(x, (height + size) / 2);
        context.stroke();
      }
      context.globalAlpha = 1;
      if (recording) frame = requestAnimationFrame(paint);
    };
    const resize = () => {
      const box = canvas.getBoundingClientRect();
      width = box.width;
      height = box.height;
      const ratio = Math.min(2, window.devicePixelRatio || 1);
      canvas.width = Math.round(width * ratio);
      canvas.height = Math.round(height * ratio);
      context.setTransform(ratio, 0, 0, ratio, 0, 0);
      if (phase !== "recording") paint(performance.now());
    };
    const observer = new ResizeObserver(resize);
    observer.observe(canvas);
    const redraw = () => {
      if (phase !== "recording") paint(performance.now());
    };
    const theme = new MutationObserver(redraw);
    theme.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });
    motion.addEventListener("change", redraw);
    resize();
    frame = requestAnimationFrame(paint);
    return () => {
      active = false;
      cancelAnimationFrame(frame);
      observer.disconnect();
      theme.disconnect();
      motion.removeEventListener("change", redraw);
    };
  }, [phase]);

  return (
    <div
      className="dictation-waveform"
      role="meter"
      aria-label="Microphone input level"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={0}
    >
      <canvas ref={canvasRef} aria-hidden="true" />
    </div>
  );
}
