import { useEffect, useRef, useState } from "react";
import { IconInbox, IconRefresh, IconSearch } from "@tabler/icons-react";
import { Button } from "./components/ui/button";
import { Input } from "./components/ui/input";
import { DropdownSelect } from "./components/DropdownSelect";
import "../styles/agent-inbox.css";

export type InboxParticipant = {
  id: string;
  root_run_id: string;
  session_id: string;
  run_id: string | null;
  parent_id: string | null;
  subagent_id: string | null;
  generation: number;
  open: boolean;
  closed_reason: string | null;
  pending_messages: number;
  pending_bytes: number;
  created_at: string;
};
export type InboxMessage = {
  id: string;
  root_run_id: string;
  sender:
    | { kind: "participant"; participant_id: string }
    | { kind: "application"; application_id: string };
  recipient_id: string;
  sequence: number;
  text: string;
  reply_to: string | null;
  accepted_at: string;
  receipt:
    | { state: "accepted" }
    | {
        state: "included_in_turn";
        run_id: string;
        turn: number;
        request_hash: string;
      }
    | { state: "not_delivered"; reason: string };
};
export type InboxPage = {
  messages: InboxMessage[];
  next_sequence: number;
  has_more: boolean;
};
export type AgentInboxProps = {
  rootRunId: string;
  available: boolean;
  loadParticipants?:
    ((rootRunId: string) => Promise<InboxParticipant[]>) | undefined;
  loadMessages?:
    | ((participantId: string, afterSequence: number) => Promise<InboxPage>)
    | undefined;
};

function receiptLabel(message: InboxMessage) {
  switch (message.receipt.state) {
    case "accepted":
      return "Waiting for next turn";
    case "included_in_turn":
      return `Included · turn ${message.receipt.turn}`;
    case "not_delivered":
      return `Undelivered · ${message.receipt.reason.replaceAll("_", " ")}`;
  }
}

function participantLabel(item: InboxParticipant, items: InboxParticipant[]) {
  if (!item.parent_id) return "Lead agent";
  const jobs = [
    ...new Set(
      items
        .filter((entry) => entry.parent_id)
        .map((entry) => entry.subagent_id ?? entry.id),
    ),
  ];
  return `Agent ${jobs.indexOf(item.subagent_id ?? item.id) + 1}`;
}

function receivedAt(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? value
    : date.toLocaleString([], {
        month: "short",
        day: "numeric",
        hour: "numeric",
        minute: "2-digit",
      });
}

