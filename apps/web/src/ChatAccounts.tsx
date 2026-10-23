import { useEffect, useRef, useState } from "react";
import { Button } from "@colossus/ui";
import { projectPath, request } from "./api";
import { useResource, errorMessage } from "./resources";
import { LoadState } from "./Home";
import type { ChatSettingsSnapshot } from "./chat-settings";

interface Account {
  id: string;
  label: string;
  status: "deployment_managed" | "setup_required";
  reason: string;
  can_connect: boolean;
}
interface McpAccount {
  id: string;
  label: string;
  oauth: boolean;
  authentication: string;
  authenticated: boolean;
}
export function ChatAccounts({
  project,
  identity,
  canConnect,
}: {
  project: string;
  identity: string;
  canConnect: boolean;
}) {
  const path = projectPath(project) + "/chat";
  const accounts = useResource<{
    accounts: Account[];
    execution?: ChatSettingsSnapshot["execution"];
  }>(path + "/accounts", 0, identity);
  const mcp = useResource<{ connections: McpAccount[] }>(
    accounts.data && accounts.data.execution?.settings_editable !== false
      ? path + "/connections"
      : null,
    5000,
    identity,
  );
  const operation = useRef<AbortController | null>(null);
  const editable =
    canConnect && accounts.data?.execution?.settings_editable !== false;
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [setup, setSetup] = useState(false);
  const [authorization, setAuthorization] = useState<{
    id: string;
    url: string;
  } | null>(null);
  const scope = JSON.stringify([project, identity, canConnect]);
  const current = useRef(scope);
  current.current = scope;
  useEffect(() => {
    operation.current?.abort();
    operation.current = null;
    setBusy("");
    setError("");
    setSetup(false);
    setAuthorization(null);
  }, [scope]);
  useEffect(
    () => () => {
      current.current = "";
      operation.current?.abort();
    },
    [],
  );
  async function action(account: McpAccount, disconnect: boolean) {
    if (operation.current || !editable) return;
    const abort = new AbortController();
    operation.current = abort;
    const submitted = scope;
    setBusy(account.id);
    setError("");
    setAuthorization(null);
    try {
      const result = await request<{ authorization_url?: string }>(
        `${path}/connections/${encodeURIComponent(account.id)}/${disconnect ? "disconnect" : "auth"}`,
        {},
        abort.signal,
      );
      if (current.current !== submitted || abort.signal.aborted) return;
      if (!disconnect) {
        const url = new URL(result.authorization_url ?? "");
        if (
          url.protocol !== "https:" ||
          url.username ||
          url.password ||
          url.hash
        )
          throw new Error("The account returned an invalid sign-in address.");
        setAuthorization({ id: account.id, url: url.href });
      }
      mcp.refresh();
    } catch (e) {
      if (current.current === submitted && !abort.signal.aborted)
        setError(errorMessage(e));
    } finally {
      if (operation.current === abort) {
        operation.current = null;
        if (current.current === submitted) setBusy("");
      }
    }
  }
  return (
    <section aria-label="Connected accounts" className="control-card">
      <header>
        <h2>Connected accounts</h2>
      </header>
      <p className="field-help">
        Manage subscription and OAuth sign-ins separately from saved API keys.
        Personal sign-ins stay private to your account in this project.
      </p>
      <LoadState {...accounts} retry={accounts.refresh} />
      <LoadState {...mcp} retry={mcp.refresh} />
      {accounts.data?.execution?.settings_editable === false ? (
        <p className="notice" role="status">
          {accounts.data.execution.reason}
        </p>
      ) : null}
      {error ? <p role="alert">{error}</p> : null}
      {(accounts.data?.accounts ?? []).map((account) => (
        <section
          key={account.id}
          className="chat-connection"
          aria-label={account.label}
        >
          <div>
            <strong>{account.label}</strong>
            <p className="field-help">
              {account.status === "deployment_managed"
                ? "Deployment account configured · Personal linking requires server setup"
                : "Server setup required"}
            </p>
            {setup ? (
              <div role="status">
                <p>{account.reason}</p>
                <p>
                  A server administrator must configure the supported
                  integration before people can link subscriptions here.
                </p>
                <a
                  href="https://developers.openai.com/siwc/token-sharing-open-source"
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  OpenAI connection requirements
                </a>
              </div>
            ) : null}
          </div>
          <Button onClick={() => setSetup((value) => !value)}>
            {setup ? "Hide setup requirements" : "View setup requirements"}
          </Button>
        </section>
      ))}
      {(mcp.data?.connections ?? [])
        .filter(
          (account) => account.oauth && account.authentication === "personal",
        )
        .map((account) => (
          <section
            key={account.id}
            className="chat-connection"
            aria-label={account.label}
          >
            <div>
              <strong>{account.label}</strong>
              <p className="field-help">
                MCP account ·{" "}
                {account.authenticated ? "Connected" : "Sign-in required"}
              </p>
              {authorization?.id === account.id && !account.authenticated ? (
                <p role="status">
                  <a
                    href={authorization.url}
                    target="_blank"
                    rel="noopener noreferrer"
                  >
                    Continue sign-in for {account.label}
                  </a>
                </p>
              ) : null}
            </div>
            <div>
              <Button
                disabled={Boolean(busy) || !editable}
                onClick={() => void action(account, false)}
              >
                {busy === account.id
                  ? "Working…"
                  : account.authenticated
                    ? "Reconnect"
                    : "Connect"}
              </Button>
              {account.authenticated ? (
                <Button
                  disabled={Boolean(busy) || !editable}
                  onClick={() => void action(account, true)}
                >
                  Disconnect
                </Button>
              ) : null}
            </div>
          </section>
        ))}
    </section>
  );
}
