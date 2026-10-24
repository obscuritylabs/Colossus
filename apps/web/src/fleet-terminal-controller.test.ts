// @vitest-environment happy-dom
import { beforeEach, expect, it, vi } from "vitest";
import type { Terminal } from "@xterm/xterm";
import type {
  HumanTerminalOperation,
  HumanTerminalReply,
  HumanTerminalSession,
} from "@colossus/control-plane-client";
import {
  FleetTerminalController,
  type FleetTerminalState,
} from "./fleet-terminal-controller";

const wire = vi.hoisted(() => ({
  requests: [] as HumanTerminalOperation[],
  handler: null as
    ((operation: HumanTerminalOperation) => Promise<HumanTerminalReply>) | null,
  dropped: null as (() => void) | null,
}));
vi.mock("./fleet-terminal-connection", () => ({
  FleetTerminalFailure: class extends Error {
    constructor(public code: string) {
      super(code);
    }
  },
  FleetTerminalConnection: class {
    constructor(_project: string, _node: string, disconnected: () => void) {
      wire.dropped = disconnected;
    }
    request(operation: HumanTerminalOperation) {
      wire.requests.push(operation);
      return wire.handler!(operation);
    }
    close() {}
  },
}));
const reply = (
  patch: Partial<HumanTerminalReply> = {},
): HumanTerminalReply => ({
  available: true,
  incarnation: "incarnation",
  sessions: [],
  session: null,
  frames: [],
  next_sequence: 0,
  gap: false,
  input_sequence: 0,
  ...patch,
});
let session: HumanTerminalSession;
beforeEach(() => {
  sessionStorage.clear();
  wire.requests = [];
  wire.dropped = null;
  session = {
    terminal_id: "shell",
    controller_id: "",
    writer_epoch: 1,
    rows: 24,
    cols: 80,
    status: "running",
    first_sequence: 1,
    next_sequence: 1,
    complete_history: true,
    exit_code: null,
  };
  wire.handler = async (operation) => {
    if (operation.operation === "context") return reply();
    if (operation.operation === "open") {
      session.controller_id = operation.controller_id;
      return reply({ session });
    }
    if (operation.operation === "attach") return reply({ session });
    if (operation.operation === "read") {
      if (operation.after_sequence === 0)
        return reply({
          session,
          frames: [
            {
              sequence: 1,
              kind: { kind: "output", data_base64: btoa("saved output") },
            },
          ],
          next_sequence: 1,
        });
      return new Promise(() => {});
    }
    if (operation.operation === "write")
      return reply({ session, input_sequence: operation.input_sequence });
    return reply({ session });
  };
});
const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
function setup(write?: (bytes: Uint8Array, done: () => void) => void) {
  const terminal = {
    cols: 80,
    rows: 24,
    write: vi.fn(write ?? ((_bytes, done) => done())),
    resize: vi.fn(),
    reset: vi.fn(),
  };
  const states: FleetTerminalState[] = [];
  const controller = new FleetTerminalController(
    terminal as unknown as Terminal,
    "project",
    "node",
    "project:node",
    (state) => states.push(state),
  );
  return { controller, terminal, states };
}

it("does not request the next output page until the emulator has parsed the current bytes", async () => {
  let parsed: (() => void) | null = null;
  const { controller, terminal, states } = setup((_bytes, done) => {
    parsed = done;
  });
  void controller.connect();
  await flush();
  expect(terminal.write).toHaveBeenCalledOnce();
  expect(
    wire.requests.filter((request) => request.operation === "read"),
  ).toHaveLength(1);
  expect(states.at(-1)?.phase).toBe("live");
  parsed!();
  await flush();
  expect(states.at(-1)?.phase).toBe("live");
  expect(
    wire.requests.filter((request) => request.operation === "read"),
  ).toHaveLength(2);
  controller.dispose();
});

it("reattaches a fresh page to the existing shell and replays its display from zero", async () => {
  const first = setup();
  void first.controller.connect();
  await flush();
  first.controller.dispose();
  const second = setup();
  void second.controller.connect();
  await flush();
  expect(
    wire.requests.filter((request) => request.operation === "open"),
  ).toHaveLength(1);
  expect(wire.requests).toContainEqual(
    expect.objectContaining({
      operation: "attach",
      terminal_id: "shell",
      takeover: false,
    }),
  );
  expect(
    wire.requests.filter(
      (request) => request.operation === "read" && request.after_sequence === 0,
    ),
  ).toHaveLength(2);
  expect(second.terminal.write).toHaveBeenCalledOnce();
  second.controller.dispose();
});