/** Released records only. The host owns authentication, transport and capability decisions. */
export function AgentInboxInspector({
  rootRunId,
  available,
  loadParticipants,
  loadMessages,
}: AgentInboxProps) {
  const [participants, setParticipants] = useState<InboxParticipant[]>([]);
  const [selected, setSelected] = useState("");
  const [messages, setMessages] = useState<InboxMessage[]>([]);
  const [cursor, setCursor] = useState(0);
  const [hasMore, setHasMore] = useState(false);
  const [busy, setBusy] = useState(false);
  const [participantsBusy, setParticipantsBusy] = useState(false);
  const [error, setError] = useState("");
  const [query, setQuery] = useState("");
  const [state, setState] = useState("all");
  const [revision, setRevision] = useState(0);
  const generation = useRef(0);
  const inboxGeneration = useRef(0);
  const selectedRef = useRef(selected);
  selectedRef.current = selected;
  const scope = useRef("");
  // Host callbacks may be recreated during rendering; only scope changes invalidate reads.
  const loaders = useRef({ loadParticipants, loadMessages });
  loaders.current = { loadParticipants, loadMessages };
  useEffect(() => {
    const current = ++generation.current;
    if (scope.current !== rootRunId || !available) {
      scope.current = rootRunId;
      setParticipants([]);
      setSelected("");
      setMessages([]);
      setCursor(0);
      setHasMore(false);
      setQuery("");
      setState("all");
    }
    setError("");
    setParticipantsBusy(false);
    if (!available || !loaders.current.loadParticipants) return;
    setParticipantsBusy(true);
    void loaders.current
      .loadParticipants(rootRunId)
      .then((items) => {
        if (generation.current !== current) return;
        setParticipants(items);
        setSelected(
          items.some((item) => item.id === selectedRef.current)
            ? selectedRef.current
            : (items[0]?.id ?? ""),
        );
      })
      .catch(() => {
        if (generation.current === current)
          setError("Could not load this run’s inboxes. Refresh to try again.");
      })
      .finally(() => {
        if (generation.current === current) setParticipantsBusy(false);
      });
    return () => {
      generation.current++;
    };
  }, [rootRunId, available, revision]);
  useEffect(() => {
    let active = true;
    ++inboxGeneration.current;
    setMessages([]);
    setCursor(0);
    setHasMore(false);
    setBusy(false);
    if (!selected || !loaders.current.loadMessages) return;
    setBusy(true);
    setError("");
    void loaders.current
      .loadMessages(selected, 0)
      .then((page) => {
        if (!active) return;
        setMessages(page.messages);
        setCursor(page.next_sequence);
        setHasMore(page.has_more);
      })
      .catch(() => {
        if (active)
          setError("Could not load this inbox. Refresh to try again.");
      })
      .finally(() => {
        if (active) setBusy(false);
      });
    return () => {
      active = false;
      ++inboxGeneration.current;
    };
  }, [selected, revision]);
  async function more() {
    if (!loaders.current.loadMessages || busy) return;
    const current = inboxGeneration.current,
      id = selected;
    setBusy(true);
    setError("");
    try {
      const page = await loaders.current.loadMessages(id, cursor);
      if (inboxGeneration.current !== current) return;
      setMessages((previous) => [...previous, ...page.messages].slice(-256));
      setCursor(page.next_sequence);
      setHasMore(page.has_more);
    } catch {
      if (inboxGeneration.current === current)
        setError("Could not load more messages. Try again.");
    } finally {
      if (inboxGeneration.current === current) setBusy(false);
    }
  }
  const participant = participants.find((item) => item.id === selected);
  const needle = query.toLocaleLowerCase();
  const visible = messages.filter(
    (message) =>
      (state === "all" || message.receipt.state === state) &&
      `${message.id} ${JSON.stringify(message.sender)} ${message.text}`
        .toLocaleLowerCase()
        .includes(needle),
  );
  const filtered = query !== "" || state !== "all";
  function senderLabel(message: InboxMessage) {
    if (message.sender.kind === "application")
      return message.sender.application_id === "desktop-managed-local"
        ? "Desktop"
        : message.sender.application_id;
    const id = message.sender.participant_id;
    const sender = participants.find((item) => item.id === id);
    return sender
      ? participantLabel(sender, participants)
      : `Agent · ${id.slice(-8)}`;
  }
  return (
    <section className="agent-inbox" aria-label="Agent inboxes">
      <div className="agent-inbox-heading">
        <div>
          <h2>Agent inboxes</h2>
          <p className="agent-inbox-help">
            Follow messages between agents and see when they reach a turn.
          </p>
        </div>
        <Button
          type="button"
          variant="outline"
          disabled={!available || busy || participantsBusy}
          onClick={() => setRevision((value) => value + 1)}
        >
          <IconRefresh size={16} aria-hidden="true" /> Refresh
        </Button>
      </div>
      {!available ? (
        <p>
          This connection does not provide inbox inspection. A compatible
          runtime and message read permission are required.
        </p>
      ) : (
        <>
          <div className="agent-inbox-recipient">
            <label>
              Recipient attempt
              <DropdownSelect
                aria-label="Recipient attempt"
                value={selected}
                onChange={(event) => setSelected(event.target.value)}
                disabled={busy || participantsBusy || participants.length === 0}
              >
                {participants.length === 0 && (
                  <option value="">No inboxes registered</option>
                )}
                {participants.map((item) => (
                  <option key={item.id} value={item.id}>
                    {participantLabel(item, participants)} · attempt{" "}
                    {item.generation} · {item.open ? "open" : "closed"}
                  </option>
                ))}
              </DropdownSelect>
            </label>
            {participant && (
              <div className="agent-inbox-status" aria-label="Inbox status">
                <span
                  className="agent-inbox-badge"
                  data-state={participant.open ? "open" : "closed"}
                >
                  {participant.open ? "Open" : "Closed"}
                </span>
                <span>{participant.pending_messages} pending</span>
              </div>
            )}
          </div>
          <div className="agent-inbox-controls">
            <label>
              Search loaded messages
              <span className="agent-inbox-search">
                <IconSearch size={16} aria-hidden="true" />
                <Input
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder="Search messages…"
                />
              </span>
            </label>
            <label>
              Receipt
              <DropdownSelect
                aria-label="Receipt"
                value={state}
                onChange={(event) => setState(event.target.value)}
              >
                <option value="all">All receipts</option>
                <option value="accepted">Waiting for next turn</option>
                <option value="included_in_turn">Included in a turn</option>
                <option value="not_delivered">Undelivered</option>
              </DropdownSelect>
            </label>
          </div>
          {participant && (
            <details className="agent-inbox-details">
              <summary>Recipient details</summary>
              <dl>
                <dt>Participant</dt>
                <dd>{participant.id}</dd>
                <dt>Run</dt>
                <dd>{participant.run_id ?? "Queued"}</dd>
                <dt>Job</dt>
                <dd>{participant.subagent_id ?? "Root execution"}</dd>
                <dt>Session</dt>
                <dd>{participant.session_id}</dd>
                <dt>Created</dt>
                <dd>{participant.created_at}</dd>
                {!participant.open && (
                  <>
                    <dt>Closed</dt>
                    <dd>
                      {participant.closed_reason?.replaceAll("_", " ") ??
                        "finished"}
                    </dd>
                  </>
                )}
              </dl>
            </details>
          )}
          {error && <p role="alert">{error}</p>}
          {(busy || participantsBusy) && <p role="status">Loading inbox…</p>}
          <div className="agent-inbox-results">
            <span role="status">
              {visible.length} {visible.length === 1 ? "message" : "messages"}
              {filtered ? ` of ${messages.length} loaded` : " loaded"}
            </span>
            {filtered && (
              <Button
                variant="ghost"
                onClick={() => {
                  setQuery("");
                  setState("all");
                }}
              >
                Clear filters
              </Button>
            )}
          </div>
          {!busy && !participantsBusy && !error && visible.length === 0 && (
            <div className="agent-inbox-empty">
              <IconInbox size={28} stroke={1.5} aria-hidden="true" />
              <strong>
                {filtered
                  ? "No matching messages"
                  : participants.length
                    ? "No messages yet"
                    : "No agent inboxes yet"}
              </strong>
              <p>
                {filtered
                  ? "Try another search or clear the filters."
                  : participants.length
                    ? "No messages have been accepted in this inbox."
                    : "Inboxes appear when agents join this execution."}
              </p>
            </div>
          )}
          <ol className="agent-inbox-messages">
            {visible.map((message) => (
              <li key={message.id}>
                <article>
                  <div className="agent-inbox-message-heading">
                    <strong>{senderLabel(message)}</strong>
                    <span
                      className="agent-inbox-badge"
                      data-state={message.receipt.state}
                    >
                      {receiptLabel(message)}
                    </span>
                  </div>
                  <p className="agent-inbox-meta">
                    Message {message.sequence} ·{" "}
                    <time
                      dateTime={message.accepted_at}
                      title={message.accepted_at}
                    >
                      {receivedAt(message.accepted_at)}
                    </time>
                  </p>
                  <pre>{message.text}</pre>
                  <details>
                    <summary>Message details</summary>
                    <dl>
                      <dt>Message</dt>
                      <dd>{message.id}</dd>
                      <dt>Sender</dt>
                      <dd>
                        {message.sender.kind === "participant"
                          ? message.sender.participant_id
                          : message.sender.application_id}
                      </dd>
                      <dt>Recipient</dt>
                      <dd>{message.recipient_id}</dd>
                      {message.reply_to && (
                        <>
                          <dt>Reply to</dt>
                          <dd>{message.reply_to}</dd>
                        </>
                      )}
                      {message.receipt.state === "included_in_turn" && (
                        <>
                          <dt>Consuming run</dt>
                          <dd>{message.receipt.run_id}</dd>
                          <dt>Prepared turn</dt>
                          <dd>{message.receipt.turn}</dd>
                          <dt>Input request hash</dt>
                          <dd>{message.receipt.request_hash}</dd>
                        </>
                      )}
                    </dl>
                  </details>
                </article>
              </li>
            ))}
          </ol>
          <p className="agent-inbox-help">
            {messages.length > 0 &&
              `Showing messages ${messages[0]?.sequence}–${messages.at(-1)?.sequence}. `}
            Filters apply to the {messages.length} loaded messages.
          </p>
          <details className="agent-inbox-help">
            <summary>About delivery status</summary>
            <p>
              Waiting messages have been accepted. Included messages are part of
              a prepared model turn; this does not confirm that the agent acted
              on them. Refresh to read the latest receipts.
            </p>
          </details>
          {hasMore && (
            <Button type="button" disabled={busy} onClick={() => void more()}>
              Load more messages
            </Button>
          )}
        </>
      )}
    </section>
  );
}
