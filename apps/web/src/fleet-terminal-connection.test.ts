// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { FleetTerminalConnection } from "./fleet-terminal-connection";
import type { HumanTerminalReply } from "@colossus/control-plane-client";

class Socket extends EventTarget {
  static OPEN = 1;
  static instances: Socket[] = [];
  readyState = 0;
  sent: string[] = [];
  constructor(readonly url: URL) {
    super();
    Socket.instances.push(this);
  }
  open() {
    this.readyState = 1;
    this.dispatchEvent(new Event("open"));
  }
  send(value: string) {
    this.sent.push(value);
  }
  close() {
    this.readyState = 3;
    this.dispatchEvent(Object.assign(new Event("close"), { code: 1000 }));
  }
  message(data: string) {
    this.dispatchEvent(new MessageEvent("message", { data }));
  }
}
const reply: HumanTerminalReply = {
  available: true,
  incarnation: "fixture",
  sessions: [],
  session: null,
  frames: [],
  next_sequence: 0,
  gap: false,
  input_sequence: 0,
};
beforeEach(() => {
  Socket.instances = [];
  vi.stubGlobal("WebSocket", Socket);
});
afterEach(() => vi.unstubAllGlobals());
const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

it("confines the native socket URL and sends operation envelopes without caller-controlled ownership", async () => {
  const connection = new FleetTerminalConnection("p/", "n?", vi.fn());
  const socket = Socket.instances[0]!;
  expect(socket.url.protocol).toBe("ws:");
  expect(socket.url.pathname).toBe("/api/projects/p%2F/nodes/n%3F/terminal");
  socket.open();
  const pending = connection.request({ operation: "context" });
  await flush();
  const sent = JSON.parse(socket.sent[0]!);
  expect(Object.keys(sent).sort()).toEqual(["id", "operation"]);
  expect(sent.operation).toEqual({ operation: "context" });
  socket.message(JSON.stringify({ id: sent.id, reply }));
  await expect(pending).resolves.toEqual(reply);
  connection.close();
});

it("rejects an unconfirmed write on disconnect without opening another socket or replaying bytes", async () => {
  const connection = new FleetTerminalConnection("p", "n", vi.fn());
  const socket = Socket.instances[0]!;
  socket.open();
  const pending = connection.request({
    operation: "write",
    terminal_id: "shell",
    writer_epoch: 1,
    input_sequence: 1,
    data_base64: "aGk=",
  });
  const failed = expect(pending).rejects.toThrow(
    "Unconfirmed input is not replayed",
  );
  await flush();
  socket.close();
  await failed;
  expect(socket.sent).toHaveLength(1);
  expect(Socket.instances).toHaveLength(1);
});

it("reconciles identity after a terminal authorization denial", async () => {
  const reconcile = vi.fn();
  window.addEventListener("colossus:web:reconcile-identity", reconcile);
  const connection = new FleetTerminalConnection("p", "n", vi.fn());
  const socket = Socket.instances[0]!;
  socket.open();
  const pending = connection.request({ operation: "context" });
  const failed = expect(pending).rejects.toMatchObject({
    code: "permission_denied",
  });
  await flush();
  socket.message(
    JSON.stringify({
      id: JSON.parse(socket.sent[0]!).id,
      error: { code: "permission_denied" },
    }),
  );
  await failed;
  expect(reconcile).toHaveBeenCalledOnce();
  window.removeEventListener("colossus:web:reconcile-identity", reconcile);
  connection.close();
});

it("closes oversized replies instead of buffering unbounded terminal history", async () => {
  const disconnected = vi.fn();
  const connection = new FleetTerminalConnection("p", "n", disconnected);
  const socket = Socket.instances[0]!;
  socket.open();
  const pending = connection.request({ operation: "context" });
  const failed = expect(pending).rejects.toThrow("Disconnected");
  await flush();
  socket.message("x".repeat(192 * 1024 + 1));
  await failed;
  expect(disconnected).toHaveBeenCalledOnce();
  expect(socket.readyState).toBe(3);
});
