use super::*;

pub(super) struct OwnedTerminal {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    mode: ScreenMode,
    inline_area: Option<Rect>,
    inline_screen_size: Option<Size>,
    inline_completion_screen_size: Option<Size>,
    committed_entries: usize,
    committed_epoch: u64,
    has_native_history: bool,
    _guard: TerminalGuard,
}

impl OwnedTerminal {
    pub(super) fn new(mode: ScreenMode) -> Result<Self, io::Error> {
        let guard = TerminalGuard::enter(mode)?;
        let mut backend = CrosstermBackend::new(io::stdout());
        let (terminal, inline_area, inline_screen_size) = match mode {
            ScreenMode::Alternate => (Terminal::new(backend)?, None, None),
            ScreenMode::Inline => {
                let (area, screen_size) =
                    initial_inline_area(&mut backend, MINIMUM_INLINE_VIEWPORT_HEIGHT)?;
                let terminal = Terminal::with_options(
                    backend,
                    TerminalOptions {
                        viewport: Viewport::Fixed(area),
                    },
                )?;
                (terminal, Some(area), Some(screen_size))
            }
        };
        Ok(Self {
            terminal,
            mode,
            inline_area,
            inline_screen_size,
            inline_completion_screen_size: None,
            committed_entries: 0,
            committed_epoch: u64::MAX,
            has_native_history: false,
            _guard: guard,
        })
    }

    pub(super) fn draw(&mut self, state: &mut TuiState) -> Result<(), io::Error> {
        if self.mode == ScreenMode::Alternate {
            self.terminal
                .draw(|frame| render(frame, state, 0, ScreenMode::Alternate))?;
            return Ok(());
        }

        self.synchronize_native_history_progress(state);
        if state.structured_completion_context().is_some() {
            self.draw_inline_completion(state)?;
            return Ok(());
        }
        self.leave_inline_completion_screen()?;
        let transcript_start = if state.has_more || state.loading_older {
            self.committed_entries
        } else {
            committable_transcript_end(&state.transcript, self.committed_entries)
        };
        let screen_size = self.terminal.backend().size()?;
        state.transcript_width = usize::from(screen_size.width).max(20);
        let viewport_height = desired_inline_viewport_height(
            state,
            screen_size.width,
            screen_size.height,
            transcript_start,
        );
        self.resize_inline_viewport(screen_size, viewport_height)?;
        self.commit_native_history(state, transcript_start)?;
        self.terminal
            .draw(|frame| render(frame, state, transcript_start, ScreenMode::Inline))?;
        Ok(())
    }

    fn draw_inline_completion(&mut self, state: &mut TuiState) -> Result<(), io::Error> {
        if !self._guard.transient_alternate_screen {
            Backend::flush(self.terminal.backend_mut())?;
            self._guard.enter_transient_alternate_screen()?;
            self.inline_completion_screen_size = None;
        }
        let screen_size = self.terminal.backend().size()?;
        if self.inline_completion_screen_size != Some(screen_size) {
            self.terminal
                .resize(Rect::new(0, 0, screen_size.width, screen_size.height))?;
            self.inline_completion_screen_size = Some(screen_size);
        }
        state.transcript_width = usize::from(screen_size.width).max(20);
        self.terminal
            .draw(|frame| render(frame, state, 0, ScreenMode::Alternate))?;
        Ok(())
    }

    fn leave_inline_completion_screen(&mut self) -> Result<(), io::Error> {
        if !self._guard.transient_alternate_screen {
            return Ok(());
        }
        Backend::flush(self.terminal.backend_mut())?;
        self._guard.leave_transient_alternate_screen()?;
        self.inline_completion_screen_size = None;

        // Leaving the alternate screen restores the main screen byte-for-byte.
        // Re-establish only the app-owned bottom viewport; the next normal draw
        // may then resize it for live transcript or activity without touching
        // the terminal rows that completion temporarily covered.
        let screen_size = self.terminal.backend().size()?;
        let current = self.inline_area.expect("inline viewport area");
        let previous_screen = self.inline_screen_size.expect("inline terminal size");
        let (restored, _) = next_inline_area(current, previous_screen, screen_size, current.height);
        self.terminal.resize(restored)?;
        self.inline_area = Some(restored);
        self.inline_screen_size = Some(screen_size);
        Ok(())
    }

