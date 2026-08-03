//! CommonMark parsing and backend-neutral terminal styling.

use crate::{StyledLine, StyledSpan, TerminalPalette, TerminalPreferences, ThemeName};
use colossus_contracts::{ThemeColor, ThemeTextStyle};
use inkjet::{
    Highlighter as InkjetHighlighter, Language, constants::HIGHLIGHT_NAMES,
    tree_sitter_highlight::HighlightEvent,
};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::collections::HashMap;
use std::hash::{Hash as _, Hasher as _};
use std::sync::{Mutex, OnceLock};
use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

const MAX_HIGHLIGHT_BYTES: usize = 512 * 1024;
const MAX_HIGHLIGHT_LINES: usize = 10_000;
const HIGHLIGHT_CACHE_LIMIT: usize = 128;
const CODE_HEAD_LINES: usize = 20;
const CODE_TAIL_LINES: usize = 8;

static HIGHLIGHT_CACHE: OnceLock<Mutex<HashMap<u64, Vec<Vec<StyledSpan>>>>> = OnceLock::new();

#[derive(Clone, Debug)]
struct ListState {
    ordered: bool,
    next: u64,
    marker: String,
    marker_emitted: bool,
}

impl ListState {
    fn new(start: Option<u64>) -> Self {
        Self {
            ordered: start.is_some(),
            next: start.unwrap_or(1),
            marker: String::new(),
            marker_emitted: false,
        }
    }

    fn begin_item(&mut self) {
        self.marker = if self.ordered {
            let marker = format!("{}. ", self.next);
            self.next = self.next.saturating_add(1);
            marker
        } else {
            "• ".into()
        };
        self.marker_emitted = false;
    }
}

#[derive(Debug)]
struct LinkState {
    destination: String,
}

#[derive(Debug)]
struct ImageState {
    destination: String,
    alt: String,
}

#[derive(Debug)]
struct CodeState {
    language: Option<String>,
    content: String,
}

#[derive(Debug)]
struct TableState {
    alignments: Vec<Alignment>,
    rows: Vec<Vec<String>>,
    row: Vec<String>,
    cell: String,
}

impl TableState {
    fn new(alignments: Vec<Alignment>) -> Self {
        Self {
            alignments,
            rows: Vec::new(),
            row: Vec::new(),
            cell: String::new(),
        }
    }

    fn finish_cell(&mut self) {
        self.row.push(self.cell.trim().to_string());
        self.cell.clear();
    }

    fn finish_row(&mut self) {
        if !self.row.is_empty() {
            self.rows.push(std::mem::take(&mut self.row));
        }
    }
}

struct Writer<'a> {
    preferences: &'a TerminalPreferences,
    palette: TerminalPalette,
    width: usize,
    lines: Vec<StyledLine>,
    current: Vec<StyledSpan>,
    pending_separator: bool,
    heading: Option<HeadingLevel>,
    strong_depth: usize,
    emphasis_depth: usize,
    strike_depth: usize,
    blockquote_depth: usize,
    lists: Vec<ListState>,
    links: Vec<LinkState>,
    image: Option<ImageState>,
    code: Option<CodeState>,
    table: Option<TableState>,
}

/// Render sanitized CommonMark into backend-neutral styled terminal lines.
pub(super) fn render(
    markdown: &str,
    width: usize,
    preferences: &TerminalPreferences,
) -> Vec<StyledLine> {
    let markdown = crate::terminal::sanitize_terminal_text(markdown);
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_SMART_PUNCTUATION);

    let mut writer = Writer::new(preferences, width);
    for event in Parser::new_ext(&markdown, options) {
        writer.handle(event);
    }
    writer.finish_current();
    writer.trim_trailing_blanks();
    writer.lines
}

impl<'a> Writer<'a> {
    fn new(preferences: &'a TerminalPreferences, width: usize) -> Self {
        Self {
            preferences,
            palette: TerminalPalette::for_preferences(preferences),
            width: width.max(1),
            lines: Vec::new(),
            current: Vec::new(),
            pending_separator: false,
            heading: None,
            strong_depth: 0,
            emphasis_depth: 0,
            strike_depth: 0,
            blockquote_depth: 0,
            lists: Vec::new(),
            links: Vec::new(),
            image: None,
            code: None,
            table: None,
        }
    }

