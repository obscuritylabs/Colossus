use super::*;
use colossus_contracts::ModelFileReference;

fn document_reference() -> ModelFileReference {
    ModelFileReference {
        artifact_id: format!("artifact-{}", "a".repeat(64)),
        file_name: "proposal.pdf".into(),
        media_type: "application/pdf".into(),
        size_bytes: 2048,
        sha256: "b".repeat(64),
    }
}

fn resumed(content: ModelContent) -> TuiState {
    let mut source = snapshot();
    source.transcript.messages[0].message.role = ModelMessageRole::User;
    source.transcript.messages[0].message.content = content;
    TuiState::from_snapshot(source)
}

#[test]
fn resumed_document_metadata_preserves_mixed_multipart_order() {
    let image = image_reference(0);
    let state = resumed(ModelContent::Parts(vec![
        ModelContentPart::Text {
            text: "Before".into(),
        },
        ModelContentPart::File {
            file: document_reference(),
        },
        ModelContentPart::Image {
            image: image.clone(),
        },
        ModelContentPart::Text {
            text: "After".into(),
        },
    ]));
    let blocks = &state.transcript[0].document.blocks;
    assert_eq!(blocks.len(), 4);
    assert_eq!(blocks[0], PresentationBlock::Markdown("Before".into()));
    assert!(matches!(&blocks[1], PresentationBlock::KeyValue(fields)
        if fields.iter().any(|(label, value)| label == "Document" && value == "proposal.pdf")));
    assert_eq!(blocks[2], PresentationBlock::Image(image));
    assert_eq!(blocks[3], PresentationBlock::Markdown("After".into()));
}

#[test]
fn document_only_history_remains_visible_as_accessible_metadata() {
    let mut state = resumed(ModelContent::Parts(vec![ModelContentPart::File {
        file: document_reference(),
    }]));
    assert_eq!(state.transcript.len(), 1);
    let mut terminal = Terminal::new(TestBackend::new(100, 28)).expect("test terminal");
    terminal
        .draw(|frame| render(frame, &mut state, 0, ScreenMode::Alternate))
        .expect("render document metadata");
    let visible = terminal.backend().to_string();
    for label in ["proposal.pdf", "application/pdf", "2048 bytes"] {
        assert!(visible.contains(label), "{visible}");
    }
    assert!(!visible.contains(&document_reference().artifact_id));
    assert!(!visible.contains(&document_reference().sha256));
}
