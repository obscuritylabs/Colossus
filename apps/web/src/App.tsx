import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";

import colossusMark from "@colossus/ui/assets/colossus-mark.svg";
import { IconLoader2 } from "@tabler/icons-react";

import { ApiFailure, request } from "./api";
import { type Me, type AuthConfig, type PublicSettings } from "./control-api";

import { SignIn } from "./SignIn";

import { NavigationProvider, restoreSignInReturn } from "./navigation";

const AuthenticatedApp = lazy(() =>
  import("./ControlPlane").then((module) => ({
    default: module.AuthenticatedApp,
  })),
);
export function App() {
  const [me, setMe] = useState<Me | null>(null),
    [auth, setAuth] = useState<"loading" | "signed_out" | "ready">("loading"),
    [authConfig, setAuthConfig] = useState<AuthConfig | null>(null),
    [settings, setSettings] = useState<PublicSettings | null>(null),
    [error, setError] = useState("");
  const epoch = useRef(0);
  const loadMe = useCallback(async (signal?: AbortSignal) => {
    const generation = ++epoch.current;
    try {
      const response = await request<Me>("/api/me", undefined, signal);
      if (signal?.aborted || generation !== epoch.current) return;
      setMe(response);
      setAuth("ready");
      setError("");
      restoreSignInReturn();
    } catch (e) {
      if (!signal?.aborted && generation === epoch.current) {
        setMe(null);
        setAuth("signed_out");
        if (!(e instanceof ApiFailure && e.status === 403))
          setError(e instanceof Error ? e.message : "Sign-in unavailable.");
      }
    }
  }, []);
  useEffect(() => {
    const abort = new AbortController();
    let pending = false;
    const reconcile = () => {
      if (pending) return;
      pending = true;
      void loadMe(abort.signal).finally(() => {
        pending = false;
      });
    };
    window.addEventListener("colossus:web:reconcile-identity", reconcile);
    return () => {
      abort.abort();
      window.removeEventListener("colossus:web:reconcile-identity", reconcile);
    };
  }, [loadMe]);
  useEffect(() => {
    const abort = new AbortController();
    void loadMe(abort.signal);
    void request<AuthConfig>("/api/auth/config", undefined, abort.signal)
      .then((value) => {
        if (!abort.signal.aborted) setAuthConfig(value);
      })
      .catch(() => {
        if (!abort.signal.aborted)
          setError(
            "Sign-in configuration could not be loaded. Refresh to retry.",
          );
      });
    void request<PublicSettings>("/api/settings", undefined, abort.signal)
      .then((value) => {
        if (!abort.signal.aborted) setSettings(value);
      })
      .catch(() => {});
    return () => abort.abort();
  }, [loadMe]);
  async function signOut() {
    try {
      await request("/auth/logout", {});
      epoch.current++;
      setMe(null);
      setAuth("signed_out");
    } catch (e) {
      setError(e instanceof Error ? e.message : "Sign-out failed.");
    }
  }
  if (auth === "loading")
    return (
      <main className="startup" aria-busy="true">
        <img src={colossusMark} alt="Colossus" />
        <IconLoader2 size={20} className="spin" aria-hidden="true" />
        <p>Connecting to your Control Plane…</p>
      </main>
    );
  if (auth === "signed_out")
    return (
      <SignIn
        config={authConfig}
        classification={settings?.classification}
        error={error}
        onSignedIn={() => void loadMe()}
      />
    );
  if (!me) return null;
  return (
    <NavigationProvider>
      <Suspense
        fallback={
          <main className="startup" role="status">
            Opening Control Plane…
          </main>
        }
      >
        <AuthenticatedApp
          key={me.user.id}
          me={me}
          authConfig={authConfig}
          settings={settings}
          onSettings={setSettings}
          loadMe={() => void loadMe()}
          onSignOut={() => void signOut()}
        />
      </Suspense>
    </NavigationProvider>
  );
}
