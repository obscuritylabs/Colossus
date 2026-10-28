import { describe, expect, it } from "vitest";
import {
  conversationFilename,
  conversationMarkdown,
  findConversationMessages,
  type ConversationTextSnapshot,
} from "./conversation-text";

const snapshot: ConversationTextSnapshot = {
  title: "Deployment investigation",
  messages: [
    {
      id: "user",
      role: "user",
      author: "Alice",
      createdAt: "2026-10-27T12:00:00Z",
      text: "Check **Kubernetes** in 東京",
    },
    {
      id: "reply",
      role: "assistant",
      author: "Colossus",
      createdAt: "2026-10-27T12:01:00Z",
      text: "```sh\nkubectl get pods\n```\n\nPods are ready.",
    },
  ],
  earlierHistoryAvailable: false,
  incompleteHistory: false,
  streaming: false,
};

describe("loaded conversation text", () => {
  it("finds matching messages once each, case insensitively, without parsing Markdown", () => {
    const messages = [
      ...snapshot.messages,
      {
        ...snapshot.messages[0]!,
        id: "repeated",
        text: "Kubernetes kubernetes",
      },
    ];
    expect(findConversationMessages(messages, "KUBERNETES")).toEqual([
      "user",
      "repeated",
    ]);
    expect(findConversationMessages(messages, "東京")).toEqual(["user"]);
    expect(findConversationMessages(messages, "**Kubernetes**")).toEqual([
      "user",
    ]);
    expect(findConversationMessages(messages, "Alice")).toEqual([]);
    expect(findConversationMessages(messages, "  ")).toEqual([]);
  });

  it("keeps ordered message Markdown and Unicode intact in a scoped snapshot", () => {
    const markdown = conversationMarkdown(snapshot);
    expect(markdown).toContain("# Deployment investigation");
    expect(markdown).toContain(
      "only loaded, released user and assistant messages",
    );
    expect(markdown).toContain(
      "## User — Alice\n\n2026-10-27T12:00:00Z\n\nCheck **Kubernetes** in 東京",
    );
    expect(markdown).toContain(
      "## Assistant — Colossus\n\n2026-10-27T12:01:00Z\n\n```sh\nkubectl get pods\n```\n\nPods are ready.",
    );
    expect(markdown.indexOf("## User")).toBeLessThan(
      markdown.indexOf("## Assistant"),
    );
    expect(markdown).not.toContain("still in progress");
  });

  it("discloses unloaded, bounded, and still-streaming history without claiming a complete conversation", () => {
    const markdown = conversationMarkdown({
      ...snapshot,
      earlierHistoryAvailable: true,
      incompleteHistory: true,
      streaming: true,
    });
    expect(markdown).toContain(
      "Earlier history is available but is not included",
    );
    expect(markdown).toContain("not the complete agent conversation");
    expect(markdown).toContain("response was still in progress");
    expect(conversationMarkdown({ ...snapshot, messages: [] })).toContain(
      "No loaded user or assistant messages.",
    );
  });

  it("keeps title, author, and time metadata from injecting Markdown blocks or HTML", () => {
    const markdown = conversationMarkdown({
      ...snapshot,
      title: "Title\n# injected <script>",
      messages: [
        {
          ...snapshot.messages[0]!,
          author: "Alice\r\n## forged [link](https://example.com)",
          createdAt: "time\n<img>",
        },
      ],
    });
    expect(markdown).toContain("# Title \\# injected \\<script\\>");
    expect(markdown).not.toContain("\n# injected");
    expect(markdown).not.toContain("\n## forged");
    expect(markdown).not.toContain("<img>");
    expect(markdown).toContain(snapshot.messages[0]!.text);
  });

  it("creates bounded UTF-8 filenames without paths, controls, or Windows device names", () => {
    expect(conversationFilename("Investigate / 東京: 🦀?\n")).toBe(
      "Investigate 東京 🦀-loaded.md",
    );
    expect(conversationFilename("CON")).toBe("conversation-loaded.md");
    expect(conversationFilename("nul.log")).toBe("conversation-loaded.md");
    expect(conversationFilename("... ")).toBe("conversation-loaded.md");
    const filename = conversationFilename("🦀".repeat(100));
    expect(new TextEncoder().encode(filename).length).toBeLessThanOrEqual(210);
    expect(Array.from(filename.replace("-loaded.md", ""))).toHaveLength(50);
    expect(filename).not.toContain("\ufffd");
    expect(conversationFilename("secret\u202etxt")).not.toContain("\u202e");
  });
});
