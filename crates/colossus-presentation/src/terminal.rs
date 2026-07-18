use super::*;

/// Width-aware human terminal renderer for presentation documents.
pub struct TerminalDocumentRenderer {
    preferences: TerminalPreferences,
    width: usize,
    color: bool,
}

/// Width-aware presentation renderer that emits backend-neutral styled lines.
pub struct StyledDocumentRenderer {
    preferences: TerminalPreferences,
    width: usize,
    surface: StyledRenderSurface,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum StyledRenderSurface {
    #[default]
    Document,
    Transcript,
}

impl StyledDocumentRenderer {
    /// Build a renderer for one immutable preference snapshot and viewport width.
    pub fn new(preferences: TerminalPreferences, width: usize) -> Self {
        Self {
            preferences,
            width: width.clamp(20, 240),
            surface: StyledRenderSurface::Document,
        }
    }

    /// Build a renderer for a TUI transcript where semantic hierarchy is expressed with
    /// colored headings and indentation instead of recursively nested borders.
    pub fn for_transcript(preferences: TerminalPreferences, width: usize) -> Self {
        Self {
            preferences,
            width: width.clamp(20, 240),
            surface: StyledRenderSurface::Transcript,
        }
    }

    /// Render a retained semantic document for a terminal UI backend.
    pub fn render(&self, document: &PresentationDocument) -> Vec<StyledLine> {
        let renderer = TerminalDocumentRenderer::new(self.preferences.clone(), self.width);
        let palette = TerminalPalette::for_preferences(&self.preferences);
        if self.surface == StyledRenderSurface::Transcript {
            return self.render_transcript_document(document, &renderer, palette);
        }
        let mut lines = Vec::new();
        for block in &document.blocks {
            let rendered = renderer.render_block(block, self.width);
            if !lines.is_empty()
                && !rendered.is_empty()
                && lines
                    .last()
                    .is_some_and(|line: &StyledLine| !line.spans.is_empty())
                && !matches!(block, PresentationBlock::Blank)
            {
                lines.push(StyledLine::default());
            }
            let style = palette.style_for_block(block);
            lines.extend(rendered.into_iter().map(|line| {
                if line.is_empty() {
                    StyledLine::default()
                } else {
                    StyledLine::from_span(line, style)
                }
            }));
        }
        while lines.last().is_some_and(|line| line.spans.is_empty()) {
            lines.pop();
        }
        lines
    }

    fn render_transcript_document(
        &self,
        document: &PresentationDocument,
        renderer: &TerminalDocumentRenderer,
        palette: TerminalPalette,
    ) -> Vec<StyledLine> {
        let mut lines = Vec::new();
        for block in &document.blocks {
            let rendered = self.render_transcript_block(renderer, palette, block, self.width, None);
            if !lines.is_empty()
                && !rendered.is_empty()
                && lines
                    .last()
                    .is_some_and(|line: &StyledLine| !line.spans.is_empty())
                && !matches!(block, PresentationBlock::Blank)
            {
                lines.push(StyledLine::default());
            }
            lines.extend(rendered);
        }
        while lines.last().is_some_and(|line| line.spans.is_empty()) {
            lines.pop();
        }
        lines
    }

