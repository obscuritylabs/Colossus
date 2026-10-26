import type { Terminal } from "@xterm/xterm";
import type { HumanTerminalSession } from "@colossus/control-plane-client";
import {
  FleetTerminalConnection,
  FleetTerminalFailure,
} from "./fleet-terminal-connection";
import {
  MAX_PENDING_INPUT,
  MAX_TERMINAL_INPUT,
  terminalBase64,
  terminalBytes,
  terminalSequence,
  validateTerminalFrames,
} from "./fleet-terminal-model";

interface SavedTerminal {
  request_id: string;
  controller_id: string;
  terminal_id?: string;
  incarnation?: string;
  new_requested?: boolean;
  open_pending?: boolean;
}
export interface FleetTerminalState {
  phase:
    | "connecting"
    | "live"
    | "disconnected"
    | "history_gap"
    | "exited"
    | "error"
    | "takeover"
    | "choose";
  message: string;
  controlling: boolean;
  session: HumanTerminalSession | null;
  sessions: HumanTerminalSession[];
}
const initialState: FleetTerminalState = {
  phase: "connecting",
  message: "Connecting to workspace…",
  controlling: false,
  session: null,
  sessions: [],
};

/** Process identity and display cursor survive a socket drop, never shell input. */
export class FleetTerminalController {
  private connection: FleetTerminalConnection | null = null;
  private generation = 0;
  private disposed = false;
  private cursor = 0;
  private replayUntil = 0;
  private inputSequence = 0;
  private pendingInputBytes = 0;
  private queuedInputBytes = 0;
  private inputQueue: Uint8Array[] = [];
  private writingInput = false;
  private resizePending = false;
  private pendingSize: { rows: number; cols: number } | null = null;
  private interruptPending = false;
  private parsing = Promise.resolve();
  private finishParse: (() => void) | null = null;
  private saved: SavedTerminal;
  private state = initialState;
  private storageKey: string;

  constructor(
    private terminal: Terminal,
    private project: string,
    private node: string,
    scope: string,
    private changed: (state: FleetTerminalState) => void,
  ) {
    this.storageKey = `colossus:fleet-shell:${scope}`;
    this.saved = this.load();
  }