    fn handle(&mut self, event: Event<'_>) {
        if self.table.is_some() && self.handle_table_event(&event) {
            return;
        }
        if self.code.is_some() && self.handle_code_event(&event) {
            return;
        }
        if self.image.is_some() && self.handle_image_event(&event) {
            return;
        }

        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => self.push_text(&text, self.inline_style()),
            Event::Code(code) => {
                let mut style = self.palette.tool_style();
                style.bold = false;
                self.push_text(&code, style);
            }
            Event::InlineMath(math) => {
                let mut style = self.palette.tool_style();
                style.italic = true;
                self.push_text(&format!("${math}$"), style);
            }
            Event::DisplayMath(math) => {
                self.begin_block();
                let mut style = self.palette.tool_style();
                style.italic = true;
                self.current.push(StyledSpan {
                    content: format!("$${math}$$"),
                    style,
                });
                self.finish_current();
                self.pending_separator = self.is_top_level();
            }
            Event::SoftBreak => self.push_text(" ", self.inline_style()),
            Event::HardBreak => self.finish_current(),
            Event::Rule => {
                self.begin_block();
                let structural_width = self.structural_content_width();
                self.append_structured_lines(vec![StyledLine::from_span(
                    "─".repeat(structural_width),
                    self.palette.meta_style(),
                )]);
                self.pending_separator = self.is_top_level();
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                let mut style = self.palette.meta_style();
                style.italic = true;
                self.push_text(&html, style);
            }
            Event::FootnoteReference(label) => {
                self.push_text(&format!("[^{label}]"), self.palette.meta_style());
            }
            Event::TaskListMarker(checked) => {
                if let Some(list) = self.lists.last_mut() {
                    list.marker = if checked { "☑ " } else { "☐ " }.into();
                }
            }
        }
    }

    fn start_tag(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                if self.is_top_level() {
                    self.begin_block();
                }
            }
            Tag::Heading { level, .. } => {
                self.begin_block();
                self.heading = Some(level);
            }
            Tag::BlockQuote(_) => {
                if self.blockquote_depth == 0 && self.lists.is_empty() {
                    self.begin_block();
                }
                self.finish_current();
                self.blockquote_depth = self.blockquote_depth.saturating_add(1).min(4);
            }
            Tag::CodeBlock(kind) => {
                if self.is_top_level() {
                    self.begin_block();
                }
                self.finish_current();
                let language = match kind {
                    CodeBlockKind::Indented => None,
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .filter(|value| !value.is_empty())
                        .map(str::to_string),
                };
                self.code = Some(CodeState {
                    language,
                    content: String::new(),
                });
            }
            Tag::List(start) => {
                if self.lists.is_empty() && self.blockquote_depth == 0 {
                    self.begin_block();
                }
                self.finish_current();
                self.lists.push(ListState::new(start));
            }
            Tag::Item => {
                self.finish_current();
                if let Some(list) = self.lists.last_mut() {
                    list.begin_item();
                }
            }
            Tag::Emphasis => self.emphasis_depth = self.emphasis_depth.saturating_add(1),
            Tag::Strong => self.strong_depth = self.strong_depth.saturating_add(1),
            Tag::Strikethrough => self.strike_depth = self.strike_depth.saturating_add(1),
            Tag::Link { dest_url, .. } => self.links.push(LinkState {
                destination: dest_url.to_string(),
            }),
            Tag::Image { dest_url, .. } => {
                self.image = Some(ImageState {
                    destination: dest_url.to_string(),
                    alt: String::new(),
                });
            }
            Tag::Table(alignments) => {
                self.begin_block();
                self.finish_current();
                self.table = Some(TableState::new(alignments));
            }
            Tag::FootnoteDefinition(label) => {
                self.begin_block();
                self.push_text(&format!("[^{label}] "), self.palette.meta_style());
            }
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.finish_current();
                if self.is_top_level() {
                    self.pending_separator = true;
                }
            }
            TagEnd::Heading(_) => {
                self.finish_current();
                self.heading = None;
                if self.is_top_level() {
                    self.pending_separator = true;
                }
            }
            TagEnd::BlockQuote(_) => {
                self.finish_current();
                self.blockquote_depth = self.blockquote_depth.saturating_sub(1);
                if self.blockquote_depth == 0 && self.lists.is_empty() {
                    self.pending_separator = true;
                }
            }
            TagEnd::List(_) => {
                self.finish_current();
                self.lists.pop();
                if self.lists.is_empty() && self.blockquote_depth == 0 {
                    self.pending_separator = true;
                }
            }
            TagEnd::Item => self.finish_current(),
            TagEnd::Emphasis => self.emphasis_depth = self.emphasis_depth.saturating_sub(1),
            TagEnd::Strong => self.strong_depth = self.strong_depth.saturating_sub(1),
            TagEnd::Strikethrough => self.strike_depth = self.strike_depth.saturating_sub(1),
            TagEnd::Link => {
                if let Some(link) = self.links.pop()
                    && !link.destination.is_empty()
                {
                    self.push_text(" (", self.inline_style());
                    self.push_text(&link.destination, self.palette.meta_style());
                    self.push_text(")", self.inline_style());
                }
            }
            TagEnd::FootnoteDefinition => {
                self.finish_current();
                self.pending_separator = self.is_top_level();
            }
            _ => {}
        }
    }

    fn handle_code_event(&mut self, event: &Event<'_>) -> bool {
        match event {
            Event::Text(text) | Event::Code(text) => {
                if let Some(code) = self.code.as_mut() {
                    code.content.push_str(text);
                }
                true
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(code) = self.code.as_mut() {
                    code.content.push('\n');
                }
                true
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(code) = self.code.take() {
                    let content_width = self.structural_content_width();
                    let lines = render_code_block(
                        code.language.as_deref(),
                        &code.content,
                        content_width,
                        self.palette,
                        self.preferences.theme != ThemeName::Mono,
                    );
                    self.append_structured_lines(lines);
                    if self.is_top_level() {
                        self.pending_separator = true;
                    }
                }
                true
            }
            _ => true,
        }
    }

    fn handle_image_event(&mut self, event: &Event<'_>) -> bool {
        match event {
            Event::Text(text) | Event::Code(text) => {
                if let Some(image) = self.image.as_mut() {
                    image.alt.push_str(text);
                }
                true
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(image) = self.image.as_mut() {
                    image.alt.push(' ');
                }
                true
            }
            Event::End(TagEnd::Image) => {
                if let Some(image) = self.image.take() {
                    let label = if image.alt.trim().is_empty() {
                        "image".to_string()
                    } else {
                        format!("image: {}", image.alt.trim())
                    };
                    self.push_text(&label, self.palette.tool_style());
                    if !image.destination.is_empty() {
                        self.push_text(" (", self.inline_style());
                        self.push_text(&image.destination, self.palette.meta_style());
                        self.push_text(")", self.inline_style());
                    }
                }
                true
            }
            _ => true,
        }
    }

    fn handle_table_event(&mut self, event: &Event<'_>) -> bool {
        match event {
            Event::Text(text) | Event::Code(text) | Event::Html(text) | Event::InlineHtml(text) => {
                if let Some(table) = self.table.as_mut() {
                    table.cell.push_str(text);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(table) = self.table.as_mut() {
                    table.cell.push(' ');
                }
            }
            Event::TaskListMarker(checked) => {
                if let Some(table) = self.table.as_mut() {
                    table.cell.push_str(if *checked { "☑ " } else { "☐ " });
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(table) = self.table.as_mut() {
                    table.finish_cell();
                }
            }
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                if let Some(table) = self.table.as_mut() {
                    table.finish_row();
                }
            }
            Event::End(TagEnd::Table) => {
                if let Some(mut table) = self.table.take() {
                    table.finish_row();
                    let content_width = self.structural_content_width();
                    let lines =
                        render_table(&table.rows, &table.alignments, content_width, self.palette);
                    self.append_structured_lines(lines);
                    if self.is_top_level() {
                        self.pending_separator = true;
                    }
                }
            }
            _ => {}
        }
        true
    }

    fn inline_style(&self) -> ThemeTextStyle {
        let mut style = if self.heading.is_some() {
            if self.heading == Some(HeadingLevel::H1) {
                self.palette.section_style()
            } else {
                self.palette.tool_style()
            }
        } else if self.links.is_empty() {
            self.palette.assistant_style()
        } else {
            self.palette.tool_style()
        };
        if self.heading.is_some() || self.strong_depth > 0 {
            style.bold = true;
            style.dim = false;
        }
        if self.emphasis_depth > 0 {
            style.italic = true;
        }
        if self.strike_depth > 0 {
            style.dim = true;
        }
        if self.blockquote_depth > 0 {
            style.italic = true;
        }
        style
    }

    fn push_text(&mut self, value: &str, style: ThemeTextStyle) {
        if value.is_empty() {
            return;
        }
        if let Some(last) = self.current.last_mut()
            && last.style == style
        {
            last.content.push_str(value);
        } else {
            self.current.push(StyledSpan {
                content: value.to_string(),
                style,
            });
        }
    }

    fn begin_block(&mut self) {
        if self.pending_separator {
            push_blank(&mut self.lines);
            self.pending_separator = false;
        }
    }

    fn finish_current(&mut self) {
        if self.current.is_empty() {
            return;
        }
        let (first_prefix, continuation_prefix) = self.structural_prefixes();
        let body = std::mem::take(&mut self.current);
        self.lines.extend(wrap_styled(
            body,
            first_prefix,
            continuation_prefix,
            self.width,
        ));
    }

    fn append_structured_lines(&mut self, lines: Vec<StyledLine>) {
        let (first_prefix, continuation_prefix) = self.structural_prefixes();
        for (index, mut line) in lines.into_iter().enumerate() {
            let mut prefixed = if index == 0 {
                first_prefix.clone()
            } else {
                continuation_prefix.clone()
            };
            prefixed.append(&mut line.spans);
            self.lines.push(StyledLine { spans: prefixed });
        }
    }

    fn structural_prefixes(&mut self) -> (Vec<StyledSpan>, Vec<StyledSpan>) {
        let mut first = Vec::new();
        let mut continuation = Vec::new();
        if self.blockquote_depth > 0 {
            let quote = "│ ".repeat(self.blockquote_depth);
            first.push(StyledSpan {
                content: quote.clone(),
                style: self.palette.meta_style(),
            });
            continuation.push(StyledSpan {
                content: quote,
                style: self.palette.meta_style(),
            });
        }
        let list_depth = self.lists.len();
        if let Some(list) = self.lists.last_mut() {
            let indent = "  ".repeat(list_depth.saturating_sub(1));
            let marker = if list.marker.is_empty() {
                "• ".to_string()
            } else {
                list.marker.clone()
            };
            let marker_width = display_width(&marker);
            let first_text = if list.marker_emitted {
                format!("{indent}{}", " ".repeat(marker_width))
            } else {
                list.marker_emitted = true;
                format!("{indent}{marker}")
            };
            let continuation_text = format!("{indent}{}", " ".repeat(marker_width));
            first.push(StyledSpan {
                content: first_text,
                style: self.palette.meta_style(),
            });
            continuation.push(StyledSpan {
                content: continuation_text,
                style: self.palette.meta_style(),
            });
        }
        (first, continuation)
    }

    fn structural_content_width(&self) -> usize {
        let quote = self.blockquote_depth.saturating_mul(2);
        let list = self.lists.last().map_or(0, |state| {
            self.lists.len().saturating_sub(1).saturating_mul(2)
                + display_width(if state.marker.is_empty() {
                    "• "
                } else {
                    &state.marker
                })
        });
        self.width.saturating_sub(quote + list).max(8)
    }

    fn is_top_level(&self) -> bool {
        self.blockquote_depth == 0 && self.lists.is_empty()
    }

    fn trim_trailing_blanks(&mut self) {
        while self.lines.last().is_some_and(|line| line.spans.is_empty()) {
            self.lines.pop();
        }
    }
}