    fn render_transcript_block(
        &self,
        renderer: &TerminalDocumentRenderer,
        palette: TerminalPalette,
        block: &PresentationBlock,
        width: usize,
        inherited_accent: Option<ThemeTextStyle>,
    ) -> Vec<StyledLine> {
        match block {
            PresentationBlock::Card { title, tone, body } => {
                let accent = if *tone == PresentationTone::Neutral {
                    palette.section_style()
                } else {
                    palette.tone_style(*tone)
                };
                let mut title_style = accent;
                title_style.bold = true;
                title_style.dim = false;
                let marker = match tone {
                    PresentationTone::Neutral => "◆",
                    PresentationTone::Success => "✓",
                    PresentationTone::Warning => "!",
                    PresentationTone::Error => "×",
                    PresentationTone::Thinking => "…",
                    PresentationTone::Tool => "›",
                };
                let title = truncate_width(
                    &sanitize_terminal_text(title),
                    width.saturating_sub(2).max(1),
                );
                let mut lines = vec![StyledLine {
                    spans: vec![
                        StyledSpan {
                            content: format!("{marker} "),
                            style: accent,
                        },
                        StyledSpan {
                            content: title,
                            style: title_style,
                        },
                    ],
                }];
                for child in body {
                    if lines.len() > 1
                        && !matches!(child, PresentationBlock::Blank)
                        && lines.last().is_some_and(|line| !line.spans.is_empty())
                    {
                        lines.push(StyledLine::default());
                    }
                    let rendered = self.render_transcript_block(
                        renderer,
                        palette,
                        child,
                        width.saturating_sub(2).max(20),
                        Some(accent),
                    );
                    lines.extend(rendered.into_iter().map(|mut line| {
                        if !line.spans.is_empty() {
                            line.spans.insert(
                                0,
                                StyledSpan {
                                    content: "  ".into(),
                                    style: accent,
                                },
                            );
                        }
                        line
                    }));
                }
                lines
            }
            PresentationBlock::KeyValue(entries) => {
                self.render_transcript_key_values(entries, width, palette, inherited_accent)
            }
            PresentationBlock::Table(table) => {
                self.render_transcript_collection(table, width, palette, inherited_accent)
            }
            _ => {
                let style = palette.style_for_block(block);
                renderer
                    .render_block(block, width)
                    .into_iter()
                    .map(|line| {
                        if line.is_empty() {
                            StyledLine::default()
                        } else {
                            StyledLine::from_span(line, style)
                        }
                    })
                    .collect()
            }
        }
    }

    fn render_transcript_key_values(
        &self,
        entries: &[(String, String)],
        width: usize,
        palette: TerminalPalette,
        inherited_accent: Option<ThemeTextStyle>,
    ) -> Vec<StyledLine> {
        if entries.is_empty() {
            return vec![StyledLine::from_span("No details.", palette.meta_style())];
        }
        let maximum_label_width = entries
            .iter()
            .map(|(label, _)| display_width(&sanitize_terminal_text(label)))
            .max()
            .unwrap_or(1);
        let label_width = maximum_label_width.min((width / 3).clamp(8, 20));
        let value_width = width.saturating_sub(label_width + 2).max(8);
        let stacked = width < 36 || value_width < 12;
        let mut label_style = inherited_accent.unwrap_or_else(|| palette.meta_style());
        label_style.bold = true;
        label_style.dim = false;
        let value_style = palette.assistant_style();
        let mut lines = Vec::new();
        for (label, value) in entries {
            let label = sanitize_terminal_text(label);
            let value = sanitize_terminal_text(value);
            if stacked {
                lines.push(StyledLine::from_span(label, label_style));
                lines.extend(
                    wrap_text(&value, width.saturating_sub(2).max(8))
                        .into_iter()
                        .map(|value| StyledLine {
                            spans: vec![
                                StyledSpan {
                                    content: "  ".into(),
                                    style: label_style,
                                },
                                StyledSpan {
                                    content: value,
                                    style: value_style,
                                },
                            ],
                        }),
                );
                continue;
            }
            let label = truncate_width(&label, label_width);
            let padding = label_width.saturating_sub(display_width(&label));
            let values = wrap_text(&value, value_width);
            for (index, value) in values.into_iter().enumerate() {
                lines.push(StyledLine {
                    spans: vec![
                        StyledSpan {
                            content: if index == 0 {
                                format!("{label}{}", " ".repeat(padding))
                            } else {
                                " ".repeat(label_width)
                            },
                            style: label_style,
                        },
                        StyledSpan {
                            content: "  ".into(),
                            style: palette.meta_style(),
                        },
                        StyledSpan {
                            content: value,
                            style: value_style,
                        },
                    ],
                });
            }
        }
        lines
    }

