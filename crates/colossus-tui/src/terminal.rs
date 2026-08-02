use super::*;

pub(super) struct OwnedTerminal {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    _guard: TerminalGuard,
}

impl OwnedTerminal {
    pub(super) fn new(mode: ScreenMode) -> Result<Self, io::Error> {
        let guard = TerminalGuard::enter(mode)?;
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = match mode {
            ScreenMode::Alternate => Terminal::new(backend)?,
            ScreenMode::Inline => Terminal::with_options(
                backend,
                TerminalOptions {
                    viewport: Viewport::Inline(20),
                },
            )?,
        };
        Ok(Self {
            terminal,
            _guard: guard,
        })
    }

    pub(super) fn draw(&mut self, state: &mut TuiState) -> Result<(), io::Error> {
        self.terminal.draw(|frame| render(frame, state))?;
        Ok(())
    }
}

struct TerminalGuard {
    mode: ScreenMode,
}

impl TerminalGuard {
    fn enter(mode: ScreenMode) -> Result<Self, io::Error> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, Hide, EnableBracketedPaste) {
            let _ = disable_raw_mode();
            return Err(error);
        }
        if mode == ScreenMode::Alternate
            && let Err(error) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        {
            let _ = execute!(
                stdout,
                DisableMouseCapture,
                LeaveAlternateScreen,
                Show,
                DisableBracketedPaste
            );
            let _ = disable_raw_mode();
            return Err(error);
        }
        Ok(Self { mode })
    }

    fn restore(&self) {
        let mut stdout = io::stdout();
        if self.mode == ScreenMode::Alternate {
            let _ = execute!(stdout, DisableMouseCapture, LeaveAlternateScreen);
        }
        let _ = execute!(stdout, Show, DisableBracketedPaste);
        let _ = stdout.flush();
        let _ = disable_raw_mode();
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        self.restore();
    }
}