  async connect(takeover = false) {
    const generation = ++this.generation;
    this.connection?.close();
    this.discardInput();
    this.pendingSize = null;
    this.resizePending = false;
    this.interruptPending = false;
    this.update({
      phase: "connecting",
      controlling: false,
      message: "Connecting to workspace…",
    });
    try {
      // A dropped socket cannot undo bytes already queued in this emulator.
      // Finish parsing them before selecting the reconnect replay cursor.
      await this.parsing;
      if (!this.current(generation)) return;
      const connection = new FleetTerminalConnection(
        this.project,
        this.node,
        () => {
          if (!this.current(generation)) return;
          this.generation++;
          this.discardInput();
          this.update({
            phase: "disconnected",
            controlling: false,
            message:
              "Disconnected. Reconnect to the same shell. Unconfirmed input will not be sent again.",
          });
        },
      );
      this.connection = connection;
      const context = await connection.request({ operation: "context" });
      if (!this.current(generation)) return;
      if (!context.available)
        throw new Error("Human shells are disabled on this workspace.");
      if (
        this.saved.incarnation &&
        context.incarnation !== this.saved.incarnation
      )
        throw new Error(
          "The workspace restarted. The previous shell ended; open a new shell to continue.",
        );
      if (!this.saved.incarnation) {
        // An Open reply can be lost after the process starts. Fence that
        // uncertain request to this worker before sending it.
        this.saved.incarnation = context.incarnation;
        this.save();
      }
      this.update({ sessions: context.sessions });
      if (!this.saved.terminal_id) {
        const running = context.sessions.filter(
          (session) => session.status === "running",
        );
        const owned = (
          this.saved.open_pending ? context.sessions : running
        ).find((session) => session.controller_id === this.saved.controller_id);
        if (this.saved.open_pending && !owned)
          throw new Error(
            "The earlier shell open could not be confirmed. Open a new shell to continue; the earlier request will not be sent again.",
          );
        if (!this.saved.new_requested && !owned && running.length > 1) {
          this.update({
            phase: "choose",
            controlling: false,
            session: null,
            message: "Choose an existing shell or open a new one.",
          });
          return;
        }
        const previous =
          owned ?? (!this.saved.new_requested ? running.at(-1) : undefined);
        if (previous) {
          this.saved = {
            ...this.saved,
            terminal_id: previous.terminal_id,
            incarnation: context.incarnation,
          };
          this.save();
        }
      }
      const previous = context.sessions.find(
        (session) => session.terminal_id === this.saved.terminal_id,
      );
      if (
        previous &&
        previous.status === "running" &&
        previous.controller_id !== this.saved.controller_id &&
        !takeover
      ) {
        this.update({
          phase: "takeover",
          session: previous,
          controlling: false,
          message:
            "Another tab controls this shell. Take control to attach here.",
        });
        return;
      }
      const opening = !this.saved.terminal_id;
      if (opening) {
        this.saved.new_requested = false;
        this.saved.open_pending = true;
        this.save();
      }
      const reply = await connection.request(
        this.saved.terminal_id
          ? {
              operation: "attach",
              terminal_id: this.saved.terminal_id,
              controller_id: this.saved.controller_id,
              takeover,
            }
          : {
              operation: "open",
              request_id: this.saved.request_id,
              controller_id: this.saved.controller_id,
              rows: this.terminal.rows,
              cols: this.terminal.cols,
            },
      );
      if (!this.current(generation)) return;
      if (!reply.session)
        throw new Error("The workspace did not return a shell session.");
      const selected = reply.session;
      this.saved = {
        ...this.saved,
        terminal_id: reply.session.terminal_id,
        incarnation: reply.incarnation,
        new_requested: false,
        open_pending: false,
      };
      this.save();
      this.inputSequence = terminalSequence(reply.input_sequence ?? 0);
      this.replayUntil = opening
        ? 0
        : terminalSequence(reply.session.next_sequence);
      this.update({
        session: reply.session,
        sessions: context.sessions.some(
          (session) => session.terminal_id === selected.terminal_id,
        )
          ? context.sessions.map((session) =>
              session.terminal_id === selected.terminal_id ? selected : session,
            )
          : [...context.sessions, selected],
        controlling: false,
        message: "Restoring terminal output…",
      });
      await this.read(connection, generation);
    } catch (error) {
      if (this.current(generation)) {
        if (
          error instanceof FleetTerminalFailure &&
          error.code === "conflict" &&
          this.saved.terminal_id
        )
          this.update({
            phase: "takeover",
            controlling: false,
            message: "Shell control changed. Take control to attach here.",
          });
        else this.fail(error);
      }
    }
  }

  private async read(connection: FleetTerminalConnection, generation: number) {
    while (this.current(generation)) {
      const session = this.state.session;
      if (!session) return;
      const reply = await connection.request({
        operation: "read",
        terminal_id: session.terminal_id,
        after_sequence: this.cursor,
        max_bytes: 65536,
        wait_ms: 1000,
      });
      if (!this.current(generation)) return;
      if (reply.incarnation !== this.saved.incarnation)
        throw new Error("The workspace restarted; this shell has ended.");
      if (reply.gap) {
        this.update({
          phase: "history_gap",
          controlling: session.controller_id === this.saved.controller_id,
          message:
            "Earlier output is no longer retained. This terminal cannot safely reconstruct the screen. Close this shell or open a new one.",
        });
        return;
      }
      const next = validateTerminalFrames(reply.frames, this.cursor);
      if (terminalSequence(reply.next_sequence) !== next)
        throw new Error("The terminal output cursor did not match its frames.");
      for (const frame of reply.frames) {
        // Replayed device queries must not inject answers into a later prompt.
        // New output beyond the attach watermark can drive ordinary VT replies.
        if (
          frame.sequence > this.replayUntil &&
          this.state.phase !== "live" &&
          (reply.session ?? session).status === "running"
        )
          this.live(reply.session ?? session);
        if (frame.kind.kind === "resize")
          this.terminal.resize(frame.kind.cols, frame.kind.rows);
        else {
          const bytes = terminalBytes(frame.kind.data_base64);
          this.parsing = new Promise<void>((resolve) => {
            this.finishParse = resolve;
            this.terminal.write(bytes, () => {
              this.finishParse = null;
              resolve();
            });
          });
          await this.parsing;
        }
        if (!this.disposed && this.saved.terminal_id === session.terminal_id)
          this.cursor = frame.sequence;
        if (!this.current(generation)) return;
      }
      const latest = reply.session ?? session;
      if (next < terminalSequence(latest.next_sequence)) continue;
      if (latest.status !== "running") {
        this.update({
          phase: "exited",
          controlling: false,
          session: latest,
          message:
            latest.status === "exited"
              ? `Shell exited${latest.exit_code === null ? "" : ` (${latest.exit_code})`}.`
              : "The shell was interrupted.",
        });
        return;
      }
      this.live(latest);
    }
  }