it("commits output already queued in the emulator before reconnecting after a socket drop", async () => {
  let parsed: (() => void) | null = null;
  const { controller, terminal } = setup((_bytes, done) => {
    parsed = done;
  });
  void controller.connect();
  await flush();
  wire.dropped!();
  void controller.connect();
  await flush();
  expect(
    wire.requests.filter((request) => request.operation === "read"),
  ).toHaveLength(1);
  parsed!();
  await flush();
  expect(wire.requests).toContainEqual(
    expect.objectContaining({ operation: "read", after_sequence: 1 }),
  );
  expect(terminal.write).toHaveBeenCalledOnce();
  controller.dispose();
});

it("coalesces ordered key events behind an outstanding write acknowledgement", async () => {
  const original = wire.handler!;
  let acknowledge: (() => void) | null = null;
  wire.handler = async (operation) => {
    if (operation.operation === "write" && operation.input_sequence === 1)
      return new Promise((resolve) => {
        acknowledge = () => resolve(reply({ session, input_sequence: 1 }));
      });
    return original(operation);
  };
  const { controller } = setup();
  void controller.connect();
  await flush();
  controller.input(new TextEncoder().encode("f"));
  expect(
    wire.requests.filter((request) => request.operation === "write"),
  ).toHaveLength(1);
  const queued = "leet 界\r".repeat(40);
  for (const key of queued) controller.input(new TextEncoder().encode(key));
  await flush();
  expect(
    wire.requests.filter((request) => request.operation === "write"),
  ).toHaveLength(1);
  acknowledge!();
  await flush();
  const writes = wire.requests.filter(
    (request) => request.operation === "write",
  );
  expect(writes.map((write) => write.input_sequence)).toEqual([1, 2]);
  expect(writes.map((write) => write.writer_epoch)).toEqual([1, 1]);
  expect(atob(writes[0]!.data_base64)).toBe("f");
  expect(
    new TextDecoder().decode(
      Uint8Array.from(atob(writes[1]!.data_base64), (char) =>
        char.charCodeAt(0),
      ),
    ),
  ).toBe(queued);
  controller.dispose();
});

it("counts the outstanding write against its bounded queued input budget", async () => {
  const original = wire.handler!;
  let acknowledge: (() => void) | null = null;
  wire.handler = async (operation) => {
    if (operation.operation === "write" && operation.input_sequence === 1)
      return new Promise((resolve) => {
        acknowledge = () => resolve(reply({ session, input_sequence: 1 }));
      });
    return original(operation);
  };
  const { controller, states } = setup();
  void controller.connect();
  await flush();
  controller.input(new Uint8Array(65536).fill(65));
  controller.input(new Uint8Array(65535).fill(66));
  controller.input(new Uint8Array([67]));
  controller.input(new Uint8Array([68]));
  expect(states.at(-1)?.message).toContain("Input was not sent");
  acknowledge!();
  await flush();
  const writes = wire.requests.filter(
    (request) => request.operation === "write",
  );
  expect(writes.map((write) => atob(write.data_base64))).toEqual([
    "A".repeat(65536),
    `${"B".repeat(65535)}C`,
  ]);
  expect(writes.map((write) => write.input_sequence)).toEqual([1, 2]);
  controller.dispose();
});

it("drops batched queued input on disconnect and ignores a stale write acknowledgement", async () => {
  const original = wire.handler!;
  let acknowledge: (() => void) | null = null;
  let firstWrite = true;
  let resumeRead = false;
  wire.handler = async (operation) => {
    if (operation.operation === "write" && firstWrite) {
      firstWrite = false;
      return new Promise((resolve) => {
        acknowledge = () => resolve(reply({ session, input_sequence: 1 }));
      });
    }
    if (operation.operation === "attach")
      return reply({ session, input_sequence: 1 });
    if (operation.operation === "read" && resumeRead) {
      resumeRead = false;
      return reply({ session, next_sequence: operation.after_sequence });
    }
    return original(operation);
  };
  const { controller, states } = setup();
  void controller.connect();
  await flush();
  controller.input(new TextEncoder().encode("first\r"));
  for (const key of "never replay\r".repeat(40))
    controller.input(new TextEncoder().encode(key));
  await flush();
  wire.dropped!();
  expect(states.at(-1)?.phase).toBe("disconnected");
  resumeRead = true;
  session.writer_epoch = 2;
  void controller.connect();
  await flush();
  expect(
    wire.requests.filter((request) => request.operation === "write"),
  ).toHaveLength(1);
  expect(wire.requests).toContainEqual(
    expect.objectContaining({ operation: "read", after_sequence: 1 }),
  );
  acknowledge!();
  await flush();
  expect(
    wire.requests.filter((request) => request.operation === "write"),
  ).toHaveLength(1);
  controller.input(new TextEncoder().encode("fresh\r"));
  await flush();
  const writes = wire.requests.filter(
    (request) => request.operation === "write",
  );
  expect(writes.map((write) => atob(write.data_base64))).toEqual([
    "first\r",
    "fresh\r",
  ]);
  expect(writes[1]!.input_sequence).toBe(2);
  expect(writes[1]!.writer_epoch).toBe(2);
  controller.dispose();
});