    fn render_transcript_collection(
        &self,
        table: &PresentationTable,
        width: usize,
        palette: TerminalPalette,
        inherited_accent: Option<ThemeTextStyle>,
    ) -> Vec<StyledLine> {
        if table.rows.is_empty() {
            return wrap_text(&sanitize_terminal_text(&table.empty_message), width)
                .into_iter()
                .map(|line| StyledLine::from_span(line, palette.meta_style()))
                .collect();
        }
        let column_count = table
            .headers
            .len()
            .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
        let headers = (0..column_count)
            .map(|index| {
                table.headers.get(index).map_or_else(
                    || format!("Field {}", index + 1),
                    |header| sanitize_terminal_text(header),
                )
            })
            .collect::<Vec<_>>();
        let primary_index = collection_primary_index(&headers);
        let status_index = collection_status_index(&headers);
        let mut primary_style = inherited_accent.unwrap_or_else(|| palette.section_style());
        primary_style.bold = true;
        primary_style.dim = false;
        let meta_style = palette.meta_style();
        let metadata_style = palette.assistant_style();
        let mut lines = Vec::new();
        for (row_index, row) in table.rows.iter().enumerate() {
            let primary = row
                .get(primary_index)
                .map(|value| sanitize_terminal_text(value))
                .filter(|value| !value.trim().is_empty() && value != "—")
                .unwrap_or_else(|| format!("Item {}", row_index + 1));
            let status = status_index.and_then(|index| {
                row.get(index).and_then(|value| {
                    collection_status(&headers[index], &sanitize_terminal_text(value), palette)
                })
            });
            let status_width = status
                .as_ref()
                .map_or(0, |(label, _, _)| display_width(label) + 4);
            let primary = truncate_width(&primary, width.saturating_sub(status_width + 2).max(8));
            let mut spans = vec![
                StyledSpan {
                    content: "• ".into(),
                    style: inherited_accent.unwrap_or_else(|| palette.section_style()),
                },
                StyledSpan {
                    content: primary,
                    style: primary_style,
                },
            ];
            if let Some((label, marker, style)) = status {
                spans.extend([
                    StyledSpan {
                        content: "  ".into(),
                        style: meta_style,
                    },
                    StyledSpan {
                        content: format!("{marker} {label}"),
                        style,
                    },
                ]);
            }
            lines.push(StyledLine { spans });

            let metadata = row
                .iter()
                .enumerate()
                .filter(|(index, _)| {
                    *index != primary_index
                        && status_index != Some(*index)
                        && headers.get(*index).is_none_or(|header| header != "#")
                })
                .filter_map(|(index, value)| {
                    let value = sanitize_terminal_text(value);
                    (!value.trim().is_empty() && value != "—").then(|| {
                        format!(
                            "{}: {value}",
                            headers
                                .get(index)
                                .cloned()
                                .unwrap_or_else(|| format!("Field {}", index + 1))
                        )
                    })
                })
                .collect::<Vec<_>>()
                .join(" · ");
            if !metadata.is_empty() {
                lines.extend(
                    wrap_text(&metadata, width.saturating_sub(2).max(8))
                        .into_iter()
                        .map(|line| StyledLine {
                            spans: vec![
                                StyledSpan {
                                    content: "  ".into(),
                                    style: meta_style,
                                },
                                StyledSpan {
                                    content: line,
                                    style: metadata_style,
                                },
                            ],
                        }),
                );
            }
        }
        lines
    }
}

fn collection_primary_index(headers: &[String]) -> usize {
    for preferred in [
        "name", "title", "id", "path", "tool", "server", "model", "value",
    ] {
        if let Some(index) = headers
            .iter()
            .position(|header| header.eq_ignore_ascii_case(preferred))
        {
            return index;
        }
    }
    headers
        .iter()
        .position(|header| {
            !matches!(
                header.to_ascii_lowercase().as_str(),
                "status" | "active" | "enabled" | "trusted" | "state" | "#"
            )
        })
        .unwrap_or(0)
}

fn collection_status_index(headers: &[String]) -> Option<usize> {
    ["status", "state", "active", "enabled", "trusted"]
        .into_iter()
        .find_map(|preferred| {
            headers
                .iter()
                .position(|header| header.eq_ignore_ascii_case(preferred))
        })
}

fn collection_status(
    header: &str,
    value: &str,
    palette: TerminalPalette,
) -> Option<(String, &'static str, ThemeTextStyle)> {
    let value = value.trim();
    if value.is_empty() || value == "—" {
        return None;
    }
    let header = header.to_ascii_lowercase();
    let normalized = value.to_ascii_lowercase();
    let label: String = match (header.as_str(), normalized.as_str()) {
        ("active", "yes") => "active".into(),
        ("active", "no") => "inactive".into(),
        ("enabled", "yes") => "enabled".into(),
        ("enabled", "no") => "disabled".into(),
        ("trusted", "yes") => "trusted".into(),
        ("trusted", "no") => "untrusted".into(),
        _ => value.into(),
    };
    let semantic = label.to_ascii_lowercase();
    let (marker, style) = match semantic.as_str() {
        "active" | "ready" | "ok" | "healthy" | "completed" | "connected" | "enabled"
        | "trusted" | "running" => ("✓", palette.tone_style(PresentationTone::Success)),
        "failed" | "error" | "denied" | "blocked" | "cancelled" => ("×", palette.error_style()),
        "waiting" | "queued" | "pending" | "paused" | "draft" | "interrupted" | "unknown"
        | "untrusted" => ("!", palette.warning_style()),
        "inactive" | "disabled" => ("·", palette.meta_style()),
        _ => ("·", palette.meta_style()),
    };
    Some((label, marker, style))
}

impl TerminalDocumentRenderer {
    /// Build a renderer for one immutable presentation preference snapshot.
    pub fn new(preferences: TerminalPreferences, width: usize) -> Self {
        Self {
            preferences,
            width: width.clamp(40, 240),
            color: false,
        }
    }

