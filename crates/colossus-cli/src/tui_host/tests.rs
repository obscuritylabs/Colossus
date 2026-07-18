use super::*;

fn session(
    id: &str,
    message_count: u64,
    preview: Option<&str>,
    title: Option<&str>,
) -> SessionSummary {
    SessionSummary {
        id: id.into(),
        title: title.map(str::to_owned),
        created_at: "2026-07-18T01:00:00Z".into(),
        updated_at: "2026-07-18T01:41:50.425459Z".into(),
        message_count,
        last_run_id: None,
        last_user_preview: preview.map(str::to_owned),
    }
}

#[test]
fn session_picker_choice_prioritizes_human_context_over_full_ids() {
    let choice = session_picker_choice(&session(
        "019f72e2-c116-7fa3-b668-5778378e114f",
        12,
        Some("How can we\nget   sccache working locally?"),
        Some("Build speed"),
    ));
    assert_eq!(
        choice,
        "Build speed · 12 msgs · 019f72e2 · 2026-07-18 01:41Z\nHow can we get sccache working locally?"
    );
    assert!(!choice.contains("c116-7fa3"));
}

#[test]
fn resume_picker_excludes_empty_sessions_before_applying_the_limit() {
    let sessions = resumable_sessions(
        vec![
            session("empty", 0, None, None),
            session("first", 2, Some("first"), None),
            session("second", 3, Some("second"), None),
        ],
        1,
    );
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "first");
}