    fn synchronize_native_history_progress(&mut self, state: &TuiState) {
        if self.committed_epoch != state.transcript_epoch
            || self.committed_entries > state.transcript.len()
        {
            self.committed_entries = 0;
            self.committed_epoch = state.transcript_epoch;
            self.has_native_history = false;
        }
    }

    fn resize_inline_viewport(&mut self, screen_size: Size, height: u16) -> Result<(), io::Error> {
        let current = self.inline_area.expect("inline viewport area");
        let previous_screen = self.inline_screen_size.expect("inline terminal size");
        let (next, scroll_up) = next_inline_area(current, previous_screen, screen_size, height);
        if next == current && screen_size == previous_screen {
            return Ok(());
        }

        clear_backend_rows(self.terminal.backend_mut(), current, screen_size.height)?;
        if scroll_up > 0 {
            scroll_screen_up(self.terminal.backend_mut(), screen_size.height, scroll_up)?;
        }
        self.terminal.resize(next)?;
        self.inline_area = Some(next);
        self.inline_screen_size = Some(screen_size);
        Ok(())
    }

    fn commit_native_history(&mut self, state: &mut TuiState, end: usize) -> Result<(), io::Error> {
        if end == self.committed_entries {
            return Ok(());
        }

        let screen_size = self.inline_screen_size.expect("inline terminal size");
        let width = usize::from(screen_size.width).max(20);
        state.transcript_width = width;
        let mut lines = transcript_lines_range(
            state,
            width,
            self.committed_entries,
            end,
            self.has_native_history,
        );
        while lines
            .last()
            .is_some_and(|line| line.to_string().trim().is_empty())
        {
            lines.pop();
        }
        let mut area = self.inline_area.expect("inline viewport area");
        for chunk in lines.chunks(HISTORY_INSERT_CHUNK_LINES) {
            let rendered = chunk.to_vec();
            let height = u16::try_from(rendered.len()).unwrap_or(u16::MAX);
            let mut buffer = Buffer::empty(Rect::new(0, 0, screen_size.width, height));
            let buffer_area = buffer.area;
            Paragraph::new(rendered).render(buffer_area, &mut buffer);
            insert_history_buffer(self.terminal.backend_mut(), &buffer, &mut area, screen_size)?;
        }
        self.terminal.resize(area)?;
        self.inline_area = Some(area);
        self.committed_entries = end;
        self.has_native_history |= !lines.is_empty();
        Ok(())
    }
}

fn initial_inline_area<B: Backend<Error = io::Error>>(
    backend: &mut B,
    requested_height: u16,
) -> Result<(Rect, Size), io::Error> {
    let screen_size = backend.size()?;
    let cursor = backend.get_cursor_position()?;
    let height = requested_height.min(screen_size.height);
    let lines_after_cursor = height.saturating_sub(1);
    backend.append_lines(lines_after_cursor)?;
    let available_lines = screen_size
        .height
        .saturating_sub(cursor.y)
        .saturating_sub(1);
    let rows_scrolled = lines_after_cursor.saturating_sub(available_lines);
    let y = cursor.y.saturating_sub(rows_scrolled);
    Ok((Rect::new(0, y, screen_size.width, height), screen_size))
}

pub(super) fn next_inline_area(
    current: Rect,
    previous_screen: Size,
    screen: Size,
    requested_height: u16,
) -> (Rect, u16) {
    let height = requested_height.min(screen.height);
    let was_bottom_anchored = current.bottom() >= previous_screen.height;
    let maximum_y = screen.height.saturating_sub(height);
    let y = if was_bottom_anchored {
        maximum_y
    } else {
        current.y.min(maximum_y)
    };
    (
        Rect::new(0, y, screen.width, height),
        current.y.saturating_sub(y),
    )
}

pub(super) fn clear_backend_rows<B: Backend>(
    backend: &mut B,
    area: Rect,
    screen_height: u16,
) -> Result<(), B::Error> {
    for y in area.top()..area.bottom().min(screen_height) {
        backend.set_cursor_position(Position::new(0, y))?;
        backend.clear_region(ClearType::CurrentLine)?;
    }
    Ok(())
}

