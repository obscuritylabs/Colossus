/** Human terminal control is separate from agent execution authority. */
export type HumanTerminalOperation =
  | { operation: "context" }
  | {
      operation: "open";
      request_id: string;
      controller_id: string;
      rows: number;
      cols: number;
    }
  | {
      operation: "attach";
      terminal_id: string;
      controller_id: string;
      takeover: boolean;
    }
  | {
      operation: "read";
      terminal_id: string;
      after_sequence: number;
      max_bytes: number;
      wait_ms: number;
    }
  | {
      operation: "write";
      terminal_id: string;
      writer_epoch: number;
      input_sequence: number;
      data_base64: string;
    }
  | {
      operation: "resize";
      terminal_id: string;
      writer_epoch: number;
      rows: number;
      cols: number;
    }
  | {
      operation: "signal";
      terminal_id: string;
      writer_epoch: number;
      signal: "interrupt" | "terminate";
    }
  | { operation: "close"; terminal_id: string; writer_epoch: number };

export interface HumanTerminalSession {
  terminal_id: string;
  writer_epoch: number;
  controller_id: string;
  rows: number;
  cols: number;
  status: "running" | "exited" | "interrupted";
  first_sequence: number;
  next_sequence: number;
  complete_history: boolean;
  exit_code: number | null;
}

export interface HumanTerminalFrame {
  sequence: number;
  kind:
    | { kind: "output"; data_base64: string }
    | { kind: "resize"; rows: number; cols: number };
}

export interface HumanTerminalReply {
  available: boolean;
  incarnation: string;
  sessions: HumanTerminalSession[];
  session: HumanTerminalSession | null;
  frames: HumanTerminalFrame[];
  next_sequence: number;
  gap: boolean;
  input_sequence: number | null;
}