it("reports a history gap without feeding a corrupt suffix to the emulator", async () => {
  const original = wire.handler!;
  wire.handler = async (operation) =>
    operation.operation === "read"
      ? reply({ session, gap: true, next_sequence: 500 })
      : original(operation);
  const { controller, terminal, states } = setup();
  void controller.connect();
  await flush();
  expect(states.at(-1)?.phase).toBe("history_gap");
  expect(terminal.write).not.toHaveBeenCalled();
  controller.input(new TextEncoder().encode("unsafe\r"));
  expect(wire.requests.some((request) => request.operation === "write")).toBe(
    false,
  );
  controller.dispose();
});

it("never opens a new process after the native owner incarnation changes", async () => {
  const first = setup();
  void first.controller.connect();
  await flush();
  first.controller.dispose();
  const original = wire.handler!;
  wire.handler = async (operation) =>
    operation.operation === "context"
      ? reply({ incarnation: "new-incarnation" })
      : original(operation);
  const second = setup();
  void second.controller.connect();
  await flush();
  expect(second.states.at(-1)?.message).toContain("restarted");
  expect(
    wire.requests.filter((request) => request.operation === "open"),
  ).toHaveLength(1);
  second.controller.dispose();
});

it.each([
  { name: "worker restart", incarnation: "restarted", message: "restarted" },
  {
    name: "same-worker session expiry",
    incarnation: "incarnation",
    message: "could not be confirmed",
  },
])(
  "requires an explicit new shell after a lost Open reply and $name",
  async ({ incarnation, message }) => {
    const original = wire.handler!;
    wire.handler = async (operation) => {
      if (operation.operation === "open") {
        session.controller_id = operation.controller_id;
        expect(
          JSON.parse(
            sessionStorage.getItem("colossus:fleet-shell:project:node")!,
          ),
        ).toMatchObject({
          incarnation: "incarnation",
          open_pending: true,
          new_requested: false,
        });
        throw new Error("Open reply was lost");
      }
      return original(operation);
    };
    const first = setup();
    await first.controller.connect();
    expect(first.states.at(-1)?.phase).toBe("error");
    first.controller.dispose();
    wire.handler = async (operation) => ({
      ...(await original(operation)),
      incarnation,
    });
    const second = setup();
    await second.controller.connect();
    expect(second.states.at(-1)?.phase).toBe("error");
    expect(second.states.at(-1)?.message).toContain(message);
    expect(
      wire.requests.filter((operation) => operation.operation === "open"),
    ).toHaveLength(1);
    expect(
      wire.requests.some((operation) => operation.operation === "attach"),
    ).toBe(false);

    await second.controller.startNew();
    await flush();
    const opens = wire.requests.filter(
      (operation) => operation.operation === "open",
    );
    expect(opens).toHaveLength(2);
    expect(opens[1]!.request_id).not.toBe(opens[0]!.request_id);
    expect(opens[1]!.controller_id).not.toBe(opens[0]!.controller_id);
    expect(second.states.at(-1)?.phase).toBe("live");
    second.controller.dispose();
  },
);

it.each(["running", "exited"] as const)(
  "reconciles a lost Open reply with its retained %s shell without repeating Open",
  async (status) => {
    const original = wire.handler!;
    let opened = false;
    wire.handler = async (operation) => {
      if (operation.operation === "context")
        return reply({ sessions: opened ? [session] : [] });
      if (operation.operation === "open") {
        session.controller_id = operation.controller_id;
        session.status = status;
        opened = true;
        throw new Error("Open reply was lost");
      }
      return original(operation);
    };
    const first = setup();
    await first.controller.connect();
    first.controller.dispose();
    const second = setup();
    void second.controller.connect();
    await flush();
    expect(
      wire.requests.filter((operation) => operation.operation === "open"),
    ).toHaveLength(1);
    expect(wire.requests).toContainEqual(
      expect.objectContaining({
        operation: "attach",
        terminal_id: "shell",
        controller_id: session.controller_id,
      }),
    );
    expect(second.terminal.write).toHaveBeenCalledOnce();
    expect(second.states.at(-1)?.phase).toBe(
      status === "running" ? "live" : "exited",
    );
    expect(
      JSON.parse(sessionStorage.getItem("colossus:fleet-shell:project:node")!),
    ).toMatchObject({ terminal_id: "shell", open_pending: false });
    second.controller.dispose();
  },
);

