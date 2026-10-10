import { useEffect, useRef, useState } from "react";
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
      return "Accepted · awaiting input inclusion";
    case "included_in_turn":
      return `Included in prepared turn ${message.receipt.turn}`;
    case "not_delivered":
      return `Undelivered · ${message.receipt.reason.replaceAll("_", " ")}`;
  }
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
  // Host callbacks may be recreated during rendering; only scope changes invalidate reads.
  const loaders = useRef({ loadParticipants, loadMessages });
  loaders.current = { loadParticipants, loadMessages };
  useEffect(() => {
    const current = ++generation.current;
    setParticipants([]);
    setSelected("");
    setMessages([]);
    setCursor(0);
    setHasMore(false);
    setError("");
    setParticipantsBusy(false);
    if (!available || !loaders.current.loadParticipants) return;
    setParticipantsBusy(true);
    void loaders.current
      .loadParticipants(rootRunId)
      .then((items) => {
        if (generation.current !== current) return;
        setParticipants(items);
        setSelected(items[0]?.id ?? "");
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
  return (
    <section className="agent-inbox" aria-label="Agent inboxes">
      <div className="agent-inbox-heading">
        <h2>Agent inboxes</h2>
        <Button
          type="button"
          disabled={!available || busy || participantsBusy}
          onClick={() => setRevision((value) => value + 1)}
        >
          Refresh
        </Button>
      </div>
      {!available ? (
        <p>
          This connection does not provide inbox inspection. A compatible
          runtime and message read permission are required.
        </p>
      ) : (
        <>
          <p className="agent-inbox-help">
            Receipts distinguish accepted text from text included in a prepared
            turn. Refresh to read the latest receipts.
          </p>
          <div className="agent-inbox-controls">
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
                    {item.parent_id ? "Child" : "Root"} · attempt{" "}
                    {item.generation} · {item.id} ·{" "}
                    {item.open ? "open" : "closed"}
                  </option>
                ))}
              </DropdownSelect>
            </label>
            <label>
              Search loaded messages
              <Input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder="Sender, message ID, or text"
              />
            </label>
            <label>
              Receipt
              <DropdownSelect
                aria-label="Receipt"
                value={state}
                onChange={(event) => setState(event.target.value)}
              >
                <option value="all">All receipts</option>
                <option value="accepted">Accepted</option>
                <option value="included_in_turn">Included in a turn</option>
                <option value="not_delivered">Undelivered</option>
              </DropdownSelect>
            </label>
          </div>
          {participant && (
            <details className="agent-inbox-details">
              <summary>
                {participant.open
                  ? "Open"
                  : `Closed · ${participant.closed_reason?.replaceAll("_", " ") ?? "finished"}`}{" "}
                · {participant.pending_messages} pending
              </summary>
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
              </dl>
            </details>
          )}
          {error && <p role="alert">{error}</p>}
          {(busy || participantsBusy) && <p role="status">Loading inbox…</p>}
          {!busy && !participantsBusy && !error && visible.length === 0 && (
            <p>
              {messages.length
                ? "No loaded messages match these filters."
                : "No messages have been accepted in this inbox."}
            </p>
          )}
          <ol className="agent-inbox-messages">
            {visible.map((message) => (
              <li key={message.id}>
                <article>
                  <div className="agent-inbox-message-heading">
                    <strong>Message {message.sequence}</strong>
                    <span>{receiptLabel(message)}</span>
                  </div>
                  <p className="agent-inbox-meta">
                    From{" "}
                    {message.sender.kind === "participant"
                      ? message.sender.participant_id
                      : message.sender.application_id}{" "}
                    ·{" "}
                    <time dateTime={message.accepted_at}>
                      {message.accepted_at}
                    </time>
                  </p>
                  <pre>{message.text}</pre>
                  <details>
                    <summary>Message details</summary>
                    <dl>
                      <dt>Message</dt>
                      <dd>{message.id}</dd>
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