pub(super) fn insert_history_buffer<B: Backend>(
    backend: &mut B,
    buffer: &Buffer,
    viewport: &mut Rect,
    screen_size: Size,
) -> Result<(), B::Error> {
    let mut first_row = 0_i32;
    let mut drawn_height = i32::from(viewport.top());
    let mut remaining = i32::from(buffer.area.height);
    let viewport_height = i32::from(viewport.height);
    let screen_height = i32::from(screen_size.height);

    while remaining + viewport_height > screen_height {
        let rows = remaining.min(screen_height);
        let scroll_up = 0.max(drawn_height + rows - screen_height);
        scroll_screen_up(
            backend,
            screen_size.height,
            u16::try_from(scroll_up).unwrap_or(u16::MAX),
        )?;
        draw_buffer_rows(
            backend,
            buffer,
            u16::try_from(first_row).unwrap_or(u16::MAX),
            u16::try_from(rows).unwrap_or(u16::MAX),
            u16::try_from(drawn_height - scroll_up).unwrap_or_default(),
        )?;
        first_row += rows;
        drawn_height += rows - scroll_up;
        remaining -= rows;
    }

    let scroll_up = 0.max(drawn_height + remaining + viewport_height - screen_height);
    scroll_screen_up(
        backend,
        screen_size.height,
        u16::try_from(scroll_up).unwrap_or(u16::MAX),
    )?;
    draw_buffer_rows(
        backend,
        buffer,
        u16::try_from(first_row).unwrap_or(u16::MAX),
        u16::try_from(remaining).unwrap_or(u16::MAX),
        u16::try_from(drawn_height - scroll_up).unwrap_or_default(),
    )?;
    viewport.y = u16::try_from(drawn_height + remaining - scroll_up).unwrap_or_default();
    Ok(())
}

pub(super) fn scroll_screen_up<B: Backend>(
    backend: &mut B,
    screen_height: u16,
    rows: u16,
) -> Result<(), B::Error> {
    if rows == 0 {
        return Ok(());
    }
    backend.set_cursor_position(Position::new(0, screen_height.saturating_sub(1)))?;
    backend.append_lines(rows)
}

fn draw_buffer_rows<B: Backend>(
    backend: &mut B,
    buffer: &Buffer,
    first_row: u16,
    row_count: u16,
    destination_y: u16,
) -> Result<(), B::Error> {
    if row_count == 0 {
        return Ok(());
    }
    let width = usize::from(buffer.area.width);
    let start = usize::from(first_row) * width;
    let end = start + usize::from(row_count) * width;
    let cells = &buffer.content()[start..end];
    backend.draw(
        cells
            .iter()
            .enumerate()
            .map(|(index, cell): (usize, &Cell)| {
                (
                    u16::try_from(index % width).unwrap_or(u16::MAX),
                    destination_y + u16::try_from(index / width).unwrap_or(u16::MAX),
                    cell,
                )
            }),
    )?;
    backend.flush()
}

pub(super) fn committable_transcript_end(
    transcript: &[TranscriptEntry],
    committed_entries: usize,
) -> usize {
    let committed_entries = committed_entries.min(transcript.len());
    transcript[committed_entries..]
        .iter()
        .position(|entry| entry.temporary)
        .map_or(transcript.len(), |offset| committed_entries + offset)
}

struct TerminalGuard {
    mode: ScreenMode,
    transient_alternate_screen: bool,
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
        Ok(Self {
            mode,
            transient_alternate_screen: false,
        })
    }

    fn enter_transient_alternate_screen(&mut self) -> Result<(), io::Error> {
        debug_assert_eq!(self.mode, ScreenMode::Inline);
        if self.transient_alternate_screen {
            return Ok(());
        }
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture) {
            let _ = execute!(stdout, DisableMouseCapture, LeaveAlternateScreen);
            return Err(error);
        }
        self.transient_alternate_screen = true;
        Ok(())
    }

    fn leave_transient_alternate_screen(&mut self) -> Result<(), io::Error> {
        if !self.transient_alternate_screen {
            return Ok(());
        }
        let mut stdout = io::stdout();
        execute!(stdout, DisableMouseCapture, LeaveAlternateScreen)?;
        self.transient_alternate_screen = false;
        Ok(())
    }

    fn restore(&mut self) {
        let mut stdout = io::stdout();
        if self.mode == ScreenMode::Alternate || self.transient_alternate_screen {
            let _ = execute!(stdout, DisableMouseCapture, LeaveAlternateScreen);
        }
        self.transient_alternate_screen = false;
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
