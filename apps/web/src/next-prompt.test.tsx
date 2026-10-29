// @vitest-environment happy-dom
import { StrictMode, act, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { localNextPrompt, normalizeNextPrompt, NextPromptSuggestion, useNextPrompt, type NextPromptMode, type NextPromptPrediction } from "@colossus/ui/conversation";

it("extracts bounded explicit actions and excludes code, controls, and slash commands", () => {
  expect(localNextPrompt("Done.\n\n## Next steps\n- [ ] Add coverage for cookie restoration.")).toBe("Add coverage for cookie restoration");
  expect(localNextPrompt("```\nI can delete it.\n```\nWould you like me to review the deployment?")).toBe("Review the deployment");
  expect(localNextPrompt("The deployment passed all checks.")).toBeNull();
  expect(normalizeNextPrompt("/reset")).toBeNull();
  expect(normalizeNextPrompt("  !git status  ")).toBeNull();
  expect(localNextPrompt("I can !git status")).toBeNull();
  expect(normalizeNextPrompt("continue\nexecute")).toBeNull();
  expect(localNextPrompt("x".repeat(12001) + "\nI can invent an action.")).toBeNull();
});

it("inserts an editable draft without sending and makes no provider call in Local or Off mode", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const load = vi.fn(async (): Promise<NextPromptPrediction | null> => ({ source_run_id:"run", text:"Review the tests", usage:{total_tokens:25} }));
  function Fixture({ mode }: { mode: NextPromptMode }) {
    const [draft, setDraft] = useState("");
    const suggestion = useNextPrompt({ source:{key:"run",output:"I can review the deployment."}, mode, draft, busy:false, load });
    return <><textarea value={draft} onChange={event => setDraft(event.target.value)} />{suggestion.text ? <NextPromptSuggestion text={suggestion.text} onAccept={() => setDraft(suggestion.text!)} onDismiss={suggestion.dismiss} /> : null}</>;
  }
  await act(async () => root.render(<Fixture mode="local" />));
  expect(load).not.toHaveBeenCalled();
  await act(async () => container.querySelector<HTMLButtonElement>(".shared-next-prompt-accept")!.click());
  expect(container.querySelector("textarea")!.value).toBe("Review the deployment");
  expect(container.querySelector(".shared-next-prompt")).toBeNull();
  await act(async () => root.render(<Fixture mode="off" />));
  expect(load).not.toHaveBeenCalled();
  await act(async () => root.unmount());
  container.remove();
});

it("deduplicates Model effect replay and ignores stale source predictions", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const container = document.createElement("div");
  const root = createRoot(container);
  const releases: ((value: NextPromptPrediction | null) => void)[] = [];
  const load = vi.fn(() => new Promise<NextPromptPrediction | null>(resolve => { releases.push(resolve); }));
  function Fixture({ source }: { source: string }) {
    const prediction = useNextPrompt({ source:{key:source, output:""}, mode:"model", draft:"", busy:false, load });
    return <span>{prediction.text}</span>;
  }
  await act(async () => root.render(<StrictMode><Fixture source="old" /></StrictMode>));
  expect(load).toHaveBeenCalledTimes(1);
  await act(async () => root.render(<StrictMode><Fixture source="new" /></StrictMode>));
  expect(load).toHaveBeenCalledTimes(2);
  await act(async () => releases[0]!({source_run_id:"old",text:"Stale old run",usage:null}));
  expect(container.textContent).toBe("");
  await act(async () => releases[1]!({source_run_id:"new",text:"Review the new run",usage:null}));
  expect(container.textContent).toBe("Review the new run");
  await act(async () => root.unmount());
});

it("preserves billed receipts through typing, mode changes, source return, and real unmount", async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  const container = document.createElement("div");
  const root = createRoot(container);
  const releases: ((value: NextPromptPrediction | null) => void)[] = [];
  const signals: AbortSignal[] = [];
  const load = vi.fn((signal: AbortSignal) => {
    signals.push(signal);
    return new Promise<NextPromptPrediction | null>(resolve => releases.push(resolve));
  });
  function Fixture({ source = "one", draft = "", mode = "model" }: { source?: string; draft?: string; mode?: NextPromptMode }) {
    const prediction = useNextPrompt({ source:{key:source,output:""}, mode, draft, busy:false, load });
    return <span>{prediction.text}|{prediction.auxiliaryTokens}</span>;
  }
  const render = async (props: {source?: string; draft?: string; mode?: NextPromptMode}) => act(async () => root.render(<StrictMode><Fixture {...props} /></StrictMode>));
  await render({});
  await render({draft:"my draft"});
  expect(signals[0]!.aborted).toBe(false);
  await act(async () => releases[0]!({source_run_id:"one",text:"Review the first run",usage:{total_tokens:25}}));
  expect(container.textContent).toBe("|25");
  await render({});
  expect(container.textContent).toBe("Review the first run|25");
  await render({mode:"off"});
  expect(container.textContent).toBe("|25");
  await render({});
  expect(load).toHaveBeenCalledTimes(1);
  await render({source:"two"});
  await render({source:"one"});
  await act(async () => releases[1]!({source_run_id:"two",text:"Review the second run",usage:{total_tokens:30}}));
  expect(container.textContent).toBe("Review the first run|25");
  await render({source:"two"});
  expect(container.textContent).toBe("Review the second run|30");
  expect(load).toHaveBeenCalledTimes(2);
  await act(async () => root.unmount());
  await Promise.resolve();
  expect(signals.every(signal => signal.aborted)).toBe(true);
});

it("shows separate charged usage when a provider returns no usable suggestion", async () => {
  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => root.render(<NextPromptSuggestion text={null} onAccept={() => { throw new Error("must not accept absent text"); }} onDismiss={() => {}} auxiliaryTokens={25} />));
  expect(container.textContent).toContain("Prediction: 25 tokens");
  expect(container.querySelector("button")).toBeNull();
  await act(async () => root.unmount());
});