fn render_code_block(
    language: Option<&str>,
    content: &str,
    width: usize,
    palette: TerminalPalette,
    syntax_colors: bool,
) -> Vec<StyledLine> {
    let width = width.max(8);
    let label = language.unwrap_or("code");
    let header = truncate_plain(&format!("┌─ {label}"), width);
    let mut lines = vec![StyledLine::from_span(header, palette.meta_style())];
    let highlighted = if syntax_colors {
        highlight_code(content, language)
    } else {
        None
    };
    let source = content.lines().collect::<Vec<_>>();
    let count = source.len().max(1);
    let indexes = bounded_line_indexes(count);
    for index in indexes {
        let mut spans = vec![StyledSpan {
            content: "│ ".into(),
            style: palette.meta_style(),
        }];
        match index {
            Some(index) => {
                let source_line = source.get(index).copied().unwrap_or("");
                let code_spans = highlighted
                    .as_ref()
                    .and_then(|lines| lines.get(index).cloned())
                    .unwrap_or_else(|| {
                        vec![StyledSpan {
                            content: source_line.to_string(),
                            style: palette.assistant_style(),
                        }]
                    });
                spans.extend(truncate_spans(code_spans, width.saturating_sub(2)));
            }
            None => spans.push(StyledSpan {
                content: truncate_plain(
                    &format!(
                        "… {} lines omitted …",
                        count.saturating_sub(CODE_HEAD_LINES + CODE_TAIL_LINES)
                    ),
                    width.saturating_sub(2),
                ),
                style: palette.meta_style(),
            }),
        }
        lines.push(StyledLine { spans });
    }
    lines.push(StyledLine::from_span(
        format!("└{}", "─".repeat(width.saturating_sub(1))),
        palette.meta_style(),
    ));
    lines
}

