use base64::{Engine as _, engine::general_purpose::STANDARD};
use colossus_api::{HumanTerminalFrame, HumanTerminalFrameKind};
use std::collections::VecDeque;

pub(super) const RING_BYTES: usize = 2 * 1024 * 1024;

pub(super) struct Output {
    frames: VecDeque<(HumanTerminalFrame, usize)>,
    bytes: usize,
    pub(super) sequence: u64,
}

impl Output {
    pub(super) fn new(rows: u16, cols: u16) -> Self {
        let mut value = Self {
            frames: VecDeque::new(),
            bytes: 0,
            sequence: 0,
        };
        value.push(HumanTerminalFrameKind::Resize { rows, cols });
        value
    }
    pub(super) fn push(&mut self, kind: HumanTerminalFrameKind) {
        self.sequence += 1;
        let size = match &kind {
            HumanTerminalFrameKind::Output { data_base64 } => data_base64.len() + 64,
            HumanTerminalFrameKind::Resize { .. } => 64,
        };
        self.frames.push_back((
            HumanTerminalFrame {
                sequence: self.sequence,
                kind,
            },
            size,
        ));
        self.bytes += size;
        while self.bytes > RING_BYTES {
            if let Some((_, removed)) = self.frames.pop_front() {
                self.bytes -= removed;
            }
        }
    }
    pub(super) fn bytes(&mut self, value: &[u8]) {
        if !value.is_empty() {
            self.push(HumanTerminalFrameKind::Output {
                data_base64: STANDARD.encode(value),
            });
        }
    }
    pub(super) fn first(&self) -> u64 {
        self.frames
            .front()
            .map_or(self.sequence + 1, |(v, _)| v.sequence)
    }
    pub(super) fn read(&self, after: u64, max: usize) -> (Vec<HumanTerminalFrame>, u64, bool) {
        let gap = after.saturating_add(1) < self.first() || after > self.sequence;
        if gap {
            return (Vec::new(), after, true);
        }
        let mut bytes = 0;
        let frames = self
            .frames
            .iter()
            .filter(|(v, _)| v.sequence > after)
            .take(256)
            .take_while(|(_, size)| {
                bytes += *size;
                bytes <= max
            })
            .map(|(v, _)| v.clone())
            .collect::<Vec<_>>();
        let next = frames.last().map_or(after, |v| v.sequence);
        (frames, next, false)
    }
}

/// Stateful OSC removal also covers sequences split between native reads.
#[derive(Default)]
pub(super) struct OscFilter {
    state: u8,
    utf8_remaining: u8,
}
impl OscFilter {
    pub(super) fn filter(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut result = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            if self.state == 0 && self.utf8_remaining != 0 {
                if byte & 0xc0 == 0x80 {
                    self.utf8_remaining -= 1;
                    result.push(byte);
                    continue;
                }
                self.utf8_remaining = 0;
            }
            match self.state {
                0 if byte == 0x1b => self.state = 1,
                0 if byte == 0x9d => self.state = 2,
                0 if byte == 0xc2 => self.state = 4,
                0 => {
                    self.utf8_remaining = match byte {
                        0xc3..=0xdf => 1,
                        0xe0..=0xef => 2,
                        0xf0..=0xf4 => 3,
                        _ => 0,
                    };
                    result.push(byte);
                }
                1 if byte == b']' => self.state = 2,
                1 if byte == 0x1b => {}
                1 => {
                    result.push(0x1b);
                    result.push(byte);
                    self.state = 0;
                }
                2 if byte == 7 || byte == 0x9c => self.state = 0,
                2 if byte == 0x1b => self.state = 3,
                2 => {}
                3 if byte == b'\\' || byte == 7 || byte == 0x9c => self.state = 0,
                3 if byte == 0x1b => {}
                3 => self.state = 2,
                4 if byte == 0x9d => self.state = 2,
                4 if byte & 0xc0 != 0x80 => {
                    result.push(0xc2);
                    self.state = 0;
                    result.extend(self.filter(&[byte]));
                }
                4 => {
                    result.push(0xc2);
                    result.push(byte);
                    self.state = 0;
                }
                _ => self.state = 0,
            }
        }
        result
    }
}
