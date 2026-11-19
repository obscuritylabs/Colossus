//! Human-owned transcript output; source reads never start provider work.

use clap::ValueEnum;
use colossus_contracts::TranscriptPage;
use colossus_presentation::{TranscriptFormat, TranscriptWriter};
use colossus_runtime::Runtime;
use colossus_worker::{WorkerClient, WorkerOperation};
use std::{
    io::{self, Seek as _, Write as _},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub(super) enum TranscriptFormatArg {
    #[default]
    Markdown,
    Text,
    Json,
}

impl From<TranscriptFormatArg> for TranscriptFormat {
    fn from(value: TranscriptFormatArg) -> Self {
        match value {
            TranscriptFormatArg::Markdown => Self::Markdown,
            TranscriptFormatArg::Text => Self::Text,
            TranscriptFormatArg::Json => Self::Json,
        }
    }
}

pub(super) enum TranscriptSource<'a> {
    Embedded(&'a Runtime),
    Worker(&'a WorkerClient),
}

impl TranscriptSource<'_> {
    async fn page(
        &self,
        session_id: &str,
        cursor: Option<String>,
    ) -> Result<TranscriptPage, String> {
        match self {
            Self::Embedded(runtime) => runtime
                .read_session_transcript(session_id, cursor.as_deref(), 100)
                .map_err(|error| error.to_string()),
            Self::Worker(client) => {
                let result = client
                    .call(WorkerOperation::SessionTranscriptPage {
                        session_id: session_id.to_owned(),
                        cursor,
                        page_size: 100,
                    })
                    .await
                    .map_err(|error| error.to_string())?;
                serde_json::from_value(result)
                    .map_err(|_| "The worker returned an invalid transcript page.".into())
            }
        }
    }
}

enum Destination {
    File {
        staged: tempfile::NamedTempFile,
        path: PathBuf,
    },
    Stdout(std::fs::File),
}

impl Destination {
    fn new(path: &Path) -> Result<Self, String> {
        if path == Path::new("-") {
            return tempfile::tempfile()
                .map(Self::Stdout)
                .map_err(|_| "The transcript staging file could not be created.".into());
        }
        if path.as_os_str().is_empty() || path.file_name().is_none() {
            return Err("Choose an output file for the transcript.".into());
        }
        match std::fs::symlink_metadata(path) {
            Ok(_) => return Err("The output already exists. Choose a new file.".into()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err("The output destination is unavailable.".into()),
        }
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let staged = tempfile::NamedTempFile::new_in(parent)
            .map_err(|_| "The transcript staging file could not be created.".to_owned())?;
        Ok(Self::File {
            staged,
            path: path.to_owned(),
        })
    }

    fn writer(&mut self) -> &mut std::fs::File {
        match self {
            Self::File { staged, .. } => staged.as_file_mut(),
            Self::Stdout(file) => file,
        }
    }

    fn publish(mut self) -> Result<(), String> {
        self.writer()
            .sync_all()
            .map_err(|_| "The transcript could not be saved.".to_owned())?;
        match self {
            Self::File { staged, path } => {
                staged.persist_noclobber(path).map(|_| ()).map_err(|_| {
                    "The transcript could not be published. Choose a new output file and retry."
                        .into()
                })
            }
            Self::Stdout(mut file) => {
                file.rewind()
                    .map_err(|_| "The transcript staging file could not be read.".to_owned())?;
                let mut output = io::stdout().lock();
                io::copy(&mut file, &mut output)
                    .and_then(|_| output.flush())
                    .map_err(|_| "The transcript could not be written to stdout.".into())
            }
        }
    }
}

pub(super) async fn export_transcript(
    source: TranscriptSource<'_>,
    session_id: &str,
    output: &Path,
    format: TranscriptFormatArg,
) -> Result<(), String> {
    export_transcript_controlled(
        source,
        session_id,
        output,
        format,
        &colossus_ports::RunControl::default(),
    )
    .await
}

pub(super) async fn export_transcript_controlled(
    source: TranscriptSource<'_>,
    session_id: &str,
    output: &Path,
    format: TranscriptFormatArg,
    control: &colossus_ports::RunControl,
) -> Result<(), String> {
    check_cancelled(control)?;
    let mut destination = Destination::new(output)?;
    let mut page = source.page(session_id, None).await?;
    if page.snapshot.session_id != session_id {
        return Err("The transcript belongs to a different session.".into());
    }
    let mut writer = TranscriptWriter::new(destination.writer(), format.into(), &page.snapshot)
        .map_err(|error| error.to_string())?;
    loop {
        check_cancelled(control)?;
        writer
            .write_page(&page)
            .map_err(|error| error.to_string())?;
        let Some(cursor) = page.next_cursor.take() else {
            break;
        };
        page = source.page(session_id, Some(cursor)).await?;
    }
    writer.finish().map_err(|error| error.to_string())?;
    check_cancelled(control)?;
    destination.publish()
}

fn check_cancelled(control: &colossus_ports::RunControl) -> Result<(), String> {
    if control.is_cancelled() {
        Err("Conversation export cancelled. Nothing was saved.".into())
    } else {
        Ok(())
    }
}

pub(super) fn interactive_export_arguments(
    arguments: &str,
) -> Result<(PathBuf, TranscriptFormatArg), String> {
    let mut arguments = export_fields(arguments)?;
    let mut format = TranscriptFormatArg::Markdown;
    if let Some(index) = arguments.iter().position(|argument| argument == "--format") {
        if index + 1 >= arguments.len() {
            return Err(export_usage());
        }
        format = match arguments.remove(index + 1).as_str() {
            "markdown" => TranscriptFormatArg::Markdown,
            "text" => TranscriptFormatArg::Text,
            "json" => TranscriptFormatArg::Json,
            _ => return Err(export_usage()),
        };
        arguments.remove(index);
    }
    if arguments.len() != 1
        || arguments[0].is_empty()
        || arguments[0] == "-"
        || arguments[0].chars().any(char::is_control)
    {
        return Err(export_usage());
    }
    Ok((PathBuf::from(arguments.remove(0)), format))
}

fn export_usage() -> String {
    "Usage: /session export PATH [--format markdown|text|json] (or /export PATH). Quote paths containing spaces.".into()
}

fn export_fields(input: &str) -> Result<Vec<String>, String> {
    if input.len() > 64 * 1024 || input.chars().any(char::is_control) {
        return Err(export_usage());
    }
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quote = None;
    for character in input.chars() {
        if quote == Some(character) {
            quote = None;
        } else if quote.is_none() && matches!(character, '\'' | '"') {
            quote = Some(character);
        } else if quote.is_none() && character.is_whitespace() {
            if !field.is_empty() {
                fields.push(std::mem::take(&mut field));
            }
        } else {
            // Backslashes are ordinary filename bytes, including Windows paths.
            field.push(character);
        }
    }
    if quote.is_some() {
        return Err(export_usage());
    }
    if !field.is_empty() {
        fields.push(field);
    }
    Ok(fields)
}

pub(super) fn interactive_export_line(line: &str) -> Option<&str> {
    ["/session export", "/export"]
        .into_iter()
        .find_map(|prefix| {
            line.strip_prefix(prefix)
                .filter(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
        })
}

pub(super) fn interactive_export_status() -> colossus_presentation::PresentationDocument {
    colossus_presentation::PresentationDocument::from_block(
        colossus_presentation::PresentationBlock::Text("Conversation exported.".into()),
    )
}

#[cfg(test)]
mod tests;
