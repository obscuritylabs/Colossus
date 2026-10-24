import type {
  HumanTerminalOperation,
  HumanTerminalReply,
} from "@colossus/control-plane-client";

const MAX_REPLY_CHARS = 192 * 1024;
const MAX_PENDING_REQUESTS = 4;

export class FleetTerminalFailure extends Error {
  constructor(public readonly code: string) {
    super("The workspace declined this shell operation.");
  }
}

/** One authenticated socket; failed mutations are never retried implicitly. */
export class FleetTerminalConnection {
  private socket: WebSocket;
  private pending = new Map<
    string,
    {
      resolve: (reply: HumanTerminalReply) => void;
      reject: (error: Error) => void;
      timer: ReturnType<typeof setTimeout>;
    }
  >();
  readonly ready: Promise<void>;

  constructor(
    project: string,
    node: string,
    private onDisconnect: () => void,
  ) {
    const url = new URL(
      `/api/projects/${encodeURIComponent(project)}/nodes/${encodeURIComponent(node)}/terminal`,
      window.location.origin,
    );
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
    this.socket = new WebSocket(url);
    this.ready = new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        reject(
          new Error(
            "The shell connection timed out. Reconnect to inspect its state.",
          ),
        );
        this.socket.close();
      }, 15_000);
      this.socket.addEventListener(
        "open",
        () => {
          clearTimeout(timer);
          resolve();
        },
        { once: true },
      );
      this.socket.addEventListener(
        "error",
        () => {
          clearTimeout(timer);
          reject(new Error("The shell connection could not be opened."));
        },
        { once: true },
      );
      this.socket.addEventListener(
        "close",
        () => {
          clearTimeout(timer);
          reject(new Error("The shell connection closed."));
        },
        { once: true },
      );
    });
    this.socket.addEventListener("message", (event) =>
      this.receive(event.data),
    );
    this.socket.addEventListener("close", (event) => {
      if ([1008, 4401, 4403].includes(event.code))
        window.dispatchEvent(new Event("colossus:web:reconcile-identity"));
      this.rejectPending();
      this.onDisconnect();
    });
  }

  async request(
    operation: HumanTerminalOperation,
  ): Promise<HumanTerminalReply> {
    await this.ready;
    if (this.socket.readyState !== WebSocket.OPEN)
      throw new Error("The shell is disconnected. Reconnect before typing.");
    if (this.pending.size >= MAX_PENDING_REQUESTS)
      throw new Error("Wait for the pending shell operations to finish.");
    const id = crypto.randomUUID();
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(
          new Error(
            "The shell did not confirm this operation. Reconnect to inspect its state; input will not be sent again.",
          ),
        );
        this.close();
      }, 15_000);
      this.pending.set(id, { resolve, reject, timer });
      try {
        this.socket.send(JSON.stringify({ id, operation }));
      } catch {
        clearTimeout(timer);
        this.pending.delete(id);
        reject(new Error("The shell did not confirm this operation."));
      }
    });
  }

  close() {
    this.rejectPending();
    this.socket.close();
  }

  private rejectPending() {
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(
        new Error("Disconnected. Unconfirmed input is not replayed."),
      );
    }
    this.pending.clear();
  }

  private receive(data: unknown) {
    if (typeof data !== "string" || data.length > MAX_REPLY_CHARS) {
      this.close();
      return;
    }
    try {
      const value = JSON.parse(data) as {
        id?: unknown;
        reply?: HumanTerminalReply;
        error?: { code?: string };
      };
      if (typeof value.id !== "string") throw new Error();
      const pending = this.pending.get(value.id);
      if (!pending) return;
      clearTimeout(pending.timer);
      this.pending.delete(value.id);
      if (value.error) {
        if (
          ["unauthenticated", "permission_denied"].includes(
            value.error.code ?? "",
          )
        )
          window.dispatchEvent(new Event("colossus:web:reconcile-identity"));
        pending.reject(
          new FleetTerminalFailure(value.error.code ?? "internal"),
        );
      } else if (value.reply) pending.resolve(value.reply);
      else
        pending.reject(
          new Error("The workspace returned an invalid shell reply."),
        );
    } catch {
      this.close();
    }
  }
}
