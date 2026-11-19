//! Incremental, transport-independent rendering of released conversation snapshots.

use colossus_contracts::{
    TranscriptActivity, TranscriptPage, TranscriptPart, TranscriptRole, TranscriptSnapshot,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeSet,
    io::{self, Write},
};

/// Maximum formatted bytes in one export. Exceeding it fails without truncation.
pub const MAX_TRANSCRIPT_EXPORT_BYTES: u64 = 256 * 1024 * 1024;
/// Maximum source pages consumed by one export, including filtered empty pages.
pub const MAX_TRANSCRIPT_EXPORT_PAGES: usize = 10_000;

/// Human-readable and machine-readable conversation export encodings.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptFormat {
    /// Markdown preserves the original message formatting.
    #[default]
    Markdown,
    /// Plain text adds readable role and timestamp separators.
    Text,
    /// Versioned JSON contains only the released transcript contract.
    Json,
}

impl TranscriptFormat {
    /// File extension without a leading dot.
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Text => "txt",
            Self::Json => "json",
        }
    }

    /// UTF-8 media type for this encoding.
    pub const fn media_type(self) -> &'static str {
        match self {
            Self::Markdown => "text/markdown;charset=utf-8",
            Self::Text => "text/plain;charset=utf-8",
            Self::Json => "application/json;charset=utf-8",
        }
    }
}

struct BoundedWriter<W> {
    inner: W,
    bytes: u64,
}

impl<W: Write> Write for BoundedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX)
            > MAX_TRANSCRIPT_EXPORT_BYTES.saturating_sub(self.bytes)
        {
            return Err(invalid("The conversation exceeds the export size limit."));
        }
        let written = self.inner.write(bytes)?;
        self.bytes += u64::try_from(written).unwrap_or(u64::MAX);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// A streaming formatter for one immutable released transcript snapshot.
///
/// Saved text is preserved, including materialized text attachment contents.
/// Attachment references become metadata placeholders; this formatter fetches
/// no attachment files and embeds no image bytes.
///
/// Hosts must stage output and publish it only after [`Self::finish`] succeeds.
/// A failed read, changed snapshot, repeated cursor, size limit, or write error
/// must never turn a partial staging file into a successful export.
pub struct TranscriptWriter<W: Write> {
    output: BoundedWriter<W>,
    format: TranscriptFormat,
    snapshot: TranscriptSnapshot,
    last_sequence: Option<u64>,
    seen_cursors: BTreeSet<[u8; 32]>,
    pages: usize,
    messages: usize,
    complete: bool,
    failed: bool,
}

impl<W: Write> TranscriptWriter<W> {
    /// Start formatting a snapshot without buffering its messages.
    ///
    /// # Errors
    /// Returns an error for unsupported metadata or destination write failure.
    pub fn new(
        writer: W,
        format: TranscriptFormat,
        snapshot: &TranscriptSnapshot,
    ) -> io::Result<Self> {
        if snapshot.schema_version != 1 || snapshot.session_id.is_empty() {
            return Err(invalid("The transcript snapshot is unsupported."));
        }
        let mut result = Self {
            output: BoundedWriter {
                inner: writer,
                bytes: 0,
            },
            format,
            snapshot: snapshot.clone(),
            last_sequence: None,
            seen_cursors: BTreeSet::new(),
            pages: 0,
            messages: 0,
            complete: false,
            failed: false,
        };
        result.header()?;
        Ok(result)
    }

    /// Append one canonical page, including empty pages with continuation cursors.
    ///
    /// # Errors
    /// Rejects changed snapshots, unordered messages, stalled cursors, too many
    /// pages, output size overflow, and destination failures.
    pub fn write_page(&mut self, page: &TranscriptPage) -> io::Result<()> {
        if self.failed || self.complete || page.snapshot != self.snapshot {
            self.failed = true;
            return Err(invalid("The transcript snapshot changed during export."));
        }
        self.failed = true;
        if self.pages >= MAX_TRANSCRIPT_EXPORT_PAGES {
            return Err(invalid("The conversation exceeds the export page limit."));
        }
        if let Some(cursor) = &page.next_cursor
            && (cursor.is_empty()
                || cursor.len() > 16 * 1024
                || !self
                    .seen_cursors
                    .insert(Sha256::digest(cursor.as_bytes()).into()))
        {
            return Err(invalid("The transcript continuation did not advance."));
        }
        for message in &page.messages {
            if message.sequence > self.snapshot.stream_version
                || self
                    .last_sequence
                    .is_some_and(|sequence| message.sequence <= sequence)
            {
                return Err(invalid("The transcript message order is invalid."));
            }
            match self.format {
                TranscriptFormat::Json => {
                    if self.messages > 0 {
                        self.output.write_all(b",\n")?;
                    }
                    serde_json::to_writer(&mut self.output, message).map_err(io::Error::other)?;
                }
                TranscriptFormat::Markdown | TranscriptFormat::Text => {
                    let role = match message.role {
                        TranscriptRole::User => "User",
                        TranscriptRole::Assistant => "Assistant",
                    };
                    if self.format == TranscriptFormat::Markdown {
                        writeln!(
                            self.output,
                            "\n## {role}\n\n{}\n",
                            metadata(&message.created_at, true)
                        )?;
                    } else {
                        writeln!(
                            self.output,
                            "\n{role} — {}\n",
                            metadata(&message.created_at, false)
                        )?;
                    }
                    for (index, part) in message.parts.iter().enumerate() {
                        if index > 0 {
                            self.output.write_all(b"\n")?;
                        }
                        match part {
                            TranscriptPart::Text { text } => {
                                self.output.write_all(text.as_bytes())?
                            }
                            TranscriptPart::Attachment {
                                file_name,
                                media_type,
                                size_bytes,
                            } => {
                                writeln!(
                                    self.output,
                                    "[Attachment: {} ({}, {size_bytes} bytes); reference metadata only]",
                                    metadata(file_name, self.format == TranscriptFormat::Markdown),
                                    metadata(media_type, self.format == TranscriptFormat::Markdown)
                                )?;
                            }
                        }
                    }
                    self.output.write_all(b"\n")?;
                }
            }
            self.last_sequence = Some(message.sequence);
            self.messages += 1;
        }
        self.pages += 1;
        self.complete = page.next_cursor.is_none();
        self.failed = false;
        Ok(())
    }

