//! Allowlisted transcript translation shared by the server and Rust SDK.
use colossus_api::{
    ApiError, ApiErrorReason, ApiResult, TranscriptActivity, TranscriptMessage, TranscriptPage,
    TranscriptPart, TranscriptRole, TranscriptSnapshot,
};
use colossus_api_proto::v1alpha1 as proto;
use prost::Message as _;

/// Encode original released content without introducing private message fields.
pub fn page_to_proto(value: TranscriptPage) -> proto::ReadSessionTranscriptResponse {
    let snapshot = value.snapshot;
    proto::ReadSessionTranscriptResponse {
        snapshot: Some(proto::TranscriptSnapshot {
            schema_version: snapshot.schema_version,
            session_id: snapshot.session_id,
            title: snapshot.title,
            stream_version: snapshot.stream_version,
            exported_at: snapshot.exported_at,
            activity: match snapshot.activity {
                TranscriptActivity::Active => proto::TranscriptActivity::Active,
                TranscriptActivity::Idle => proto::TranscriptActivity::Idle,
                TranscriptActivity::Unknown => proto::TranscriptActivity::Unknown,
            } as i32,
        }),
        messages: value.messages.into_iter().map(message_to_proto).collect(),
        page: Some(proto::PageResponse {
            next_page_token: value.next_cursor.unwrap_or_default(),
        }),
    }
}

fn message_to_proto(value: TranscriptMessage) -> proto::TranscriptMessage {
    proto::TranscriptMessage {
        sequence: value.sequence,
        run_id: value.run_id,
        created_at: value.created_at,
        role: match value.role {
            TranscriptRole::User => proto::TranscriptMessageRole::User,
            TranscriptRole::Assistant => proto::TranscriptMessageRole::Assistant,
        } as i32,
        parts: value
            .parts
            .into_iter()
            .map(|part| proto::TranscriptPart {
                content: Some(match part {
                    TranscriptPart::Text { text } => proto::transcript_part::Content::Text(text),
                    TranscriptPart::Attachment {
                        file_name,
                        media_type,
                        size_bytes,
                    } => proto::transcript_part::Content::Attachment(proto::TranscriptAttachment {
                        file_name,
                        media_type,
                        size_bytes,
                    }),
                }),
            })
            .collect(),
    }
}

/// Decode only known public roles/parts and bounded, internally coherent pages.
pub fn page_from_proto(value: proto::ReadSessionTranscriptResponse) -> ApiResult<TranscriptPage> {
    if value.encoded_len() > 2 * 1024 * 1024 || value.messages.len() > 32 {
        return Err(invalid());
    }
    let snapshot = value.snapshot.ok_or_else(invalid)?;
    if snapshot.schema_version != 1
        || snapshot.stream_version == 0
        || !identity(&snapshot.session_id)
        || !timestamp(&snapshot.exported_at)
        || snapshot
            .title
            .as_ref()
            .is_some_and(|title| title.len() > 4096)
    {
        return Err(invalid());
    }
    let activity = match proto::TranscriptActivity::try_from(snapshot.activity) {
        Ok(proto::TranscriptActivity::Active) => TranscriptActivity::Active,
        Ok(proto::TranscriptActivity::Idle) => TranscriptActivity::Idle,
        Ok(proto::TranscriptActivity::Unknown) => TranscriptActivity::Unknown,
        _ => return Err(invalid()),
    };
    let next_cursor = value.page.ok_or_else(invalid)?.next_page_token;
    if next_cursor.len() > 4096 || next_cursor.chars().any(char::is_control) {
        return Err(invalid());
    }
    let mut previous = 0;
    let mut messages = Vec::with_capacity(value.messages.len());
    for message in value.messages {
        if message.sequence <= previous
            || message.sequence > snapshot.stream_version
            || !identity(&message.run_id)
            || !timestamp(&message.created_at)
            || message.parts.is_empty()
            || message.parts.len() > 65_536
        {
            return Err(invalid());
        }
        previous = message.sequence;
        let role = match proto::TranscriptMessageRole::try_from(message.role) {
            Ok(proto::TranscriptMessageRole::User) => TranscriptRole::User,
            Ok(proto::TranscriptMessageRole::Assistant) => TranscriptRole::Assistant,
            _ => return Err(invalid()),
        };
        let parts = message
            .parts
            .into_iter()
            .map(|part| match part.content {
                Some(proto::transcript_part::Content::Text(text)) => {
                    Ok(TranscriptPart::Text { text })
                }
                Some(proto::transcript_part::Content::Attachment(attachment))
                    if attachment.file_name.len() <= 4096 && attachment.media_type.len() <= 256 =>
                {
                    Ok(TranscriptPart::Attachment {
                        file_name: attachment.file_name,
                        media_type: attachment.media_type,
                        size_bytes: attachment.size_bytes,
                    })
                }
                _ => Err(invalid()),
            })
            .collect::<ApiResult<Vec<_>>>()?;
        messages.push(TranscriptMessage {
            sequence: message.sequence,
            run_id: message.run_id,
            created_at: message.created_at,
            role,
            parts,
        });
    }
    Ok(TranscriptPage {
        snapshot: TranscriptSnapshot {
            schema_version: snapshot.schema_version,
            session_id: snapshot.session_id,
            title: snapshot.title,
            stream_version: snapshot.stream_version,
            exported_at: snapshot.exported_at,
            activity,
        },
        messages,
        next_cursor: (!next_cursor.is_empty()).then_some(next_cursor),
    })
}

fn identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

fn timestamp(value: &str) -> bool {
    if value.len() > 64 {
        return false;
    }
    value.parse::<prost_types::Timestamp>().is_ok_and(|stamp| {
        (-62_135_596_800..=253_402_300_799).contains(&stamp.seconds)
            && (0..1_000_000_000).contains(&stamp.nanos)
    })
}

fn invalid() -> ApiError {
    ApiError::failed_precondition(
        ApiErrorReason::InternalInvariant,
        "invalid transcript response",
    )
}

#[cfg(test)]
mod tests;
