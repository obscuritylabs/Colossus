import { useEffect, useRef, useState } from "react";
import { Button, TextInput } from "@colossus/ui";
import {
  nativeWorktreesApi,
  worktreeActions,
  worktreeFailure,
} from "../worktrees";
import type {
  WorktreeAction,
  WorktreeCatalog,
  WorktreePreferences,
  WorktreesApi,
} from "../worktrees";
import type { DesktopStatus } from "../types";
import "./worktrees.css";

export function WorktreeSettings({
  workspaceId,
  onOpened,
  api = nativeWorktreesApi,
}: {
  workspaceId: string | null;
  onOpened: (status: DesktopStatus) => void;
  api?: WorktreesApi;
}) {
  const [preferences, setPreferences] = useState<WorktreePreferences | null>(
    null,
  );
  const [catalog, setCatalog] = useState<WorktreeCatalog | null>(null);
  const [permission, setPermission] = useState(false);
  const [pending, setPending] = useState(false);
  const [uncertain, setUncertain] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [name, setName] = useState("");
  const [branch, setBranch] = useState("");
  const [base, setBase] = useState("");
  const [existing, setExisting] = useState(false);
  const generation = useRef(0);
  const busy = useRef(false);

  useEffect(() => {
    const current = ++generation.current;
    busy.current = true;
    setPending(true);
    setCatalog(null);
    setPermission(false);
    setUncertain(false);
    setError("");
    setNotice("");
    const load = async () => {
      try {
        const next = await api.preferences();
        if (current !== generation.current) return;
        setPreferences(next);
        if (workspaceId) {
          const snapshot = await api.list(workspaceId, false);
          if (current !== generation.current) return;
          setCatalog(snapshot.catalog);
          setPermission(snapshot.state === "permission_required");
        }
      } catch (failure) {
        if (current === generation.current)
          setError(worktreeFailure(failure).message);
      } finally {
        if (current === generation.current) {
          busy.current = false;
          setPending(false);
        }
      }
    };
    void load();
    return () => {
      generation.current++;
    };
  }, [workspaceId, api]);

  async function perform(action: () => Promise<void>) {
    if (busy.current) return;
    busy.current = true;
    setPending(true);
    setError("");
    setNotice("");
    const current = generation.current;
    try {
      await action();
    } catch (failure) {
      if (current === generation.current) {
        const detail = worktreeFailure(failure);
        setError(detail.message);
        if (detail.uncertain) setUncertain(true);
      }
    } finally {
      if (current === generation.current) {
        busy.current = false;
        setPending(false);
      }
    }
  }
  async function refresh(approve = false) {
    if (!workspaceId) return;
    const current = generation.current;
    await perform(async () => {
      const snapshot = await api.list(workspaceId, approve);
      if (current !== generation.current) return;
      setCatalog(snapshot.catalog);
      setPermission(snapshot.state === "permission_required");
      setUncertain(false);
    });
  }
  async function mutate(action: WorktreeAction) {
    if (!workspaceId || uncertain) return;
    const current = generation.current;
    await perform(async () => {
      const result = await api.action(workspaceId, action);
      if (current !== generation.current) return;
      setCatalog(result.catalog);
      if (action.kind === "create") {
        setName("");
        setBranch("");
        setBase("");
        setExisting(false);
      }
      setNotice(
        action.kind === "remove"
          ? "Checkout removed. Its branch was retained."
          : "Worktree updated.",
      );
      if (result.opened) onOpened(result.opened);
    });
  }
  async function choose(reset: boolean) {
    const current = generation.current;
    await perform(async () => {
      const next = await api.choose(reset);
      if (current !== generation.current) return;
      setPreferences(next);
      setCatalog(null);
      setPermission(!!workspaceId);
      setNotice(
        "Directory selected. Previous checkouts remain in their original directory.",
      );
    });
  }
  const disabled = pending || uncertain;
  return (
    <section className="worktree-settings" aria-label="Worktrees">
      <header>
        <h3>Worktrees</h3>
        <p>
          Keep independent branches in separate local checkouts. Desktop and CLI
          share the same private inventory.
        </p>
      </header>
      {error && (
        <p className="worktree-error" role="alert">
          {error}
        </p>
      )}
      {uncertain && (
        <p role="status">
          The last action may have completed. Refresh before another change.
          Interrupted entries need manual inspection.
        </p>
      )}
      {notice && <p role="status">{notice}</p>}
      <section className="worktree-group" aria-label="Worktree defaults">
        <header>
          <h4>Defaults</h4>
          <p>
            Changes apply to future checkouts. Existing branches and folders
            keep their current location.
          </p>
        </header>
        <div className="worktree-row">
          <div>
            <strong>Managed directory</strong>
            <p className="worktree-path">
              {preferences?.directory ?? "Loading…"}
            </p>
          </div>
          <div className="worktree-buttons">
            <Button disabled={disabled} onClick={() => void choose(false)}>
              Choose folder…
            </Button>
            <Button disabled={disabled} onClick={() => void choose(true)}>
              Use default
            </Button>
          </div>
        </div>
        {preferences && (
          <form
            className="worktree-defaults"
            onSubmit={(event) => {
              event.preventDefault();
              const current = generation.current;
              void perform(async () => {
                const next = await api.save(preferences.settings);
                if (current !== generation.current) return;
                setPreferences(next);
                setNotice("Worktree defaults saved.");
              });
            }}
          >
            <label>
              New branch prefix
              <TextInput
                value={preferences.settings.branchPrefix}
                maxLength={120}
                disabled={disabled}
                onChange={(event) =>
                  setPreferences({
                    ...preferences,
                    settings: {
                      ...preferences.settings,
                      branchPrefix: event.target.value,
                    },
                  })
                }
              />
            </label>
            <label className="worktree-check">
              <input
                type="checkbox"
                checked={preferences.settings.openAfterCreate}
                disabled={disabled}
                onChange={(event) =>
                  setPreferences({
                    ...preferences,
                    settings: {
                      ...preferences.settings,
                      openAfterCreate: event.target.checked,
                    },
                  })
                }
              />
              Open in Desktop after creation
            </label>
            <Button type="submit" disabled={disabled}>
              Save defaults
            </Button>
          </form>
        )}
      </section>
      <section className="worktree-group" aria-label="Repository worktrees">
        <header className="worktree-row">
          <div>
            <h4>{catalog?.repositoryName ?? "Selected repository"}</h4>
            <p>
              Archive keeps files. Removing an archived checkout requires it to
              be clean and unused; its branch is kept.
            </p>
          </div>
          <Button
            disabled={pending || !workspaceId}
            onClick={() => void refresh(false)}
          >
            Refresh
          </Button>
        </header>
        {!workspaceId ? (
          <p className="worktree-empty">
            Select a local workspace with file access to manage worktrees.
          </p>
        ) : permission ? (
          <div className="worktree-empty">
            <p>
              Allow access to the selected repository and shared Git metadata.
            </p>
            <Button disabled={pending} onClick={() => void refresh(true)}>
              Allow worktrees…
            </Button>
          </div>
        ) : catalog ? (
          <>
            <form
              className="worktree-create"
              onSubmit={(event) => {
                event.preventDefault();
                void mutate({
                  kind: "create",
                  request: {
                    name,
                    branch: branch || null,
                    base: existing ? null : base || null,
                    existingBranch: existing,
                  },
                });
              }}
            >
              <label>
                Name
                <TextInput
                  value={name}
                  placeholder="feature-name"
                  maxLength={64}
                  disabled={disabled}
                  required
                  onChange={(event) => setName(event.target.value)}
                />
              </label>
              <label>
                {existing ? "Existing local branch" : "Branch (optional)"}
                <TextInput
                  value={branch}
                  placeholder={`${preferences?.settings.branchPrefix ?? catalog.settings.branchPrefix}${name || "feature-name"}`}
                  maxLength={200}
                  required={existing}
                  disabled={disabled}
                  onChange={(event) => setBranch(event.target.value)}
                />
              </label>
              <label>
                Base (optional)
                <TextInput
                  value={base}
                  placeholder="HEAD"
                  maxLength={200}
                  disabled={disabled || existing}
                  onChange={(event) => setBase(event.target.value)}
                />
              </label>
              <label className="worktree-check">
                <input
                  type="checkbox"
                  checked={existing}
                  disabled={disabled}
                  onChange={(event) => setExisting(event.target.checked)}
                />
                Use an existing local branch
              </label>
              <Button
                type="submit"
                variant="primary"
                disabled={disabled || !name}
              >
                Create worktree…
              </Button>
            </form>
            {catalog.worktrees.length === 0 ? (
              <p className="worktree-empty">
                No managed worktrees in this directory for this repository.
              </p>
            ) : (
              <ul className="worktree-list">
                {catalog.worktrees.map((entry) => {
                  const actions = worktreeActions(entry);
                  return (
                    <li key={entry.id}>
                      <div className="worktree-row">
                        <div>
                          <strong>{entry.name}</strong>
                          <p>
                            {entry.branch} · {entry.state.replaceAll("_", " ")}
                            {entry.inUse ? " · in use" : ""}
                            {entry.dirty === true
                              ? " · has local files or changes"
                              : ""}
                          </p>
                          <p className="worktree-path">{entry.path}</p>
                          {entry.note && <p role="status">{entry.note}</p>}
                        </div>
                        <div className="worktree-buttons">
                          <Button
                            disabled={disabled || !actions.open}
                            onClick={() =>
                              void mutate({ kind: "open", id: entry.id })
                            }
                          >
                            {entry.state === "archived"
                              ? "Restore and open…"
                              : "Open…"}
                          </Button>
                          {entry.state === "active" && (
                            <Button
                              disabled={disabled || !actions.archive}
                              onClick={() =>
                                void mutate({ kind: "archive", id: entry.id })
                              }
                            >
                              Archive…
                            </Button>
                          )}
                          {entry.state === "archived" && (
                            <Button
                              variant="danger"
                              disabled={disabled || !actions.remove}
                              onClick={() =>
                                void mutate({ kind: "remove", id: entry.id })
                              }
                            >
                              Remove checkout…
                            </Button>
                          )}
                        </div>
                      </div>
                    </li>
                  );
                })}
              </ul>
            )}
          </>
        ) : (
          <p className="worktree-empty">
            {pending
              ? "Loading worktrees…"
              : "Refresh to inspect this repository."}
          </p>
        )}
      </section>
    </section>
  );
}
