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
    [error, setError] = useState(""),
    [signOutError, setSignOutError] = useState(""),
    [sessionRefreshError, setSessionRefreshError] = useState(""),
    [checkingSession, setCheckingSession] = useState(false),
    [signOutPending, setSignOutPending] = useState(false);
  const epoch = useRef(0);
  const settingsEpoch = useRef(0);
  const applySettings = useCallback((value: PublicSettings) => {
    settingsEpoch.current++;
    setSettings(value);
  }, []);
  const signingOut = useRef(false);
  const authenticated = useRef(false);
  const loadMe = useCallback(async (signal?: AbortSignal) => {
    if (signingOut.current) return;
    const generation = ++epoch.current;
    setCheckingSession(authenticated.current);
    try {
      const response = await request<Me>("/api/me", undefined, signal);
      if (signal?.aborted || generation !== epoch.current) return;
      authenticated.current = true;
      setMe(response);
      setAuth("ready");
      setSessionRefreshError("");
      setError("");
      restoreSignInReturn();
    } catch (e) {
      if (!signal?.aborted && generation === epoch.current) {
        if (e instanceof ApiFailure && (e.status === 401 || e.status === 403)) {
          authenticated.current = false;
          setMe(null);
          setAuth("signed_out");
          setSessionRefreshError("");
          setError(e.status === 403 ? "" : e.message);
        } else if (authenticated.current) {
          setSessionRefreshError(
            "Could not refresh your session. Check your connection and retry.",
          );
        } else {
          setAuth("signed_out");
          setError(e instanceof Error ? e.message : "Sign-in unavailable.");
        }
      }
    } finally {
      if (!signal?.aborted && generation === epoch.current)
        setCheckingSession(false);
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
    return () => abort.abort();
  }, [loadMe]);
  useEffect(() => {
    const abort = new AbortController();
    let pending = false;
    const load = async () => {
      if (pending) return;
      pending = true;
      const generation = settingsEpoch.current;
      try {
        const value = await request<PublicSettings>(
          "/api/settings",
          undefined,
          abort.signal,
        );
        if (!abort.signal.aborted && generation === settingsEpoch.current)
          setSettings(value);
      } catch {
        // Keep the last released classification while a background refresh is unavailable.
      } finally {
        pending = false;
      }
    };
    void load();
    const timer = window.setInterval(() => {
      if (!document.hidden) void load();
    }, 30_000);
    const visible = () => {
      if (!document.hidden) void load();
    };
    window.addEventListener("focus", visible);
    document.addEventListener("visibilitychange", visible);
    return () => {
      abort.abort();
      window.clearInterval(timer);
      window.removeEventListener("focus", visible);
      document.removeEventListener("visibilitychange", visible);
    };
  }, []);
  async function signOut() {
    if (signingOut.current) return;
    signingOut.current = true;
    epoch.current++;
    setCheckingSession(false);
    setSignOutPending(true);
    setSignOutError("");
    try {
      await request("/auth/logout", {});
      authenticated.current = false;
      setSessionRefreshError("");
      setCheckingSession(false);
      setMe(null);
      setAuth("signed_out");
      setError("");
    } catch (e) {
      setSignOutError(
        e instanceof ApiFailure && (e.status === 401 || e.status === 403)
          ? "Could not sign out because your session could not be verified. Reload this page, then try again."
          : "Could not sign out. Reload this page to refresh your connection, then try again.",
      );
    } finally {
      signingOut.current = false;
      setSignOutPending(false);
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
          onSettings={applySettings}
          loadMe={() => void loadMe()}
          onSignOut={() => void signOut()}
          signOutError={signOutError}
          signingOut={signOutPending}
          sessionRefreshError={sessionRefreshError}
          checkingSession={checkingSession}
        />
      </Suspense>
    </NavigationProvider>
  );
}