    /// Finish only after the complete snapshot was read and flush its writer.
    ///
    /// # Errors
    /// Fails if a continuation remains or the destination cannot be written.
    pub fn finish(mut self) -> io::Result<W> {
        if self.failed || !self.complete {
            return Err(invalid(
                "The transcript is incomplete; no export was published.",
            ));
        }
        match self.format {
            TranscriptFormat::Json => self.output.write_all(b"\n],\"complete\":true}\n")?,
            TranscriptFormat::Markdown | TranscriptFormat::Text => {
                if self.messages == 0 {
                    self.output
                        .write_all(b"\nNo released user or assistant messages.\n")?;
                }
                self.output.write_all(b"\nEnd of snapshot.\n")?;
            }
        }
        self.output.flush()?;
        Ok(self.output.inner)
    }

    fn header(&mut self) -> io::Result<()> {
        if self.format == TranscriptFormat::Json {
            self.output
                .write_all(b"{\"schema_version\":1,\"snapshot\":")?;
            serde_json::to_writer(&mut self.output, &self.snapshot).map_err(io::Error::other)?;
            return self.output.write_all(b",\"messages\":[\n");
        }
        let title = self
            .snapshot
            .title
            .as_deref()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or("Conversation");
        let markdown = self.format == TranscriptFormat::Markdown;
        writeln!(
            self.output,
            "{}{}\n",
            if markdown { "# " } else { "" },
            metadata(title, markdown)
        )?;
        self.output.write_all(b"Released user and assistant messages. Tool activity and private agent context are excluded. Text attachment content already saved in messages is included. Attachment references contain metadata only; export does not fetch files or embed image bytes.\n")?;
        writeln!(
            self.output,
            "\nSnapshot: {}",
            metadata(&self.snapshot.exported_at, markdown)
        )?;
        match self.snapshot.activity {
            TranscriptActivity::Active => self
                .output
                .write_all(b"A run was active at snapshot time. Later content is excluded.\n"),
            TranscriptActivity::Unknown => self.output.write_all(
                b"Run activity at snapshot time is unavailable. Later content is excluded.\n",
            ),
            TranscriptActivity::Idle => Ok(()),
        }
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn metadata(value: &str, markdown: bool) -> String {
    let mut output = String::new();
    for word in value.split_whitespace() {
        if !output.is_empty() {
            output.push(' ');
        }
        for character in word.chars().filter(|character| !character.is_control()) {
            if markdown && "\\`*_{}[]()<>#+.!|~".contains(character) {
                output.push('\\');
            }
            output.push(character);
        }
    }
    output
}

/// Produce a portable bounded filename, never a path, from untrusted metadata.
pub fn transcript_filename(title: &str, stable_id: &str, format: TranscriptFormat) -> String {
    let mut name = String::new();
    for character in title.chars() {
        let character = if character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
            || matches!(character, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}')
        {
            ' '
        } else {
            character
        };
        if name.len() + character.len_utf8() > 160 {
            break;
        }
        name.push(character);
    }
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = name.trim_matches(['.', ' ']);
    let device = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(device.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (device.len() == 4
            && (device.starts_with("COM") || device.starts_with("LPT"))
            && device.as_bytes()[3].is_ascii_digit());
    let name = if name.is_empty() || reserved {
        "conversation"
    } else {
        name
    };
    let identity: String = stable_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(12)
        .collect();
    if identity.is_empty() {
        format!("{name}.{}", format.extension())
    } else {
        format!("{name}-{identity}.{}", format.extension())
    }
}

#[cfg(test)]
mod tests;