fn highlight_code(content: &str, language: Option<&str>) -> Option<Vec<Vec<StyledSpan>>> {
    if content.len() > MAX_HIGHLIGHT_BYTES || content.lines().count() > MAX_HIGHLIGHT_LINES {
        return None;
    }
    let key = highlight_key(content, language);
    let cache = HIGHLIGHT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(cache) = cache.lock()
        && let Some(lines) = cache.get(&key)
    {
        return Some(lines.clone());
    }

    let language = language
        .and_then(|language| language.split_whitespace().next())
        .map(|language| language.trim().trim_start_matches('.').to_ascii_lowercase())
        .and_then(Language::from_token)
        .unwrap_or(Language::Plaintext);
    let mut highlighter = InkjetHighlighter::new();
    let events = highlighter.highlight_raw(language, &content).ok()?;
    let mut rendered = vec![Vec::new()];
    let mut styles = Vec::new();
    for event in events {
        match event.ok()? {
            HighlightEvent::Source { start, end } => {
                let style = styles.last().copied().unwrap_or_else(plain_syntax_style);
                push_highlighted_source(&mut rendered, &content[start..end], style);
            }
            HighlightEvent::HighlightStart(highlight) => {
                let name = HIGHLIGHT_NAMES
                    .get(highlight.0)
                    .copied()
                    .unwrap_or_default();
                styles.push(syntax_style(name));
            }
            HighlightEvent::HighlightEnd => {
                styles.pop();
            }
        }
    }
    if content.ends_with('\n') && rendered.len() > 1 && rendered.last().is_some_and(Vec::is_empty) {
        rendered.pop();
    }
    if rendered.is_empty() {
        rendered.push(Vec::new());
    }
    if let Ok(mut cache) = cache.lock() {
        if cache.len() >= HIGHLIGHT_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(key, rendered.clone());
    }
    Some(rendered)
}

