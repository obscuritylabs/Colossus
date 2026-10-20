import { useEffect, useRef, useState } from "react";
import { Button } from "@colossus/ui";
import { projectPath, request } from "./api";
import { useResource, errorMessage } from "./resources";
import { LoadState } from "./Home";
import { ChatConnectionCredential } from "./ChatConnectionCredential";

interface Connection {
  id: string;
  label: string;
  authentication: "personal" | "shared" | "none";
  authenticated: boolean;
  endpoint?: string;
  credential_binding?: string | null;
  requires_credential?: boolean;
  oauth?: boolean;
}

/** Only the caller's connection metadata is released; secret values are write-only. */
export function ChatCredentials({
  project,
  identity,
  canConnect,
  onConnections,
  onSaved,
}: {
  project: string;
  identity: string;
  canConnect: boolean;
  onConnections: () => void;
  onSaved: () => void;
}) {
  const path = `${projectPath(project)}/chat`;
  const providers = useResource<{ providers: Connection[] }>(
    `${path}/providers`,
    0,
    identity,
  );
  const connections = useResource<{ connections: Connection[] }>(
    `${path}/connections`,
    0,
    identity,
  );
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const scope = JSON.stringify([project, identity]);
  const currentScope = useRef(scope);
  currentScope.current = scope;
  useEffect(
    () => () => {
      currentScope.current = "";
    },
    [],
  );
  const rows = [
    ...(providers.data?.providers ?? []).map((value) => ({
      ...value,
      kind: "providers" as const,
    })),
    ...(connections.data?.connections ?? []).map((value) => ({
      ...value,
      kind: "connections" as const,
    })),
  ];
  function saved() {
    providers.refresh();
    connections.refresh();
    onSaved();
    setNotice("Personal credential saved.");
  }
  return (
    <section
      aria-label="Personal credentials"
      className="chat-credential-inventory"
    >
      <header>
        <h2>Credentials</h2>
        <p className="field-help">
          Your API keys and account connections for this project. Saved
          credentials are encrypted in server storage and stay private to you.
        </p>
      </header>
      <LoadState {...providers} retry={providers.refresh} />
      <LoadState {...connections} retry={connections.refresh} />
      {error ? <p role="alert">{error}</p> : null}
      {notice ? <p role="status">{notice}</p> : null}
      {rows.map((connection) => (
        <section
          className="control-card"
          key={`${connection.kind}:${connection.id}`}
          aria-label={`Credentials for ${connection.label}`}
        >
          <h3>{connection.label}</h3>
          <p className="field-help">
            {connection.authentication === "shared"
              ? "Uses a project credential"
              : connection.authentication === "none"
                ? "No credential required"
                : connection.authenticated
                  ? "Personal credential saved"
                  : "Not connected"}
          </p>
          {connection.authentication === "personal" &&
          !connection.oauth &&
          (connection.requires_credential ||
            connection.kind === "connections") ? (
            <>
              <ChatConnectionCredential
                path={`${path}/${connection.kind}/${encodeURIComponent(connection.id)}/credential`}
                binding={connection.credential_binding ?? null}
                endpoint={connection.endpoint ?? ""}
                disabled={!canConnect || busy}
                onSaved={saved}
                onError={(e) => setError(errorMessage(e))}
              />
              {connection.authenticated ? (
                <Button
                  variant="danger"
                  disabled={!canConnect || busy}
                  onClick={() => {
                    const submittedScope = scope;
                    setBusy(true);
                    setError("");
                    setNotice("");
                    void request(
                      `${path}/${connection.kind}/${encodeURIComponent(connection.id)}/disconnect`,
                      connection.kind === "connections"
                        ? { credential_binding: connection.credential_binding }
                        : {},
                    )
                      .then(() => {
                        if (currentScope.current !== submittedScope) return;
                        providers.refresh();
                        connections.refresh();
                        onSaved();
                        setNotice("Personal credential removed.");
                      })
                      .catch((e) => {
                        if (currentScope.current === submittedScope)
                          setError(errorMessage(e));
                      })
                      .finally(() => {
                        if (currentScope.current === submittedScope)
                          setBusy(false);
                      });
                  }}
                >
                  Remove personal credential
                </Button>
              ) : null}
            </>
          ) : connection.authentication === "personal" ? (
            <Button onClick={onConnections}>Manage account sign-in</Button>
          ) : null}
        </section>
      ))}
      {!providers.loading &&
      !connections.loading &&
      !providers.error &&
      !connections.error &&
      !rows.length ? (
        <p>Your project has no configured connections yet.</p>
      ) : null}
    </section>
  );
}
