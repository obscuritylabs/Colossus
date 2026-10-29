import { useEffect, useRef, useState } from "react";
import { Button } from "./Controls.js";
import {
  localNextPrompt,
  normalizeNextPrompt,
  type NextPromptMode,
  type NextPromptPrediction,
  type NextPromptSource,
} from "../lib/next-prompt.js";

/** Host supplies authenticated transport. Shared UI never chooses a provider. */
export function useNextPrompt({
  source,
  mode,
  draft,
  busy,
  load,
}: {
  source: NextPromptSource | null | undefined;
  mode: NextPromptMode;
  draft: string;
  busy: boolean;
  load?:
    ((signal: AbortSignal) => Promise<NextPromptPrediction | null>) | undefined;
}) {
  const attempted = useRef(
    new Map<
      string,
      {
        controller: AbortController;
        promise: Promise<NextPromptPrediction | null>;
      }
    >(),
  );
  const mountGeneration = useRef(0);
  const mounted = useRef(false);
  const [predictions, setPredictions] = useState(
    new Map<string, { text: string | null; tokens: number | null }>(),
  );
  const [dismissed, dismiss] = useState<string | null>(null);
  useEffect(() => {
    const generation = ++mountGeneration.current;
    mounted.current = true;
    return () => {
      mounted.current = false;
      queueMicrotask(() => {
        if (mountGeneration.current === generation)
          for (const attempt of attempted.current.values())
            attempt.controller.abort();
      });
    };
  }, []);
  useEffect(() => {
    if (!source || mode !== "model" || draft || busy || !load) return;
    const key = source.key;
    let attempt = attempted.current.get(key);
    if (!attempt) {
      const controller = new AbortController();
      attempt = {
        controller,
        promise: load(controller.signal).catch(() => null),
      };
      attempted.current.set(key, attempt);
      if (attempted.current.size > 64)
        attempted.current.delete(attempted.current.keys().next().value!);
    }
    const current = attempt;
    void current.promise.then((result) => {
      if (!mounted.current || current.controller.signal.aborted) return;
      setPredictions((previous) => {
        const text = result?.text ? normalizeNextPrompt(result.text) : null;
        const tokens = result?.usage?.total_tokens ?? null;
        if (
          previous.get(key)?.text === text &&
          previous.get(key)?.tokens === tokens
        )
          return previous;
        const next = new Map(previous);
        next.set(key, { text, tokens });
        if (next.size > 64) next.delete(next.keys().next().value!);
        return next;
      });
    });
    // A dispatched generation may already be billed. Keep observing its bounded
    // receipt through edits and selection changes; only text eligibility changes.
  }, [source?.key, mode, draft, busy, load]);
  const model = source ? predictions.get(source.key) : null;
  const text =
    !source || mode === "off" || draft || busy || dismissed === source.key
      ? null
      : ((mode === "model" ? model?.text : null) ??
        localNextPrompt(source.output));
  return {
    text,
    auxiliaryTokens: model?.tokens ?? null,
    dismiss: () => dismiss(source?.key ?? null),
  };
}

/** Accept fills the host draft only. Sending always remains a separate action. */
export function NextPromptSuggestion({
  text,
  onAccept,
  onDismiss,
  auxiliaryTokens,
}: {
  text: string | null;
  onAccept: () => void;
  onDismiss: () => void;
  auxiliaryTokens?: number | null | undefined;
}) {
  return (
    <div
      className="shared-next-prompt"
      aria-label={text ? "Suggested next prompt" : "Prompt prediction usage"}
    >
      {text ? (
        <Button
          variant="tertiary"
          className="shared-next-prompt-accept"
          onClick={onAccept}
          title="Insert into your draft; Tab also accepts while the empty editor is focused"
        >
          <span className="shared-next-prompt-label">Next</span>
          <span>{text}</span>
          <kbd>Tab</kbd>
        </Button>
      ) : null}
      {text ? (
        <Button
          variant="tertiary"
          aria-label="Dismiss suggested next prompt"
          onClick={onDismiss}
        >
          ×
        </Button>
      ) : null}
      {auxiliaryTokens != null ? (
        <span
          className="shared-next-prompt-usage"
          title="Provider-reported tokens for this separate prediction request"
        >
          Prediction: {auxiliaryTokens.toLocaleString()} tokens
        </span>
      ) : null}
    </div>
  );
}
