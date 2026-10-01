//! Incremental UTF-8 and terminal-control filtering for released log bytes.
#[derive(Default)]
pub(super) struct LogDecoder {
    pending: Vec<u8>,
    escape: u8,
}
impl LogDecoder {
    pub(super) fn push(&mut self, bytes: &[u8], final_output: bool) -> String {
        self.pending.extend_from_slice(bytes);
        let mut text = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(valid) => {
                    text.push_str(valid);
                    self.pending.clear();
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    text.push_str(
                        std::str::from_utf8(&self.pending[..valid])
                            .expect("validated UTF-8 prefix"),
                    );
                    self.pending.drain(..valid);
                    if let Some(length) = error.error_len() {
                        text.push('\u{fffd}');
                        self.pending.drain(..length);
                    } else {
                        if final_output {
                            text.push('\u{fffd}');
                            self.pending.clear();
                        }
                        break;
                    }
                }
            }
        }
        let mut display = String::new();
        for character in text.chars() {
            match self.escape {
                1 => {
                    self.escape = match character {
                        '[' => 2,
                        ']' => 3,
                        _ => 0,
                    }
                }
                2 => {
                    if ('@'..='~').contains(&character) {
                        self.escape = 0;
                    }
                }
                3 => {
                    if character == '\u{7}' {
                        self.escape = 0;
                    } else if character == '\u{1b}' {
                        self.escape = 4;
                    }
                }
                4 => self.escape = if character == '\\' { 0 } else { 3 },
                _ if character == '\u{1b}' => self.escape = 1,
                _ if character == '\r' => display.push('\n'),
                _ if character == '\n'
                    || character == '\t'
                    || !colossus_contracts::unsafe_command_display_character(character) =>
                {
                    display.push(character)
                }
                _ => {}
            }
        }
        display
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decoder_retains_utf8_and_escape_state_across_chunks() {
        let mut decoder = LogDecoder::default();
        assert_eq!(decoder.push(&[0xf0, 0x9f], false), "");
        assert_eq!(decoder.push(&[0x98, 0x80, 0x1b, b'['], false), "😀");
        assert_eq!(decoder.push(b"31mred\x1b]8;;https://evil", false), "red");
        assert_eq!(
            decoder.push(b"\x1b\\visible\x1b]8;;\x07\n", true),
            "visible\n"
        );
        assert_eq!(decoder.push(&[0xff, 0xe2], true), "��");
    }
}
