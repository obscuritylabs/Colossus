//! Inert browser-like display consumption and bounded native-fixture diagnostics.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use colossus_sdk::{
    HumanTerminalFrameKind, HumanTerminalReply, HumanTerminalSession, HumanTerminalStatus,
};

const DIAGNOSTIC_BYTES: usize = 8192;

pub(super) struct TerminalDisplay {
    parser: vt100::Parser,
    incarnation: String,
    session: HumanTerminalSession,
    cursor: u64,
    read_next: u64,
    reads: usize,
    phase: &'static str,
    marker: &'static str,
    phase_cursor: u64,
    write_epoch: u64,
    write_ack: Option<u64>,
    raw_tail: Vec<u8>,
}

impl TerminalDisplay {
    pub(super) fn new(incarnation: &str, session: &HumanTerminalSession) -> Self {
        Self {
            // No responses or synthetic input: only interpret the display bytes
            // and ordered resize events that the actual browser would receive.
            parser: vt100::Parser::new(session.rows, session.cols, 0),
            incarnation: incarnation.into(),
            session: session.clone(),
            cursor: 0,
            read_next: 0,
            reads: 0,
            phase: "before shell input",
            marker: "",
            phase_cursor: 0,
            write_epoch: 0,
            write_ack: None,
            raw_tail: Vec::new(),
        }
    }

    pub(super) fn begin_phase(&mut self, phase: &'static str, marker: &'static str) {
        self.phase = phase;
        self.marker = marker;
        self.phase_cursor = self.cursor;
        self.reads = 0;
        self.write_epoch = 0;
        self.write_ack = None;
        assert!(
            !self.marker_visible(),
            "a new shell phase must not match stale display content: {}",
            self.diagnostics()
        );
    }

    pub(super) fn cursor(&self) -> u64 {
        self.cursor
    }

    pub(super) fn record_write(&mut self, reply: &HumanTerminalReply, epoch: u64) {
        self.write_epoch = epoch;
        self.write_ack = reply.input_sequence;
        assert!(
            self.write_ack.is_some(),
            "shell input must be acknowledged: {}",
            self.diagnostics()
        );
    }

    pub(super) fn observe(&mut self, reply: &HumanTerminalReply) {
        self.reads += 1;
        self.read_next = reply.next_sequence;
        let Some(session) = &reply.session else {
            panic!("selected shell metadata missing: {}", self.diagnostics());
        };
        assert_eq!(session.terminal_id, self.session.terminal_id);
        self.session = session.clone();
        assert!(
            reply.available && reply.incarnation == self.incarnation && !reply.gap,
            "same live owner and complete retained display history expected: {}",
            self.diagnostics()
        );
        for frame in &reply.frames {
            assert_eq!(
                frame.sequence,
                self.cursor + 1,
                "display frames must remain contiguous: {}",
                self.diagnostics()
            );
            match &frame.kind {
                HumanTerminalFrameKind::Output { data_base64 } => {
                    let bytes = STANDARD.decode(data_base64).expect("valid terminal output");
                    self.parser.process(&bytes);
                    self.raw_tail.extend_from_slice(&bytes);
                    let discard = self.raw_tail.len().saturating_sub(DIAGNOSTIC_BYTES);
                    self.raw_tail.drain(..discard);
                }
                HumanTerminalFrameKind::Resize { rows, cols } => {
                    self.parser.screen_mut().set_size(*rows, *cols);
                }
            }
            self.cursor = frame.sequence;
        }
        assert!(
            self.read_next == self.cursor && self.session.next_sequence >= self.cursor,
            "read must acknowledge the exact exclusive display cursor: {}",
            self.diagnostics()
        );
        assert_eq!(
            self.session.status,
            HumanTerminalStatus::Running,
            "shell ended before the requested live-session display: {}",
            self.diagnostics()
        );
    }

    pub(super) fn marker_visible(&self) -> bool {
        self.parser.screen().contents().contains(self.marker)
    }

    pub(super) fn diagnostics(&self) -> String {
        format!(
            "phase={:?} marker={:?} phase_cursor={} cursor={} read_next={} reads={} \
             status={:?} exit_code={:?} first_sequence={} session_next={} complete_history={} \
             display_size={:?} write_epoch={} write_ack={:?}\nraw_tail={}\nrendered={}",
            self.phase,
            self.marker,
            self.phase_cursor,
            self.cursor,
            self.read_next,
            self.reads,
            self.session.status,
            self.session.exit_code,
            self.session.first_sequence,
            self.session.next_sequence,
            self.session.complete_history,
            self.parser.screen().size(),
            self.write_epoch,
            self.write_ack,
            escaped_tail(&self.raw_tail),
            escaped_tail(self.parser.screen().contents().as_bytes()),
        )
    }
}

fn escaped_tail(bytes: &[u8]) -> String {
    let escaped = bytes.escape_ascii().to_string();
    // ASCII escaping is safe to slice and keeps both diagnostics independently
    // bounded to 8 KiB, including escapes for terminal controls and Unicode.
    escaped[escaped.len().saturating_sub(DIAGNOSTIC_BYTES)..].into()
}

#[test]
fn cursor_edits_and_resize_form_the_browser_display_across_read_boundaries() {
    use colossus_sdk::HumanTerminalFrame;
    let session = HumanTerminalSession {
        terminal_id: "fixture".into(),
        writer_epoch: 1,
        controller_id: "fixture".into(),
        rows: 24,
        cols: 80,
        status: HumanTerminalStatus::Running,
        first_sequence: 1,
        next_sequence: 0,
        complete_history: true,
        exit_code: None,
    };
    let mut display = TerminalDisplay::new("fixture-owner", &session);
    display.begin_phase("initial", "COLOSSUS-REMOTE-ONE");
    let mut reply = HumanTerminalReply {
        available: true,
        incarnation: "fixture-owner".into(),
        session: Some(session),
        ..Default::default()
    };
    let chunks: [&[u8]; 2] = [
        b"Write-Output ('COLOSSUS-' + 'REMOTE-ONE')\r\nCOLOSSUS-REMOTE-XXX",
        b"\x1b[3DONE\r\n",
    ];
    for (index, chunk) in chunks.iter().enumerate() {
        let sequence = index as u64 + 1;
        reply.frames = vec![HumanTerminalFrame {
            sequence,
            kind: HumanTerminalFrameKind::Output {
                data_base64: STANDARD.encode(chunk),
            },
        }];
        reply.next_sequence = sequence;
        reply.session.as_mut().unwrap().next_sequence = sequence;
        display.observe(&reply);
        assert_eq!(display.marker_visible(), index == 1);
    }
    assert!(!String::from_utf8_lossy(&display.raw_tail).contains("COLOSSUS-REMOTE-ONE"));
    display.begin_phase("reconnect", "COLOSSUS-STATE-kept");
    reply.frames = vec![
        HumanTerminalFrame {
            sequence: 3,
            kind: HumanTerminalFrameKind::Resize {
                rows: 31,
                cols: 101,
            },
        },
        HumanTerminalFrame {
            sequence: 4,
            kind: HumanTerminalFrameKind::Output {
                data_base64: STANDARD.encode(b"COLOSSUS-STATE-kept\r\n"),
            },
        },
    ];
    reply.next_sequence = 4;
    reply.session.as_mut().unwrap().next_sequence = 4;
    display.observe(&reply);
    assert_eq!(display.cursor(), 4);
    assert_eq!(display.parser.screen().size(), (31, 101));
    assert!(display.marker_visible());
}
