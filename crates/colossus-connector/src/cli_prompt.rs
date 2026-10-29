//! Invitation input belongs to the human terminal, never argv, logs, or reports.
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use std::io::{self, BufRead, IsTerminal, Read, Write};
use zeroize::{Zeroize, Zeroizing};

const TOKEN_BYTES: usize = 64;

pub(super) fn read_invitation() -> Result<Zeroizing<String>, Box<dyn std::error::Error>> {
    let token = if io::stdin().is_terminal() {
        read_terminal()
    } else {
        prompt()?;
        let mut token = Zeroizing::new(String::new());
        io::BufReader::new(io::stdin())
            .take((TOKEN_BYTES + 2) as u64)
            .read_line(&mut token)?;
        Ok(Zeroizing::new(token.trim().to_owned()))
    };
    eprintln!();
    let token = token?;
    if token.len() != TOKEN_BYTES || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid enrollment invitation".into());
    }
    Ok(token)
}

fn prompt() -> io::Result<()> {
    eprint!("One-use invitation token (hidden): ");
    io::stderr().flush()
}

struct HiddenTerminal;
impl Drop for HiddenTerminal {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

fn read_terminal() -> Result<Zeroizing<String>, Box<dyn std::error::Error>> {
    crossterm::terminal::enable_raw_mode()?;
    let _terminal = HiddenTerminal;
    // Advertise readiness only after echo is disabled; fast paste/PTY callers
    // must never race a visible prompt against the terminal mode transition.
    prompt()?;
    let mut token = Zeroizing::new(String::with_capacity(TOKEN_BYTES));
    loop {
        match crossterm::event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Enter => return Ok(token),
                KeyCode::Esc => return Err("invitation input cancelled".into()),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err("invitation input cancelled".into());
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    token.zeroize();
                }
                KeyCode::Backspace => {
                    token.pop();
                }
                KeyCode::Char(character)
                    if character.is_ascii_hexdigit()
                        && !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    if token.len() == TOKEN_BYTES {
                        return Err("invalid enrollment invitation".into());
                    }
                    token.push(character);
                }
                KeyCode::Char(_) => return Err("invalid enrollment invitation".into()),
                _ => {}
            },
            Event::Paste(value) => {
                let value = Zeroizing::new(value);
                let value = value.trim();
                if token.len().saturating_add(value.len()) > TOKEN_BYTES
                    || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    return Err("invalid enrollment invitation".into());
                }
                token.push_str(value);
            }
            _ => {}
        }
    }
}