  private live(latest: HumanTerminalSession) {
    this.update({
      phase: "live",
      session: latest,
      controlling: latest.controller_id === this.saved.controller_id,
      message:
        latest.controller_id === this.saved.controller_id
          ? "Connected"
          : "View only · another tab controls this shell",
    });
  }

  input(bytes: Uint8Array) {
    if (!this.state.controlling || this.state.phase !== "live" || !bytes.length)
      return;
    if (
      bytes.length > MAX_PENDING_INPUT ||
      this.pendingInputBytes + bytes.length > MAX_PENDING_INPUT
    ) {
      this.update({
        message:
          "Input was not sent: paste at most 128 KiB and wait for pending input.",
      });
      return;
    }
    this.pendingInputBytes += bytes.length;
    this.queuedInputBytes += bytes.length;
    this.inputQueue.push(bytes.slice());
    if (!this.writingInput) void this.writeInput(this.generation);
  }

  private async writeInput(generation: number) {
    this.writingInput = true;
    try {
      while (
        this.current(generation) &&
        this.state.controlling &&
        this.state.phase === "live" &&
        this.connection &&
        this.state.session &&
        this.queuedInputBytes
      ) {
        // Send the first event immediately, then combine all bytes queued
        // behind its acknowledgement into bounded, ordered writes.
        const batch = new Uint8Array(
          Math.min(this.queuedInputBytes, MAX_TERMINAL_INPUT),
        );
        let consumed = 0;
        let offset = 0;
        while (offset < batch.length) {
          const bytes = this.inputQueue[consumed]!;
          const size = Math.min(bytes.length, batch.length - offset);
          batch.set(bytes.subarray(0, size), offset);
          offset += size;
          if (size === bytes.length) consumed++;
          else this.inputQueue[consumed] = bytes.subarray(size);
        }
        this.inputQueue.splice(0, consumed);
        this.queuedInputBytes -= batch.length;
        const inputSequence = ++this.inputSequence;
        const reply = await this.connection.request({
          operation: "write",
          terminal_id: this.state.session.terminal_id,
          writer_epoch: this.state.session.writer_epoch,
          input_sequence: inputSequence,
          data_base64: terminalBase64(batch),
        });
        if (!this.current(generation)) return;
        if (reply.input_sequence !== inputSequence)
          throw new Error(
            "Shell input was not confirmed. Reconnect to inspect its state.",
          );
        this.pendingInputBytes -= batch.length;
      }
    } catch (error) {
      if (this.current(generation)) this.fail(error);
    } finally {
      if (this.current(generation)) this.discardInput();
    }
  }

  private discardInput() {
    this.pendingInputBytes = 0;
    this.queuedInputBytes = 0;
    this.inputQueue = [];
    this.writingInput = false;
  }

  async resize(rows: number, cols: number) {
    if (!this.state.controlling || this.state.phase !== "live") return;
    this.pendingSize = {
      rows: Math.max(2, Math.min(512, rows)),
      cols: Math.max(2, Math.min(512, cols)),
    };
    if (this.resizePending) return;
    this.resizePending = true;
    const generation = this.generation;
    try {
      while (
        this.current(generation) &&
        this.pendingSize &&
        this.state.session &&
        this.connection
      ) {
        const size = this.pendingSize;
        this.pendingSize = null;
        await this.connection.request({
          operation: "resize",
          terminal_id: this.state.session.terminal_id,
          writer_epoch: this.state.session.writer_epoch,
          ...size,
        });
      }
    } catch (error) {
      if (this.current(generation)) this.fail(error);
    } finally {
      if (this.current(generation)) this.resizePending = false;
    }
  }

