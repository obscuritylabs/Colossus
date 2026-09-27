/// Displayable MIME types can still be explicit downloads, including HTML and PDF.
pub(crate) fn allows_response(can_display: bool, disposition: Option<&str>) -> bool {
    can_display
        && !disposition.is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("attachment"))
        })
}

#[cfg(test)]
mod tests {
    use super::allows_response;

    #[test]
    fn attachments_are_denied_even_when_the_mime_type_is_displayable() {
        for header in [
            "attachment",
            "attachment; filename=page.html",
            " ATTACHMENT ; filename=report.pdf",
            "Attachment; filename=notes.txt",
        ] {
            assert!(!allows_response(true, Some(header)));
            assert!(!allows_response(false, Some(header)));
        }
    }

    #[test]
    fn inline_display_requires_a_supported_mime_type() {
        for header in [
            None,
            Some("inline"),
            Some("inline; filename=attachment.txt"),
        ] {
            assert!(allows_response(true, header));
            assert!(!allows_response(false, header));
        }
    }
}