    /// Enable ANSI styling after the caller has confirmed an interactive terminal.
    pub const fn with_color(mut self, color: bool) -> Self {
        self.color = color;
        self
    }

    /// Render one document into bounded terminal text.
    pub fn render(&self, document: &PresentationDocument) -> String {
        let mut lines = Vec::new();
        for block in &document.blocks {
            let mut rendered = self.render_block(block, self.width);
            if !lines.is_empty()
                && !rendered.is_empty()
                && lines.last().is_some_and(|line: &String| !line.is_empty())
                && !matches!(block, PresentationBlock::Blank)
            {
                lines.push(String::new());
            }
            lines.append(&mut rendered);
        }
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines.join("\n")
    }

    fn render_block(&self, block: &PresentationBlock, width: usize) -> Vec<String> {
        match block {
            PresentationBlock::Text(text) => wrap_text(&sanitize_terminal_text(text), width),
            PresentationBlock::Markdown(markdown) => self.render_markdown(markdown, width),
            PresentationBlock::Prompt {
                left,
                indicator,
                input,
                right,
            } => self.render_prompt(left, indicator, input, right.as_deref(), width),
            PresentationBlock::Table(table) => self.render_table(table, width),
            PresentationBlock::Card { title, tone, body } => {
                self.render_card(title, *tone, body, width)
            }
            PresentationBlock::KeyValue(entries) => {
                let mut table = PresentationTable::new(["Field", "Value"], "No details.");
                for (key, value) in entries {
                    table.push_row([key, value]);
                }
                self.render_table(&table, width)
            }
            PresentationBlock::Code { language, content } => {
                self.render_code(language.as_deref(), content, width)
            }
            PresentationBlock::Diff(diff) => self.render_diff(diff, width),
            PresentationBlock::Blank => vec![String::new()],
        }
    }

    fn render_prompt(
        &self,
        left: &str,
        indicator: &str,
        input: &str,
        right: Option<&str>,
        width: usize,
    ) -> Vec<String> {
        let left = sanitize_terminal_text(left);
        let indicator = sanitize_terminal_text(indicator);
        let left = truncate_width(
            &left,
            width.saturating_sub(display_width(&indicator) + 4).max(1),
        );
        let input = sanitize_terminal_text(input);
        let right = right.map(sanitize_terminal_text);
        let palette = TerminalPalette::for_preferences(&self.preferences);
        let prompt_style = palette
            .prompt_left
            .map_or_else(TextStyle::plain, TextStyle::color);
        let indicator_style = palette
            .indicator
            .map_or_else(TextStyle::plain, TextStyle::color);
        let right_style = palette
            .prompt_right
            .map_or_else(TextStyle::plain, TextStyle::color);
        let fixed_width = display_width(&left) + display_width(&indicator) + 2;
        let visible_right = right
            .as_deref()
            .filter(|right| fixed_width + display_width(right) + 10 <= width);
        let right_width = visible_right.map_or(0, |right| display_width(right) + 1);
        let input = truncate_width(
            &input,
            width.saturating_sub(fixed_width + right_width).max(1),
        );
        let left_width = fixed_width + display_width(&input);
        let gap = visible_right.map_or(0, |right| {
            width
                .saturating_sub(left_width + display_width(right))
                .max(1)
        });
        let mut rendered = format!(
            "{} {} {}",
            prompt_style.paint(&left, self.color),
            indicator_style.paint(&indicator, self.color),
            palette.assistant.paint(&input, self.color),
        );
        if let Some(right) = visible_right {
            rendered.push_str(&" ".repeat(gap));
            rendered.push_str(&right_style.paint(right, self.color));
        }
        vec![rendered]
    }