fn highlight_key(content: &str, language: Option<&str>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    language.hash(&mut hasher);
    hasher.finish()
}

fn push_highlighted_source(
    rendered: &mut Vec<Vec<StyledSpan>>,
    source: &str,
    style: ThemeTextStyle,
) {
    for segment in source.split_inclusive('\n') {
        let has_newline = segment.ends_with('\n');
        let text = segment
            .strip_suffix('\n')
            .unwrap_or(segment)
            .strip_suffix('\r')
            .unwrap_or_else(|| segment.strip_suffix('\n').unwrap_or(segment));
        if let Some(line) = rendered.last_mut() {
            push_span(line, text, style);
        }
        if has_newline {
            rendered.push(Vec::new());
        }
    }
}

fn plain_syntax_style() -> ThemeTextStyle {
    ThemeTextStyle {
        foreground: None,
        bold: false,
        dim: false,
        italic: false,
    }
}

fn syntax_style(name: &str) -> ThemeTextStyle {
    let (color, bold, dim, italic) = if name.starts_with("comment") {
        (Some((108, 112, 134)), false, true, true)
    } else if name.starts_with("keyword") {
        (Some((203, 166, 247)), true, false, false)
    } else if name.starts_with("type") || name == "constructor" || name == "attribute" {
        (Some((249, 226, 175)), false, false, false)
    } else if name.starts_with("string") || name == "escape" {
        (Some((166, 227, 161)), false, false, false)
    } else if name.starts_with("constant") {
        (Some((250, 179, 135)), false, false, false)
    } else if name.starts_with("function") {
        (Some((137, 180, 250)), false, false, false)
    } else if name == "operator" || name.starts_with("punctuation.special") {
        (Some((137, 220, 235)), false, false, false)
    } else if name.starts_with("tag") {
        (Some((243, 139, 168)), false, false, false)
    } else if name == "namespace" || name == "label" {
        (Some((148, 226, 213)), false, false, false)
    } else if name.starts_with("markup.heading") || name == "markup.bold" {
        (Some((137, 180, 250)), true, false, false)
    } else if name == "markup.italic" {
        (Some((245, 194, 231)), false, false, true)
    } else if name.starts_with("markup.link") {
        (Some((137, 220, 235)), false, false, false)
    } else if name.starts_with("markup.quote") {
        (Some((166, 227, 161)), false, true, true)
    } else if name.starts_with("diff.plus") {
        (Some((166, 227, 161)), false, false, false)
    } else if name.starts_with("diff.minus") {
        (Some((243, 139, 168)), false, false, false)
    } else if name.starts_with("diff") {
        (Some((249, 226, 175)), false, false, false)
    } else {
        return plain_syntax_style();
    };
    ThemeTextStyle {
        foreground: color.map(|(red, green, blue)| ThemeColor { red, green, blue }),
        bold,
        dim,
        italic,
    }
}

