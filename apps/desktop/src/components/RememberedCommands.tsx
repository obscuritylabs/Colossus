import { useEffect, useState } from "react";
import { clearRememberedCommands, rememberedCommandCount } from "../api";

export function RememberedCommands({ spaceId }: { spaceId: string }) {
  const [count, setCount] = useState<number | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    let active = true;
    setCount(null);
    setError("");
    void rememberedCommandCount(spaceId)
      .then((value) => {
        if (active) setCount(value);
      })
      .catch(() => {
        if (active) setError("Could not load remembered commands.");
      });
    return () => {
      active = false;
    };
  }, [spaceId]);
  async function clear() {
    setBusy(true);
    setError("");
    try {
      await clearRememberedCommands(spaceId);
      setCount(0);
    } catch {
      setError("Could not clear remembered commands. Try again.");
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="managed-card">
      <h4>Remembered commands</h4>
      <p>
        Commands you chose to always allow in this workspace. Colossus matches
        the full command and working directory. These preferences stay on this
        computer.
      </p>
      <p>
        {error
          ? ""
          : count === null
            ? "Loading…"
            : `${count} remembered ${count === 1 ? "command" : "commands"}`}
      </p>
      {error ? <p role="alert">{error}</p> : null}
      <button
        className="button secondary"
        type="button"
        disabled={busy || count === null || count === 0}
        onClick={() => void clear()}
      >
        {busy ? "Clearing…" : "Clear remembered commands"}
      </button>
    </section>
  );
}