    fn render_card(
        &self,
        title: &str,
        tone: PresentationTone,
        body: &[PresentationBlock],
        width: usize,
    ) -> Vec<String> {
        let inner_width = width.saturating_sub(4).max(20);
        let title = truncate_width(&sanitize_terminal_text(title), inner_width);
        let border_style = self.style_for_tone(tone);
        let top_fill = inner_width
            .saturating_add(1)
            .saturating_sub(UnicodeWidthStr::width(title.as_str()));
        let mut lines = vec![format!(
            "{}",
            border_style.paint(&format!("┌─{title}{}┐", "─".repeat(top_fill)), self.color)
        )];
        let mut body_lines = Vec::new();
        for block in body {
            if !body_lines.is_empty() && !matches!(block, PresentationBlock::Blank) {
                body_lines.push(String::new());
            }
            body_lines.extend(self.render_block(block, inner_width));
        }
        if body_lines.is_empty() {
            body_lines.push(String::new());
        }
        for line in body_lines {
            let raw_width = display_width(&line);
            let padding = inner_width.saturating_sub(raw_width);
            lines.push(format!(
                "{} {}{} {}",
                border_style.paint("│", self.color),
                line,
                " ".repeat(padding),
                border_style.paint("│", self.color)
            ));
        }
        lines.push(border_style.paint(&format!("└{}┘", "─".repeat(inner_width + 2)), self.color));
        lines
    }

    fn render_table(&self, table: &PresentationTable, width: usize) -> Vec<String> {
        if table.rows.is_empty() {
            return wrap_text(&sanitize_terminal_text(&table.empty_message), width);
        }
        let original_columns = table
            .headers
            .len()
            .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
        let columns = original_columns.min(width.saturating_sub(1) / 4).max(1);
        let available = width.saturating_sub(columns * 3 + 1);
        let minimum = (available / columns).clamp(1, 4);
        let mut widths = (0..columns)
            .map(|index| {
                table
                    .headers
                    .get(index)
                    .into_iter()
                    .chain(table.rows.iter().filter_map(|row| row.get(index)))
                    .flat_map(|cell| {
                        sanitize_terminal_text(cell)
                            .lines()
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    })
                    .map(|line| UnicodeWidthStr::width(line.as_str()))
                    .max()
                    .unwrap_or(minimum)
                    .max(minimum)
            })
            .collect::<Vec<_>>();
        while widths.iter().sum::<usize>() > available {
            let Some((index, _)) = widths.iter().enumerate().max_by_key(|(_, value)| **value)
            else {
                break;
            };
            if widths[index] == minimum {
                break;
            }
            widths[index] -= 1;
        }
        let palette = TerminalPalette::for_preferences(&self.preferences);
        let border = |left: char, middle: char, right: char| {
            let mut value = String::new();
            value.push(left);
            for (index, column_width) in widths.iter().enumerate() {
                value.push_str(&"─".repeat(column_width + 2));
                value.push(if index + 1 == columns { right } else { middle });
            }
            palette.meta.paint(&value, self.color)
        };
        let mut lines = vec![border('┌', '┬', '┐')];
        if !table.headers.is_empty() {
            lines.extend(self.render_table_row(&table.headers, &widths, palette.tool));
            lines.push(border('├', '┼', '┤'));
        }
        for (row_index, row) in table.rows.iter().enumerate() {
            lines.extend(self.render_table_row(row, &widths, TextStyle::plain()));
            if row_index + 1 != table.rows.len() {
                lines.push(border('├', '┼', '┤'));
            }
        }
        lines.push(border('└', '┴', '┘'));
        if original_columns > columns {
            lines.push(palette.meta.paint(
                &format!("… {} columns omitted", original_columns - columns),
                self.color,
            ));
        }
        lines
    }

    fn render_table_row(
        &self,
        cells: &[String],
        widths: &[usize],
        style: TextStyle,
    ) -> Vec<String> {
        let wrapped = widths
            .iter()
            .enumerate()
            .map(|(index, width)| {
                wrap_text(
                    &sanitize_terminal_text(cells.get(index).map_or("", String::as_str)),
                    *width,
                )
            })
            .collect::<Vec<_>>();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        let palette = TerminalPalette::for_preferences(&self.preferences);
        (0..height)
            .map(|line_index| {
                let mut line = palette.meta.paint("│", self.color);
                for (index, width) in widths.iter().enumerate() {
                    let cell = wrapped[index].get(line_index).map_or("", String::as_str);
                    let padding = width.saturating_sub(display_width(cell));
                    line.push(' ');
                    line.push_str(&style.paint(cell, self.color));
                    line.push_str(&" ".repeat(padding + 1));
                    line.push_str(&palette.meta.paint("│", self.color));
                }
                line
            })
            .collect()
    }

