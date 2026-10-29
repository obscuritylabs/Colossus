import { useEffect, useState } from "react";
import { featureError, getPetContext, petAction } from "../desktop-extras";
import type { PetContext, PetStatus } from "../desktop-extras";
import "./desktop-extras.css";

const LABELS: Record<PetStatus, string> = {
  idle: "Ready to build",
  working: "Working",
  attention: "Your input is needed",
  complete: "Work complete",
};
export function MiniHackerArt({ status }: { status: PetStatus }) {
  return (
    <svg
      viewBox="0 0 128 110"
      aria-hidden="true"
      className="hacker-art"
      shapeRendering="crispEdges"
    >
      <path className="pet-hood" d="M28 68V30h8V18h12v-8h32v8h12v12h8v38z" />
      <path className="pet-face" d="M40 32h48v34H40z" />
      <g className="pet-eyes">
        <path d="M48 44h10v6H48zm22 0h10v6H70z" />
      </g>
      <path
        className="pet-mouth"
        d={status === "attention" ? "M61 53h6v9h-6z" : "M56 56h16v4H56z"}
      />
      <path className="pet-hands" d="M22 70h18v12H22zm66 0h18v12H88z" />
      <path className="pet-laptop" d="M32 64h64v32H32zm-10 32h84v7H22z" />
      <path className="pet-screen" d="M38 70h52v20H38z" />
      <path
        className="pet-prompt"
        d="m45 74 6 5-6 5h5l6-5-6-5zm14 9h16v3H59z"
      />
      {status === "complete" && (
        <path
          className="pet-status-icon"
          d="m94 15 7 7 13-15 5 5-18 19-12-11z"
        />
      )}
      {status === "attention" && (
        <path className="pet-status-icon" d="M100 6h8v17h-8zm0 23h8v8h-8z" />
      )}
    </svg>
  );
}
export default function MiniHackerPet() {
  const [context, setContext] = useState<PetContext>({
    status: "idle",
    motion: false,
    size: "small",
  });
  const [error, setError] = useState("");
  const [contextError, setContextError] = useState("");
  useEffect(() => {
    document.documentElement.classList.add("pet-surface");
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    // Poll only the narrow native context. Generic Tauri event authority would
    // also expose other windows' events. Schedule after settlement so at most
    // one context request is in flight, including when native code is slow.
    const load = async () => {
      try {
        const value = await getPetContext();
        if (live) {
          setContext(value);
          setContextError("");
        }
      } catch (failure) {
        if (live) setContextError(featureError(failure));
      } finally {
        if (live) timer = setTimeout(() => void load(), 1000);
      }
    };
    void load();
    return () => {
      live = false;
      clearTimeout(timer);
      document.documentElement.classList.remove("pet-surface");
    };
  }, []);
  const visibleError = error || contextError;
  return (
    <main
      className={`developer-pet pet-${context.status} pet-size-${context.size} ${context.motion ? "pet-motion" : ""}`}
      aria-label="Colossus developer companion"
    >
      <div
        className="pet-character"
        data-tauri-drag-region
        onPointerUp={() => void petAction("save_position").catch(() => {})}
      >
        <MiniHackerArt status={context.status} />
      </div>
      <span className="pet-status" role="status">
        {LABELS[context.status]}
      </span>
      <div className="pet-controls">
        <button
          type="button"
          aria-label="Open Colossus"
          title="Open Colossus"
          onClick={() =>
            void petAction("open").catch((failure) =>
              setError(featureError(failure)),
            )
          }
        >
          Open
        </button>
        <button
          type="button"
          aria-label="Hide companion"
          title="Hide companion"
          onClick={() =>
            void petAction("hide").catch((failure) =>
              setError(featureError(failure)),
            )
          }
        >
          Hide
        </button>
      </div>
      {visibleError && (
        <span className="pet-error" role="alert">
          {visibleError}
        </span>
      )}
    </main>
  );
}
