import { useEffect, useState } from "react";
import { messageOf, type MobileClient, type Operation } from "./client";

/** Reads only; effects are submitted once by explicit event handlers. */
export function usePoll<T>(client: MobileClient, operation: Operation) {
  const [result, setResult] = useState<{
    client: MobileClient;
    key: string;
    value: T;
  } | null>(null);
  const [error, setError] = useState("");
  const [revision, setRevision] = useState(0);
  const key = JSON.stringify(operation);
  useEffect(() => {
    let alive = true,
      pending = false;
    setError("");
    async function read() {
      if (pending || document.hidden) return;
      pending = true;
      try {
        const value = await client.request<T>(JSON.parse(key) as Operation);
        if (alive) {
          setResult({ client, key, value });
          setError("");
        }
      } catch (failure) {
        if (alive) setError(messageOf(failure));
      } finally {
        pending = false;
      }
    }
    void read();
    const timer = setInterval(() => void read(), 5000);
    const wake = () => void read();
    document.addEventListener("visibilitychange", wake);
    window.addEventListener("focus", wake);
    return () => {
      alive = false;
      clearInterval(timer);
      document.removeEventListener("visibilitychange", wake);
      window.removeEventListener("focus", wake);
    };
  }, [client, key, revision]);
  return {
    value:
      result?.client === client && result.key === key ? result.value : null,
    error,
    refresh: () => setRevision((n) => n + 1),
  };
}
