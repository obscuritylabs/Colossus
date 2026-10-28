/** The text fields of the released, displayed user and assistant messages only. */
export interface ConversationTextMessage {
  id: string;
  role: "user" | "assistant";
  author: string;
  createdAt: string;
  text: string;
}

export interface ConversationTextSnapshot {
  title: string;
  messages: readonly ConversationTextMessage[];
  earlierHistoryAvailable: boolean;
  incompleteHistory: boolean;
  streaming: boolean;
}

export function findConversationMessages(
  messages: readonly ConversationTextMessage[],
  query: string,
): string[] {
  const needle = query.trim().toLowerCase();
  return needle
    ? messages
        .filter((message) => message.text.toLowerCase().includes(needle))
        .map((message) => message.id)
    : [];
}

function metadata(value: string): string {
  return value
    .replace(/[\s\u0000-\u001f\u007f]+/g, " ")
    .trim()
    .replace(/[\\`*_{}\[\]()<>#+.!|~]/g, "\\$&");
}

/** Keep message Markdown intact; metadata cannot introduce new Markdown blocks. */
export function conversationMarkdown(
  snapshot: ConversationTextSnapshot,
): string {
  const scope = [
    "This snapshot contains only loaded, released user and assistant messages. Run activity, tools, attachments, and private agent context are excluded.",
    snapshot.earlierHistoryAvailable
      ? "Earlier history is available but is not included. Load earlier history in Colossus to include more messages."
      : null,
    snapshot.incompleteHistory
      ? "Released history is incomplete or bounded; this is not the complete agent conversation."
      : null,
    snapshot.streaming
      ? "A response was still in progress when this snapshot was taken."
      : null,
  ].filter(Boolean);
  return (
    [
      `# ${metadata(snapshot.title) || "Conversation"}`,
      scope.join("\n\n"),
      ...snapshot.messages.map(
        (message) =>
          `## ${message.role === "user" ? "User" : "Assistant"} — ${metadata(message.author)}\n\n${metadata(message.createdAt)}\n\n${message.text}`,
      ),
      ...(snapshot.messages.length
        ? []
        : ["No loaded user or assistant messages."]),
    ].join("\n\n") + "\n"
  );
}

export function conversationFilename(title: string): string {
  const characters = Array.from(
    title
      .normalize("NFKC")
      .replace(
        /[\u0000-\u001f\u007f\u200b-\u200f\u202a-\u202e\u2060-\u206f<>:"/\\|?*]/g,
        " ",
      )
      .replace(/\s+/g, " ")
      .trim(),
  ).slice(0, 80);
  const encoder = new TextEncoder();
  let name = "",
    bytes = 0;
  for (const character of characters) {
    const size = encoder.encode(character).length;
    if (bytes + size > 200) break;
    name += character;
    bytes += size;
  }
  name = name.replace(/^[. ]+|[. ]+$/g, "");
  return `${!name || /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i.test(name) ? "conversation" : name}-loaded.md`;
}

/** Downloads are local, explicit snapshots; they never fetch additional history. */
export function downloadConversationMarkdown(title: string, markdown: string) {
  const url = URL.createObjectURL(
    new Blob([markdown], { type: "text/markdown;charset=utf-8" }),
  );
  const link = document.createElement("a");
  try {
    link.href = url;
    link.download = conversationFilename(title);
    document.body.append(link);
    link.click();
  } finally {
    link.remove();
    // Let the browser start the download before releasing the snapshot URL.
    setTimeout(() => URL.revokeObjectURL(url), 0);
  }
}