it("offers explicit takeover for an existing shell owned by another tab without starting a process", async () => {
  session.controller_id = "other-tab";
  const original = wire.handler!;
  wire.handler = async (operation) => {
    if (operation.operation === "context")
      return reply({ sessions: [session] });
    if (operation.operation === "attach") {
      expect(operation.takeover).toBe(true);
      session = {
        ...session,
        controller_id: operation.controller_id,
        writer_epoch: 2,
      };
      return reply({ session });
    }
    return original(operation);
  };
  const { controller, states } = setup();
  void controller.connect();
  await flush();
  expect(states.at(-1)?.phase).toBe("takeover");
  expect(
    wire.requests.some(
      (operation) =>
        operation.operation === "open" || operation.operation === "attach",
    ),
  ).toBe(false);
  void controller.connect(true);
  await flush();
  expect(states.at(-1)?.controlling).toBe(true);
  expect(
    wire.requests.some((operation) => operation.operation === "open"),
  ).toBe(false);
  controller.dispose();
});

it("answers new VT device queries but does not send replies from historical output after fresh-page attachment", async () => {
  const answer = new TextEncoder().encode("\u001b[1;1R");
  const first = setup((_bytes, done) => {
    first.controller.input(answer);
    done();
  });
  void first.controller.connect();
  await flush();
  expect(
    wire.requests.filter((operation) => operation.operation === "write"),
  ).toHaveLength(1);
  first.controller.dispose();
  const second = setup((_bytes, done) => {
    second.controller.input(answer);
    done();
  });
  void second.controller.connect();
  await flush();
  expect(
    wire.requests.filter((operation) => operation.operation === "write"),
  ).toHaveLength(1);
  second.controller.dispose();
});

it("asks which existing shell to use when a fresh browser has several owned sessions", async () => {
  session.controller_id = "another-tab";
  const original = wire.handler!;
  wire.handler = async (operation) =>
    operation.operation === "context"
      ? reply({ sessions: [session, { ...session, terminal_id: "second" }] })
      : original(operation);
  const { controller, states } = setup();
  void controller.connect();
  await flush();
  expect(states.at(-1)?.phase).toBe("choose");
  expect(states.at(-1)?.sessions).toHaveLength(2);
  expect(wire.requests).toEqual([{ operation: "context" }]);
  void controller.selectSession("shell");
  await flush();
  expect(states.at(-1)?.phase).toBe("takeover");
  expect(states.at(-1)?.session?.terminal_id).toBe("shell");
  expect(
    wire.requests.some(
      (operation) =>
        operation.operation === "open" || operation.operation === "attach",
    ),
  ).toBe(false);
  controller.dispose();
});

it("starts a new shell from a fresh browser when the retained sessions have all exited", async () => {
  const original = wire.handler!;
  wire.handler = async (operation) =>
    operation.operation === "context"
      ? reply({
          sessions: [
            {
              ...session,
              terminal_id: "ended",
              controller_id: "old-tab",
              status: "exited",
              exit_code: 0,
            },
          ],
        })
      : original(operation);
  const { controller, states } = setup();
  void controller.connect();
  await flush();
  expect(
    wire.requests.filter((operation) => operation.operation === "open"),
  ).toHaveLength(1);
  expect(
    wire.requests.some((operation) => operation.operation === "attach"),
  ).toBe(false);
  expect(states.at(-1)?.phase).toBe("live");
  controller.dispose();
});

it("splits a large Unicode paste into bounded sequenced writes and rejects overflow", async () => {
  const { controller } = setup();
  void controller.connect();
  await flush();
  const bytes = new TextEncoder().encode("界".repeat(30_000));
  controller.input(bytes);
  await flush();
  const writes = wire.requests.filter(
    (request) => request.operation === "write",
  );
  expect(writes.map((write) => write.input_sequence)).toEqual([1, 2]);
  const reconstructed = writes.map((write) => atob(write.data_base64)).join("");
  expect(Uint8Array.from(reconstructed, (char) => char.charCodeAt(0))).toEqual(
    bytes,
  );
  controller.input(new Uint8Array(128 * 1024 + 1));
  await flush();
  expect(
    wire.requests.filter((request) => request.operation === "write"),
  ).toHaveLength(2);
  controller.dispose();
});