    fn render_markdown(&self, markdown: &str, width: usize) -> Vec<String> {
        let markdown = sanitize_terminal_text(markdown);
        let source = markdown.lines().collect::<Vec<_>>();
        let palette = TerminalPalette::for_preferences(&self.preferences);
        let mut lines = Vec::new();
        let mut index = 0;
        while index < source.len() {
            let line = source[index];
            if line.trim_start().starts_with("```") {
                let language = line.trim().trim_start_matches("```").trim();
                let mut content = Vec::new();
                index += 1;
                while index < source.len() && !source[index].trim_start().starts_with("```") {
                    content.push(source[index]);
                    index += 1;
                }
                lines.extend(self.render_code(
                    (!language.is_empty()).then_some(language),
                    &content.join("\n"),
                    width,
                ));
            } else if is_markdown_table_header(&source, index) {
                let headers = markdown_cells(source[index]);
                let mut table = PresentationTable::new(headers, "No rows.");
                index += 2;
                while index < source.len()
                    && source[index].contains('|')
                    && !source[index].trim().is_empty()
                {
                    table.push_row(markdown_cells(source[index]));
                    index += 1;
                }
                index = index.saturating_sub(1);
                lines.extend(self.render_table(&table, width));
            } else if let Some((level, heading)) = markdown_heading(line) {
                if !lines.is_empty() && lines.last().is_some_and(|value: &String| !value.is_empty())
                {
                    lines.push(String::new());
                }
                let style = if level == 1 {
                    palette.assistant.bold()
                } else {
                    palette.tool.bold()
                };
                lines.extend(
                    wrap_text(heading, width)
                        .into_iter()
                        .map(|value| style.paint(&render_inline_plain(&value), self.color)),
                );
            } else if let Some(item) = markdown_list_item(line) {
                let prefix = if item.0.is_empty() { "• " } else { item.0 };
                let available = width.saturating_sub(display_width(prefix)).max(8);
                let wrapped = wrap_text(item.1, available);
                for (item_index, value) in wrapped.into_iter().enumerate() {
                    let marker = if item_index == 0 {
                        prefix
                    } else {
                        &" ".repeat(display_width(prefix))
                    };
                    lines.push(format!(
                        "{}{}",
                        palette.tool.paint(marker, self.color),
                        render_inline(&value, palette, self.color)
                    ));
                }
            } else if let Some(quote) = line.trim_start().strip_prefix('>') {
                let wrapped = wrap_text(quote.trim_start(), width.saturating_sub(2));
                lines.extend(wrapped.into_iter().map(|value| {
                    format!(
                        "{} {}",
                        palette.meta.paint("│", self.color),
                        palette.meta.paint(&render_inline_plain(&value), self.color)
                    )
                }));
            } else if line.trim().is_empty() {
                if lines.last().is_some_and(|value: &String| !value.is_empty()) {
                    lines.push(String::new());
                }
            } else {
                lines.extend(
                    wrap_text(line, width)
                        .into_iter()
                        .map(|value| render_inline(&value, palette, self.color)),
                );
            }
            index += 1;
        }
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines
    }

    fn render_code(&self, language: Option<&str>, content: &str, width: usize) -> Vec<String> {
        let palette = TerminalPalette::for_preferences(&self.preferences);
        let inner = width.saturating_sub(4).max(12);
        let label = language.map_or_else(|| "code".into(), sanitize_terminal_text);
        let mut lines = vec![palette.meta.paint(&format!("┌─ {label}"), self.color)];
        let content = sanitize_terminal_text(content);
        let numbered = language.is_some_and(|language| language.contains(" · "));
        let source_lines = content.lines().collect::<Vec<_>>();
        let line_count = source_lines.len().max(1);
        let number_width = line_count.to_string().len();
        if content.is_empty() {
            lines.push(format!("{} ", palette.meta.paint("│", self.color)));
        } else {
            for index in bounded_line_indexes(source_lines.len(), 20, 8) {
                let Some(index) = index else {
                    let omitted = source_lines.len().saturating_sub(28);
                    lines.push(format!(
                        "{} {}",
                        palette.meta.paint("│", self.color),
                        palette
                            .meta
                            .paint(&format!("… {omitted} lines omitted …"), self.color)
                    ));
                    continue;
                };
                let prefix = if numbered {
                    format!("{:>number_width$} │ ", index + 1)
                } else {
                    String::new()
                };
                let value = truncate_width(
                    source_lines[index],
                    inner.saturating_sub(display_width(&prefix)),
                );
                lines.push(format!(
                    "{} {}{}",
                    palette.meta.paint("│", self.color),
                    palette.meta.paint(&prefix, self.color),
                    palette.assistant.paint(&value, self.color)
                ));
            }
        }
        lines.push(
            palette
                .meta
                .paint(&format!("└{}", "─".repeat(inner + 2)), self.color),
        );
        lines
    }

