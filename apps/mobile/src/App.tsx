import { useEffect, useState } from "react";
import { Button, DropdownSelect } from "@colossus/ui";
import {
  IconFolder,
  IconServer,
  IconInbox,
  IconSettings,
} from "@tabler/icons-react";
import mark from "@colossus/ui/assets/colossus-mark.svg";
import { Login } from "./Login";
import {
  nativeClient,
  messageOf,
  type AuthConfiguration,
  type Identity,
  type MobileClient,
} from "./client";
import { Workspace, type Tab } from "./Workspace";

const tabs = [
  { id: "work", label: "Work", icon: IconFolder },
  { id: "fleet", label: "Fleet", icon: IconServer },
  { id: "inbox", label: "Inbox", icon: IconInbox },
  { id: "settings", label: "Settings", icon: IconSettings },
] as const;

function preference(name: string, fallback: string): string {
  try {
    return localStorage.getItem(`colossus.mobile.${name}`) ?? fallback;
  } catch {
    return fallback;
  }
}
export function App({
  initialClient = nativeClient,
}: {
  initialClient?: MobileClient;
}) {
  const [locked, setLocked] = useState(false);
  const [client, setClient] = useState(initialClient);
  const [identity, setIdentity] = useState<Identity | null>(null),
    [origin, setOrigin] = useState("");
  const [auth, setAuth] = useState<AuthConfiguration | null>(null);
  const [projectId, setProjectId] = useState("");
  const [tab, setTab] = useState<Tab>("work"),
    [notice, setNotice] = useState("");
  const [theme, setTheme] = useState(() => preference("theme", "dark")),
    [palette, setPalette] = useState(() => preference("palette", "black")),
    [textSize, setTextSize] = useState(() =>
      preference("text-size", "comfortable"),
    );
  useEffect(() => {
    const root = document.documentElement;
    root.dataset.theme = theme === "light" ? "light" : "dark";
    root.dataset.palette = ["black", "colossus", "neutral", "hacker"].includes(
      palette,
    )
      ? palette
      : "black";
    root.dataset.textSize = ["compact", "comfortable", "large"].includes(
      textSize,
    )
      ? textSize
      : "comfortable";
    try {
      localStorage.setItem("colossus.mobile.theme", theme);
      localStorage.setItem("colossus.mobile.palette", palette);
      localStorage.setItem("colossus.mobile.text-size", textSize);
    } catch {
      /* Appearance remains usable without storage. */
    }
  }, [theme, palette, textSize]);
  useEffect(() => {
    const expired = () => {
      setLocked(false);
      setIdentity(null);
      setAuth(null);
      setNotice("Your session ended. Connect and sign in again to continue.");
    };
    window.addEventListener("colossus:mobile-session-expired", expired);
    return () =>
      window.removeEventListener("colossus:mobile-session-expired", expired);
  }, []);
  function signedIn(
    profile: Identity,
    server: string,
    config: AuthConfiguration,
  ) {
    setIdentity(profile);
    setOrigin(server);
    setAuth(config);
    setProjectId(profile.projects[0]?.id ?? "");
    setNotice("");
    setTab("work");
  }
  async function preview() {
    if (!import.meta.env.DEV) return;
    const { previewClient, previewAuth } = await import("./preview");
    setClient(previewClient);
    signedIn(
      await previewClient.login("preview", ""),
      "https://preview.colossus.example",
      previewAuth,
    );
  }
  async function logout() {
    setLocked(true);
    try {
      await client.logout();
      setNotice("Signed out on this device.");
    } catch (failure) {
      setNotice(messageOf(failure));
    } finally {
      setLocked(false);
      setIdentity(null);
      setAuth(null);
      setClient(nativeClient);
    }
  }
  if (!identity)
    return (
      <Login
        client={client}
        onLogin={signedIn}
        notice={notice}
        {...(import.meta.env.DEV ? { onPreview: () => void preview() } : {})}
      />
    );
  const project = identity.projects.find((p) => p.id === projectId);
  const membership = identity.memberships.find(
    (m) => m.project_id === projectId,
  );
  const permissions = project?.archived
    ? (membership?.permissions ?? []).filter(
        (p) => p === "read" || p === "administer",
      )
    : (membership?.permissions ?? []);
  return (
    <div className="mobile-shell">
      {client.preview && (
        <div className="mobile-preview-banner" role="status">
          Development preview · Sample data
        </div>
      )}
      {auth?.classification?.enabled && (
        <div className="mobile-classification">{auth.classification.text}</div>
      )}
      <header className="mobile-app-header">
        <div className="mobile-brand">
          <img src={mark} alt="" />
          <strong>Colossus</strong>
        </div>
        <span className="mobile-avatar" title={identity.user.display_name}>
          {identity.user.display_name
            .split(/\s+/)
            .map((p) => p[0])
            .slice(0, 2)
            .join("")}
        </span>
      </header>
      <div className="mobile-project-picker">
        <IconFolder size={20} />
        <label className="shared-sr-only" htmlFor="project">
          Project
        </label>
        <DropdownSelect
          id="project"
          disabled={locked}
          value={projectId}
          onChange={(event) => {
            setProjectId(event.target.value);
            setTab("work");
          }}
        >
          {identity.projects.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
              {p.archived ? " · Archived" : ""}
            </option>
          ))}
        </DropdownSelect>
      </div>
      <div className="mobile-main" key={`${projectId}:${tab}`}>
        {tab === "settings" ? (
          <main className="mobile-settings">
            <h1>Settings</h1>
            <h2>Your account</h2>
            <p>{identity.user.display_name}</p>
            <p className="mobile-caption mobile-wrap">{origin}</p>
            <p className="mobile-muted">
              Sessions stay in native memory. Reopening the app requires
              sign-in.
            </p>
            <h2>Appearance</h2>
            <label htmlFor="theme">Color theme</label>
            <DropdownSelect
              id="theme"
              value={theme}
              onChange={(event) => setTheme(event.target.value)}
            >
              <option value="dark">Dark</option>
              <option value="light">Light</option>
            </DropdownSelect>
            <label htmlFor="palette">Dark palette</label>
            <DropdownSelect
              id="palette"
              value={palette}
              onChange={(event) => setPalette(event.target.value)}
            >
              <option value="black">Black</option>
              <option value="colossus">Colossus blue</option>
              <option value="neutral">Neutral</option>
              <option value="hacker">Hacker</option>
            </DropdownSelect>
            <label htmlFor="text-size">Text size</label>
            <DropdownSelect
              id="text-size"
              value={textSize}
              onChange={(event) => setTextSize(event.target.value)}
            >
              <option value="compact">Compact</option>
              <option value="comfortable">Comfortable</option>
              <option value="large">Large</option>
            </DropdownSelect>
            <Button
              onClick={() =>
                void client
                  .openBrowser()
                  .catch((failure) => setNotice(messageOf(failure)))
              }
            >
              Open Control Plane
            </Button>
            {notice && <p role="status">{notice}</p>}
            <Button
              variant="danger"
              disabled={locked}
              onClick={() => void logout()}
            >
              {locked ? "Signing out…" : "Sign out"}
            </Button>
          </main>
        ) : project ? (
          <Workspace
            client={client}
            project={projectId}
            permissions={permissions}
            tab={tab}
            onLocked={setLocked}
          />
        ) : (
          <main className="mobile-empty">
            <h1>No projects yet</h1>
            <p>
              Ask your administrator for project access. Signing in does not
              grant execution permissions.
            </p>
          </main>
        )}
      </div>
      <nav className="mobile-bottom-nav" aria-label="Main navigation">
        {tabs.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            type="button"
            aria-current={id === tab ? "page" : undefined}
            disabled={locked}
            onClick={() => setTab(id)}
          >
            <Icon size={23} stroke={1.7} />
            <span>{label}</span>
          </button>
        ))}
      </nav>
    </div>
  );
}