fn render_table(
    rows: &[Vec<String>],
    alignments: &[Alignment],
    width: usize,
    palette: TerminalPalette,
) -> Vec<StyledLine> {
    if rows.is_empty() {
        return Vec::new();
    }
    let columns = alignments
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if columns == 0 {
        return Vec::new();
    }
    let frame_width = columns.saturating_mul(3).saturating_add(1);
    if width <= frame_width.saturating_add(columns.saturating_mul(3)) {
        return render_table_records(rows, width, palette);
    }
    let mut widths = (0..columns)
        .map(|column| {
            rows.iter()
                .filter_map(|row| row.get(column))
                .map(|cell| display_width(cell))
                .max()
                .unwrap_or(1)
                .clamp(3, 48)
        })
        .collect::<Vec<_>>();
    let available = width.saturating_sub(frame_width);
    while widths.iter().sum::<usize>() > available {
        let Some((index, _)) = widths.iter().enumerate().max_by_key(|(_, value)| *value) else {
            break;
        };
        if widths[index] <= 3 {
            return render_table_records(rows, width, palette);
        }
        widths[index] -= 1;
    }

    let mut lines = vec![table_rule('┌', '┬', '┐', &widths, palette)];
    for (row_index, row) in rows.iter().enumerate() {
        let wrapped = widths
            .iter()
            .enumerate()
            .map(|(column, width)| wrap_plain(row.get(column).map_or("", String::as_str), *width))
            .collect::<Vec<_>>();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        for line_index in 0..height {
            let mut spans = vec![StyledSpan {
                content: "│ ".into(),
                style: palette.meta_style(),
            }];
            for (column, column_width) in widths.iter().enumerate() {
                let cell = wrapped[column].get(line_index).map_or("", String::as_str);
                let alignment = alignments.get(column).copied().unwrap_or(Alignment::None);
                let cell = align_cell(cell, *column_width, alignment);
                let mut style = if row_index == 0 {
                    palette.tool_style()
                } else {
                    palette.assistant_style()
                };
                style.bold = row_index == 0;
                spans.push(StyledSpan {
                    content: cell,
                    style,
                });
                spans.push(StyledSpan {
                    content: " │".into(),
                    style: palette.meta_style(),
                });
                if column + 1 < columns {
                    spans.push(StyledSpan {
                        content: " ".into(),
                        style: palette.meta_style(),
                    });
                }
            }
            lines.push(StyledLine { spans });
        }
        if row_index == 0 && rows.len() > 1 {
            lines.push(table_rule('├', '┼', '┤', &widths, palette));
        }
    }
    lines.push(table_rule('└', '┴', '┘', &widths, palette));
    lines
}