    fn render_diff(&self, diff: &str, width: usize) -> Vec<String> {
        let palette = TerminalPalette::for_preferences(&self.preferences);
        let diff = sanitize_terminal_text(diff);
        let source_lines = diff.lines().collect::<Vec<_>>();
        bounded_line_indexes(source_lines.len(), 80, 20)
            .into_iter()
            .map(|index| {
                let Some(index) = index else {
                    return palette.meta.paint(
                        &format!(
                            "… {} diff lines omitted …",
                            source_lines.len().saturating_sub(100)
                        ),
                        self.color,
                    );
                };
                let line = source_lines[index];
                let value = truncate_width(line, width);
                let style = if value.starts_with('+') && !value.starts_with("+++") {
                    palette.success
                } else if value.starts_with('-') && !value.starts_with("---") {
                    palette.error
                } else if value.starts_with("@@") {
                    palette.tool
                } else {
                    palette.meta
                };
                style.paint(&value, self.color)
            })
            .collect()
    }

    fn style_for_tone(&self, tone: PresentationTone) -> TextStyle {
        let palette = TerminalPalette::for_preferences(&self.preferences);
        match tone {
            PresentationTone::Neutral => palette.meta,
            PresentationTone::Success => palette.success,
            PresentationTone::Warning => palette.warning,
            PresentationTone::Error => palette.error,
            PresentationTone::Thinking => palette.thinking,
            PresentationTone::Tool => palette.tool,
        }
    }
}

fn bounded_line_indexes(count: usize, head: usize, tail: usize) -> Vec<Option<usize>> {
    if count <= head.saturating_add(tail) {
        return (0..count).map(Some).collect();
    }
    (0..head)
        .map(Some)
        .chain(std::iter::once(None))
        .chain((count - tail..count).map(Some))
        .collect()
}

fn sanitize_terminal_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| {
            matches!(character, '\n' | '\t')
                || (!character.is_control()
                    && !matches!(character, '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}'))
        })
        .collect()
}

pub(super) fn display_width(value: &str) -> usize {
    UnicodeWidthStr::width(strip_ansi_for_width(value).as_str())
}

fn strip_ansi_for_width(value: &str) -> String {
    let mut clean = String::new();
    let mut bytes = value.chars().peekable();
    while let Some(character) = bytes.next() {
        if character == '\u{1b}' && bytes.peek() == Some(&'[') {
            bytes.next();
            for next in bytes.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
        } else {
            clean.push(character);
        }
    }
    clean
}

fn truncate_width(value: &str, width: usize) -> String {
    if UnicodeWidthStr::width(value) <= width {
        return value.to_owned();
    }
    let target = width.saturating_sub(1);
    let mut current = 0;
    let mut rendered = String::new();
    for character in value.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if current + character_width > target {
            break;
        }
        rendered.push(character);
        current += character_width;
    }
    rendered.push('…');
    rendered
}

fn wrap_text(value: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rendered = Vec::new();
    for source_line in value.lines() {
        if source_line.is_empty() {
            rendered.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in source_line.split_whitespace() {
            let separator = usize::from(!line.is_empty());
            if display_width(&line) + separator + display_width(word) <= width {
                if separator == 1 {
                    line.push(' ');
                }
                line.push_str(word);
                continue;
            }
            if !line.is_empty() {
                rendered.push(line);
                line = String::new();
            }
            let mut remainder = word;
            while display_width(remainder) > width {
                let (chunk, consumed) = split_width_prefix(remainder, width);
                rendered.push(chunk.into());
                remainder = &remainder[consumed..];
            }
            line.push_str(remainder);
        }
        rendered.push(line);
    }
    if rendered.is_empty() {
        rendered.push(String::new());
    }
    rendered
}

fn split_width_prefix(value: &str, width: usize) -> (&str, usize) {
    let mut current = 0;
    let mut consumed = 0;
    for (index, character) in value.char_indices() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if current + character_width > width && consumed > 0 {
            break;
        }
        current += character_width;
        consumed = index + character.len_utf8();
        if current >= width {
            break;
        }
    }
    if consumed == 0 {
        consumed = value.chars().next().map_or(0, char::len_utf8);
    }
    (&value[..consumed], consumed)
}

