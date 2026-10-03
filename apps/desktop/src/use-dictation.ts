import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { ComposerDraft } from "./composer-paste";
import { DictationController, nativeDictationApi } from "./dictation";

const noSubscribe = () => () => {};
const noSnapshot = () => null;

export function useDictation(
  context: string,
  read: () => ComposerDraft,
  write: (draft: ComposerDraft) => void,
) {
  const drafts = useRef({ read, write });
  drafts.current = { read, write };
  const [controller] = useState(() =>
    import.meta.env.DEV
      ? new DictationController(nativeDictationApi, {
          read: () => drafts.current.read(),
          write: (draft) => drafts.current.write(draft),
        })
      : null,
  );
  const state = useSyncExternalStore(
    controller?.subscribe ?? noSubscribe,
    controller?.getSnapshot ?? noSnapshot,
    controller?.getSnapshot ?? noSnapshot,
  );
  useEffect(() => {
    if (!controller) return;
    controller.reset();
    return () => controller.reset();
  }, [context, controller]);
  useEffect(() => {
    if (!controller) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      await controller.poll();
      if (active) timer = setTimeout(() => void poll(), 150);
    };
    void poll();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [controller]);
  return controller && state ? { controller, state } : null;
}
