//! Channel admission for the explicitly selected functional browser edition.

pub(crate) fn admitted(profile: &str, channel: Option<&str>, development: bool) -> bool {
    profile == "release"
        && matches!(
            channel,
            Some("stable" | "developer_preview" | "validation_only")
        )
        && !development
}