fn markdown_heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start();
    let level = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();
    (level > 0 && level <= 6 && trimmed.as_bytes().get(level) == Some(&b' '))
        .then(|| (level, trimmed[level + 1..].trim()))
}

fn markdown_list_item(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    for marker in ["- ", "* ", "+ "] {
        if let Some(value) = trimmed.strip_prefix(marker) {
            return Some(("", value));
        }
    }
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && trimmed.get(digits..digits + 2) == Some(". ") {
        return Some((&trimmed[..digits + 2], &trimmed[digits + 2..]));
    }
    None
}

fn is_markdown_table_header(lines: &[&str], index: usize) -> bool {
    lines.get(index).is_some_and(|line| line.contains('|'))
        && lines.get(index + 1).is_some_and(|line| {
            let cells = markdown_cells(line);
            !cells.is_empty()
                && cells.iter().all(|cell| {
                    let cell = cell.trim().trim_matches(':');
                    cell.len() >= 3 && cell.chars().all(|character| character == '-')
                })
        })
}

fn markdown_cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| render_inline_plain(cell.trim()))
        .collect()
}

fn render_inline_plain(value: &str) -> String {
    let mut rendered = value.replace("**", "").replace("__", "").replace('`', "");
    rendered = rendered.replace(['*', '_'], "");
    while let Some(start) = rendered.find('[') {
        let Some(label_end) = rendered[start + 1..]
            .find("](")
            .map(|value| start + 1 + value)
        else {
            break;
        };
        let url_start = label_end + 2;
        let Some(url_end) = rendered[url_start..]
            .find(')')
            .map(|value| url_start + value)
        else {
            break;
        };
        let replacement = format!(
            "{} ({})",
            &rendered[start + 1..label_end],
            &rendered[url_start..url_end]
        );
        rendered.replace_range(start..=url_end, &replacement);
    }
    rendered
}

fn render_inline(value: &str, palette: TerminalPalette, color: bool) -> String {
    if !color {
        return render_inline_plain(value);
    }
    let mut rendered = String::new();
    let mut remaining = value;
    while !remaining.is_empty() {
        if let Some(content) = remaining.strip_prefix("**")
            && let Some(end) = content.find("**")
        {
            rendered.push_str(&palette.assistant.bold().paint(&content[..end], true));
            remaining = &content[end + 2..];
            continue;
        }
        if let Some(content) = remaining.strip_prefix("__")
            && let Some(end) = content.find("__")
        {
            rendered.push_str(&palette.assistant.bold().paint(&content[..end], true));
            remaining = &content[end + 2..];
            continue;
        }
        if let Some(content) = remaining.strip_prefix('`')
            && let Some(end) = content.find('`')
        {
            rendered.push_str(&palette.tool.paint(&content[..end], true));
            remaining = &content[end + 1..];
            continue;
        }
        if let Some(content) = remaining.strip_prefix('*')
            && let Some(end) = content.find('*')
        {
            rendered.push_str(&palette.assistant.italic().paint(&content[..end], true));
            remaining = &content[end + 1..];
            continue;
        }
        if let Some(content) = remaining.strip_prefix('_')
            && let Some(end) = content.find('_')
        {
            rendered.push_str(&palette.assistant.italic().paint(&content[..end], true));
            remaining = &content[end + 1..];
            continue;
        }
        if let Some(label) = remaining.strip_prefix('[')
            && let Some(label_end) = label.find("](")
        {
            let url = &label[label_end + 2..];
            if let Some(url_end) = url.find(')') {
                rendered.push_str(&palette.assistant.paint(&label[..label_end], true));
                rendered.push_str(&palette.meta.paint(&format!(" ({})", &url[..url_end]), true));
                remaining = &url[url_end + 1..];
                continue;
            }
        }
        let next = remaining
            .char_indices()
            .skip(1)
            .find(|(_, character)| matches!(character, '*' | '`' | '[' | '_'))
            .map_or(remaining.len(), |(index, _)| index);
        rendered.push_str(&palette.assistant.paint(&remaining[..next], true));
        remaining = &remaining[next..];
    }
    rendered
}