  async interrupt() {
    const session = this.state.session;
    if (
      !this.state.controlling ||
      !session ||
      !this.connection ||
      this.interruptPending
    )
      return;
    const generation = this.generation;
    this.interruptPending = true;
    try {
      await this.connection.request({
        operation: "signal",
        terminal_id: session.terminal_id,
        writer_epoch: session.writer_epoch,
        signal: "interrupt",
      });
    } catch (error) {
      if (this.current(generation)) this.fail(error);
    } finally {
      if (this.current(generation)) this.interruptPending = false;
    }
  }

  async closeShell() {
    const session = this.state.session;
    if (
      !session ||
      session.controller_id !== this.saved.controller_id ||
      !this.connection
    )
      return false;
    this.generation++;
    this.discardInput();
    this.pendingSize = null;
    this.update({ controlling: false, message: "Closing shell…" });
    try {
      await this.connection.request({
        operation: "close",
        terminal_id: session.terminal_id,
        writer_epoch: session.writer_epoch,
      });
      this.forget();
      this.dispose();
      return true;
    } catch (error) {
      if (!this.disposed) this.fail(error);
      return false;
    }
  }

  async startNew() {
    this.generation++;
    this.discardInput();
    this.update({
      controlling: false,
      phase: "connecting",
      message: "Opening a new shell…",
    });
    await this.parsing;
    if (this.disposed) return;
    this.cursor = 0;
    this.saved = { ...this.fresh(), new_requested: true };
    this.save();
    this.terminal.reset();
    this.update({ ...initialState });
    void this.connect();
  }

  async selectSession(terminalId: string) {
    const session = this.state.sessions.find(
      (value) => value.terminal_id === terminalId,
    );
    if (!session) return;
    this.generation++;
    this.discardInput();
    this.update({
      phase: "connecting",
      controlling: false,
      session,
      message: "Attaching to shell…",
    });
    await this.parsing;
    if (this.disposed) return;
    this.cursor = 0;
    this.terminal.reset();
    this.saved = {
      ...this.saved,
      terminal_id: terminalId,
      new_requested: false,
      open_pending: false,
    };
    this.save();
    void this.connect();
  }

  dispose() {
    this.disposed = true;
    this.generation++;
    this.discardInput();
    this.finishParse?.();
    this.finishParse = null;
    this.connection?.close();
  }
  private current(generation: number) {
    return !this.disposed && this.generation === generation;
  }
  private update(patch: Partial<FleetTerminalState>) {
    if (this.disposed) return;
    this.state = { ...this.state, ...patch };
    this.changed(this.state);
  }
  private fail(error: unknown) {
    this.generation++;
    this.discardInput();
    this.connection?.close();
    this.update({
      phase: "error",
      controlling: false,
      message:
        error instanceof Error
          ? error.message
          : "The shell operation could not be completed.",
    });
  }
  private fresh(): SavedTerminal {
    return {
      request_id: crypto.randomUUID(),
      controller_id: crypto.randomUUID(),
    };
  }
  private load(): SavedTerminal {
    try {
      const raw = sessionStorage.getItem(this.storageKey);
      if (raw && raw.length < 1024) {
        const value = JSON.parse(raw) as SavedTerminal;
        const id = (input: unknown) =>
          typeof input === "string" && /^[A-Za-z0-9_:.-]{1,128}$/u.test(input);
        if (
          id(value.request_id) &&
          id(value.controller_id) &&
          (!value.terminal_id || id(value.terminal_id)) &&
          (!value.incarnation || id(value.incarnation))
        )
          return value;
      }
    } catch {
      /* A storage restriction must not prevent a live shell. */
    }
    const value = this.fresh();
    try {
      sessionStorage.setItem(this.storageKey, JSON.stringify(value));
    } catch {
      /* Best effort process reference, no authority. */
    }
    return value;
  }
  private save() {
    try {
      sessionStorage.setItem(this.storageKey, JSON.stringify(this.saved));
    } catch {
      /* Best effort process reference. */
    }
  }
  private forget() {
    try {
      sessionStorage.removeItem(this.storageKey);
    } catch {
      /* Native close is authoritative. */
    }
  }
}