fn render_table_records(
    rows: &[Vec<String>],
    width: usize,
    palette: TerminalPalette,
) -> Vec<StyledLine> {
    let headers = rows.first().cloned().unwrap_or_default();
    let mut lines = Vec::new();
    for row in rows.iter().skip(1) {
        let value = row
            .iter()
            .enumerate()
            .map(|(index, value)| {
                format!(
                    "{}: {value}",
                    headers
                        .get(index)
                        .map_or_else(|| format!("Field {}", index + 1), Clone::clone)
                )
            })
            .collect::<Vec<_>>()
            .join(" · ");
        let body = vec![StyledSpan {
            content: value,
            style: palette.assistant_style(),
        }];
        lines.extend(wrap_styled(
            body,
            vec![StyledSpan {
                content: "• ".into(),
                style: palette.meta_style(),
            }],
            vec![StyledSpan {
                content: "  ".into(),
                style: palette.meta_style(),
            }],
            width,
        ));
    }
    if lines.is_empty() {
        lines.push(StyledLine::from_span(
            headers.join(" · "),
            palette.tool_style(),
        ));
    }
    lines
}

fn table_rule(
    left: char,
    join: char,
    right: char,
    widths: &[usize],
    palette: TerminalPalette,
) -> StyledLine {
    let mut text = left.to_string();
    for (index, width) in widths.iter().enumerate() {
        text.push_str(&"─".repeat(width.saturating_add(2)));
        text.push(if index + 1 == widths.len() {
            right
        } else {
            join
        });
    }
    StyledLine::from_span(text, palette.meta_style())
}

fn align_cell(value: &str, width: usize, alignment: Alignment) -> String {
    let padding = width.saturating_sub(display_width(value));
    match alignment {
        Alignment::Right => format!("{}{value}", " ".repeat(padding)),
        Alignment::Center => {
            let left = padding / 2;
            format!("{}{value}{}", " ".repeat(left), " ".repeat(padding - left))
        }
        Alignment::Left | Alignment::None => format!("{value}{}", " ".repeat(padding)),
    }
}

