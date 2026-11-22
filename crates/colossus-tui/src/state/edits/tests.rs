use super::super::{Composer, LARGE_PASTE_CHAR_THRESHOLD, SavedComposerDraft};
use super::{ComposerEdits, EditDirection, MAX_EDIT_BYTES, MAX_EDIT_FRAMES};

#[test]
fn restores_multiline_text_and_cursor_at_grapheme_boundaries() {
    let mut composer = Composer::default();
    composer.insert("first\n\u{301}界");
    let second_line = "first\n".len();
    composer.move_to(second_line);
    composer.insert("x");

    assert!(composer.undo_edit());
    assert_eq!(composer.draft, "first\n\u{301}界");
    assert_eq!(composer.cursor, second_line);

    assert!(composer.redo_edit());
    assert_eq!(composer.draft, "first\nx\u{301}界");
    // Inserting x joined the following mark to it. The restored cursor must
    // land before the whole grapheme so the next delete removes both chars.
    assert_eq!(composer.cursor, second_line);
    composer.delete();
    assert_eq!(composer.draft, "first\n界");
    assert!(composer.undo_edit());
    assert_eq!(composer.draft, "first\nx\u{301}界");

    composer.backspace();
    assert_eq!(composer.draft, "firstx\u{301}界");
    assert!(composer.undo_edit());
    assert_eq!(composer.draft, "first\nx\u{301}界");
    assert_eq!(composer.cursor, second_line);
}

#[test]
fn large_paste_history_restores_exact_ranges_without_rebinding_typed_text() {
    let mut composer = Composer::default();
    let chars = LARGE_PASTE_CHAR_THRESHOLD + 1;
    let payload = "é".repeat(chars);
    let label = format!("[Pasted Content {chars} chars]");
    let prefix = format!("{label} before ");
    composer.insert(&prefix);
    composer.insert_paste(payload.clone());
    let both_labels = format!("{prefix}{label}");

    assert!(composer.undo_edit());
    assert_eq!(composer.draft, prefix);
    assert!(composer.pending_pastes.is_empty());
    assert!(composer.redo_edit());
    assert_eq!(composer.draft, both_labels);
    assert_eq!(composer.pending_pastes[0].start, prefix.len());
    assert_eq!(composer.pending_pastes[0].end, both_labels.len());

    composer.backspace();
    assert!(composer.pending_pastes.is_empty());
    composer.insert("]");
    assert_eq!(composer.draft, both_labels);
    assert!(composer.pending_pastes.is_empty());
    assert!(composer.undo_edit());
    assert!(composer.pending_pastes.is_empty());
    assert!(composer.undo_edit());
    assert_eq!(composer.take(), format!("{prefix}{payload}"));
    assert!(!composer.has_edit_history());
}

#[test]
fn new_branch_discards_redo_while_no_op_edits_keep_it() {
    let mut composer = Composer::default();
    composer.insert("alpha");
    composer.insert(" beta");
    assert!(composer.undo_edit());
    composer.delete();
    composer.insert("");
    composer.move_to(0);
    composer.backspace();
    assert!(composer.redo_edit());
    assert_eq!(composer.draft, "alpha beta");

    assert!(composer.undo_edit());
    composer.move_to(composer.draft.len());
    composer.insert(" branch");
    assert_eq!(composer.draft, "alpha branch");
    assert!(!composer.redo_edit());
}

#[test]
fn recalling_submitted_input_preserves_live_history_and_bindings() {
    let mut composer = Composer::default();
    let payload = "x".repeat(LARGE_PASTE_CHAR_THRESHOLD + 1);
    composer.insert_paste(payload);
    let marker = composer.draft.clone();
    assert!(composer.undo_edit());
    assert!(composer.has_edit_history());

    composer.save_history_draft();
    composer.show_history("submitted input".into(), 0);
    assert!(!composer.has_edit_history());
    assert!(!composer.undo_edit());
    assert!(!composer.redo_edit());
    composer.restore_history_draft();
    assert!(composer.redo_edit());

    composer.save_history_draft();
    composer.show_history(marker.clone(), 0);
    composer.insert(" edited");
    assert!(composer.pending_pastes.is_empty());
    assert!(composer.undo_edit());
    assert_eq!(composer.draft, marker);
    assert_eq!(composer.pending_pastes.len(), 1);
    assert!(composer.redo_edit());
    assert_eq!(composer.take(), format!("{marker} edited"));
    assert!(!composer.has_edit_history());
}

#[test]
fn draft_replacement_and_discard_clear_both_edit_stacks() {
    let mut composer = Composer::default();
    composer.insert("draft");
    composer.insert(" edit");
    assert!(composer.undo_edit());
    composer.set("selected input".into());
    assert!(!composer.has_edit_history());
    assert!(!composer.undo_edit());
    assert!(!composer.redo_edit());

    composer.insert(" changed");
    assert!(composer.undo_edit());
    composer.clear();
    assert!(composer.draft.is_empty());
    assert!(!composer.has_edit_history());
}

#[test]
fn caps_combined_frame_count_across_both_stacks() {
    let mut composer = Composer::default();
    let evicted = 6;
    for _ in 0..MAX_EDIT_FRAMES + evicted {
        composer.insert("x");
    }
    for redo_frames in 1..=MAX_EDIT_FRAMES {
        assert!(composer.undo_edit());
        assert_eq!(composer.edits.redo.len(), redo_frames);
        assert_eq!(composer.edits.undo.len(), MAX_EDIT_FRAMES - redo_frames);
    }
    assert!(!composer.undo_edit());
    assert_eq!(composer.draft.len(), evicted);
    for _ in 0..MAX_EDIT_FRAMES {
        assert!(composer.redo_edit());
    }
    assert!(!composer.redo_edit());
}

#[test]
fn accounts_for_paste_payloads_and_evicts_older_frames_by_byte_size() {
    let mut composer = Composer::default();
    let payload = "界".repeat(MAX_EDIT_BYTES / 9);
    composer.insert_paste(payload.clone());
    composer.insert_paste(payload);
    composer.insert("!");
    assert_eq!(composer.edits.undo.len(), 1);
    assert!(composer.edits.retained_bytes() > MAX_EDIT_BYTES / 2);
    assert!(composer.edits.retained_bytes() <= MAX_EDIT_BYTES);

    assert!(composer.undo_edit());
    assert!(!composer.undo_edit());
    assert!(composer.edits.retained_bytes() <= MAX_EDIT_BYTES);
    assert!(composer.redo_edit());
    assert!(composer.draft.ends_with('!'));
}

#[test]
fn oversized_snapshots_are_rejected_before_allocation() {
    let mut edits = ComposerEdits::default();
    let empty_bytes = SavedComposerDraft::retained_size("", &[]);
    edits.push(
        EditDirection::Undo,
        empty_bytes,
        SavedComposerDraft::default,
    );
    edits.push(EditDirection::Undo, MAX_EDIT_BYTES + 1, || {
        panic!("over-budget snapshots must not be created")
    });
    assert!(edits.is_empty());
}