fn wrap_styled(
    spans: Vec<StyledSpan>,
    first_prefix: Vec<StyledSpan>,
    continuation_prefix: Vec<StyledSpan>,
    width: usize,
) -> Vec<StyledLine> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = StyledLine {
        spans: first_prefix,
    };
    let mut line_width = styled_width(&line.spans);
    let mut pending_space = false;
    for span in spans {
        let mut remaining = span.content.as_str();
        while !remaining.is_empty() {
            let whitespace = remaining.chars().next().is_some_and(char::is_whitespace);
            let end = remaining
                .char_indices()
                .find(|(_, character)| character.is_whitespace() != whitespace)
                .map_or(remaining.len(), |(index, _)| index);
            let token = &remaining[..end];
            remaining = &remaining[end..];
            if whitespace {
                pending_space |= line_width > 0;
                continue;
            }

            let separator = usize::from(pending_space && line_width > 0);
            if line_width > styled_width(&continuation_prefix)
                && line_width + separator + display_width(token) > width
            {
                lines.push(line);
                line = StyledLine {
                    spans: continuation_prefix.clone(),
                };
                line_width = styled_width(&line.spans);
                pending_space = false;
            }
            if pending_space && line_width > 0 && line_width < width {
                push_span(&mut line.spans, " ", span.style);
                line_width += 1;
            }
            pending_space = false;

            let mut token = token;
            while display_width(token) > width.saturating_sub(line_width) {
                if line_width > styled_width(&continuation_prefix) {
                    lines.push(line);
                    line = StyledLine {
                        spans: continuation_prefix.clone(),
                    };
                    line_width = styled_width(&line.spans);
                    continue;
                }
                let available = width.saturating_sub(line_width).max(1);
                let (chunk, consumed) = split_width_prefix(token, available);
                push_span(&mut line.spans, chunk, span.style);
                lines.push(line);
                line = StyledLine {
                    spans: continuation_prefix.clone(),
                };
                line_width = styled_width(&line.spans);
                token = &token[consumed..];
            }
            if !token.is_empty() {
                push_span(&mut line.spans, token, span.style);
                line_width += display_width(token);
            }
        }
    }
    if !line.spans.is_empty() {
        lines.push(line);
    }
    lines
}

fn truncate_spans(spans: Vec<StyledSpan>, width: usize) -> Vec<StyledSpan> {
    let mut rendered = Vec::new();
    let mut remaining = width;
    for span in spans {
        if remaining == 0 {
            break;
        }
        let value = truncate_plain(&span.content, remaining);
        remaining = remaining.saturating_sub(display_width(&value));
        push_span(&mut rendered, value, span.style);
    }
    rendered
}

fn push_span(spans: &mut Vec<StyledSpan>, content: impl Into<String>, style: ThemeTextStyle) {
    let content = content.into();
    if content.is_empty() {
        return;
    }
    if let Some(last) = spans.last_mut()
        && last.style == style
    {
        last.content.push_str(&content);
    } else {
        spans.push(StyledSpan { content, style });
    }
}

fn wrap_plain(value: &str, width: usize) -> Vec<String> {
    let spans = vec![StyledSpan {
        content: value.to_string(),
        style: ThemeTextStyle {
            foreground: None,
            bold: false,
            dim: false,
            italic: false,
        },
    }];
    wrap_styled(spans, Vec::new(), Vec::new(), width)
        .into_iter()
        .map(|line| line.plain_text())
        .collect()
}

fn bounded_line_indexes(line_count: usize) -> Vec<Option<usize>> {
    if line_count <= CODE_HEAD_LINES + CODE_TAIL_LINES {
        return (0..line_count).map(Some).collect();
    }
    (0..CODE_HEAD_LINES)
        .map(Some)
        .chain(std::iter::once(None))
        .chain((line_count - CODE_TAIL_LINES..line_count).map(Some))
        .collect()
}

fn styled_width(spans: &[StyledSpan]) -> usize {
    spans.iter().map(|span| display_width(&span.content)).sum()
}

fn display_width(value: &str) -> usize {
    value.width()
}

fn truncate_plain(value: &str, width: usize) -> String {
    if display_width(value) <= width {
        return value.to_string();
    }
    let (prefix, _) = split_width_prefix(value, width);
    prefix.to_string()
}

fn split_width_prefix(value: &str, width: usize) -> (&str, usize) {
    let mut current = 0;
    let mut consumed = 0;
    for (index, character) in value.char_indices() {
        let character_width = character.width().unwrap_or(0);
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

fn push_blank(lines: &mut Vec<StyledLine>) {
    if lines.last().is_some_and(|line| !line.spans.is_empty()) {
        lines.push(StyledLine::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pathological_code_blocks_fall_back_without_highlighting() {
        let oversized = "x".repeat(MAX_HIGHLIGHT_BYTES + 1);
        assert!(highlight_code(&oversized, Some("rust")).is_none());

        let too_many_lines = "x\n".repeat(MAX_HIGHLIGHT_LINES + 1);
        assert!(highlight_code(&too_many_lines, Some("rust")).is_none());
    }
}
