use super::*;
use colossus_contracts::{
    MAX_COMBINED_DOCUMENT_INPUT_BYTES, MAX_DOCUMENT_INPUT_BYTES, ModelFileReference,
};

fn document(index: usize, size_bytes: u64) -> ModelFileReference {
    ModelFileReference {
        artifact_id: format!("artifact-{index:064x}"),
        file_name: format!("document-{index}.pdf"),
        media_type: "application/pdf".into(),
        size_bytes,
        sha256: format!("{index:012x}{}", "a".repeat(52)),
    }
}

fn document_message(files: impl IntoIterator<Item = ModelFileReference>) -> ModelMessage {
    let mut message = message(ModelMessageRole::User, "");
    message.content = ModelContent::Parts(
        files
            .into_iter()
            .map(|file| ModelContentPart::File { file })
            .collect(),
    );
    message
}

fn replay_request(messages: Vec<ModelMessage>) -> ContextPreparationRequest {
    let mut request = preparation_request(messages, false);
    request.route.capabilities.document_inputs = true;
    request.route.limits.input_budget_tokens = 100_000;
    request.continuation = Some(ProviderContinuationView {
        replay: true,
        id: "document-replay".into(),
        covered_count: 1,
        context_binding_hash: "f".repeat(64),
        reserved_tokens: 100,
        bytes: 300,
    });
    request
}

#[test]
fn newest_document_turn_checks_count_individual_and_combined_bytes() {
    let valid = document_message((0..4).map(|index| document(index, 8 * 1_048_576)));
    validate_newest_document_turn(&[valid]).unwrap();
    for files in [
        (0..5).map(|index| document(index, 1)).collect::<Vec<_>>(),
        vec![document(0, MAX_DOCUMENT_INPUT_BYTES + 1)],
        vec![document(0, 0)],
        vec![
            document(0, MAX_DOCUMENT_INPUT_BYTES),
            document(1, MAX_DOCUMENT_INPUT_BYTES),
            document(2, 1),
        ],
    ] {
        let error = validate_newest_document_turn(&[document_message(files)]).unwrap_err();
        assert!(error.to_string().contains("newest user turn exceeds"));
    }
}

#[test]
fn document_compaction_keeps_newest_references_and_original_part_order() {
    let mut old = document_message([document(0, 1), document(1, 1)]);
    let ModelContent::Parts(parts) = &mut old.content else {
        unreachable!()
    };
    parts.insert(
        1,
        ModelContentPart::Text {
            text: "between documents".into(),
        },
    );
    let canonical = vec![
        old,
        document_message((2..6).map(|index| document(index, 1))),
    ];
    let unchanged = canonical.clone();
    let prepared = compact_excess_documents(&canonical);
    assert_eq!(canonical, unchanged);
    assert_eq!(canonical.iter().flat_map(|m| m.content.files()).count(), 6);
    assert_eq!(prepared.iter().flat_map(|m| m.content.files()).count(), 4);
    assert_eq!(prepared[1], canonical[1]);
    let ModelContent::Parts(parts) = &prepared[0].content else {
        panic!("multipart")
    };
    assert!(matches!(&parts[0], ModelContentPart::Text { text }
        if text == &document_compaction_marker(&document(0, 1))));
    assert!(matches!(&parts[1], ModelContentPart::Text { text } if text == "between documents"));
    assert!(matches!(&parts[2], ModelContentPart::Text { text }
        if text == &document_compaction_marker(&document(1, 1))));
    let markers = prepared[0].content.plain_text();
    assert!(markers.contains("sha256:000000000001"));
    assert!(!markers.contains("artifact-"));
}

#[test]
fn document_and_image_compaction_have_independent_count_and_byte_budgets() {
    let documents = document_message([
        document(0, MAX_DOCUMENT_INPUT_BYTES),
        document(1, MAX_DOCUMENT_INPUT_BYTES),
    ]);
    let images = image_message((0..16).map(|index| image(index, 2 * 1_048_576)));
    let canonical = vec![images, documents];
    let prepared = compact_excess_documents(&compact_excess_images(&canonical));
    assert_eq!(prepared, canonical);
    assert_eq!(
        prepared
            .iter()
            .flat_map(|m| m.content.files())
            .map(|f| f.size_bytes)
            .sum::<u64>(),
        MAX_COMBINED_DOCUMENT_INPUT_BYTES
    );

    let mut over_document_budget = canonical.clone();
    over_document_budget.push(document_message([document(2, 1)]));
    let prepared = compact_excess_documents(&compact_excess_images(&over_document_budget));
    assert_eq!(prepared[0], canonical[0]);
    assert_eq!(prepared[1].content.files().count(), 1);
    assert!(prepared[1].content.plain_text().contains("document-0.pdf"));
    assert_eq!(prepared[2], over_document_budget[2]);

    let mut over_image_budget = canonical.clone();
    over_image_budget.push(image_message([image(16, 1)]));
    let prepared = compact_excess_documents(&compact_excess_images(&over_image_budget));
    assert_eq!(prepared[1], canonical[1]);
    assert_eq!(prepared.iter().flat_map(|m| m.content.images()).count(), 16);
}

#[test]
fn snapshot_and_summary_bounds_keep_document_markers_in_media_order() {
    let first = document(0, 1);
    let last = document(1, 1);
    let mut old = message(ModelMessageRole::User, "");
    old.content = ModelContent::Parts(vec![
        ModelContentPart::Text {
            text: document_compaction_marker(&first),
        },
        ModelContentPart::Image { image: image(0, 1) },
        ModelContentPart::File { file: last.clone() },
    ]);
    let messages = vec![old, message(ModelMessageRole::User, "newest guidance")];
    let mut snapshot = deterministic_snapshot("session-1", &messages[..1]);
    snapshot.summary = "long summary ".repeat(1_000);
    let mut prepared = apply_snapshot(&snapshot, &messages);
    bound_summary_to_target("unknown", "", &mut prepared, &[], 1);
    bound_summary_to_byte_limit("", &mut prepared, &[], 1);
    let summary = prepared[0].content.plain_text();
    let first_at = summary.find(&document_compaction_marker(&first)).unwrap();
    let image_at = summary
        .find(&image_compaction_marker(&image(0, 1)))
        .unwrap();
    let last_at = summary.find(&document_compaction_marker(&last)).unwrap();
    assert!(first_at < image_at && image_at < last_at);
    assert!(summary.contains("[Colossus context snapshot]"));
    assert!(!summary.contains("artifact-"));
    assert_eq!(prepared[1], messages[1]);
    assert!(prepared.iter().all(|m| m.content.files().next().is_none()));
}

#[test]
fn document_estimate_adds_a_bounded_byte_allowance_to_metadata() {
    for bytes in [1, 6_144, 6_145, MAX_DOCUMENT_INPUT_BYTES] {
        let messages = vec![document_message([document(0, bytes)])];
        let metadata_bytes = serde_json::to_vec(&messages[0]).unwrap().len();
        let expected = u64::try_from((metadata_bytes + 2).div_ceil(3)).unwrap()
            + 16
            + 8
            + bytes.div_ceil(3).max(2_048);
        assert_eq!(
            estimate_tokens_for_model("unknown", "", &messages, &[]),
            expected
        );
    }
}

#[tokio::test]
async fn ordinary_preparation_compacts_old_documents_without_mutating_session_history() {
    let provider = Arc::new(SummaryProvider {
        output: None,
        calls: AtomicUsize::new(0),
    });
    let (_, sessions, snapshots, service) = fixture(ContextConfig::default(), provider.clone());
    let messages = vec![
        document_message([document(0, 1)]),
        document_message((1..5).map(|index| document(index, 1))),
    ];
    for original in &messages {
        sessions
            .append_message("session-1", "run-1", original.clone(), user_actor())
            .unwrap();
    }
    let mut request = preparation_request(messages.clone(), false);
    request.route.limits.input_budget_tokens = 100_000;
    request.route.capabilities.document_inputs = true;
    let prepared = service.prepare(request).await.unwrap();
    assert!(
        prepared.messages[0]
            .content
            .plain_text()
            .contains("document-0.pdf")
    );
    assert_eq!(prepared.messages[1], messages[1]);
    assert_eq!(
        sessions
            .list_messages("session-1")
            .unwrap()
            .into_iter()
            .map(|m| m.message)
            .collect::<Vec<_>>(),
        messages
    );
    assert!(snapshots.list("session-1").unwrap().is_empty());
    assert_eq!(provider.calls.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn newest_document_turn_is_rejected_before_compaction_or_provider_calls() {
    let provider = Arc::new(SummaryProvider {
        output: None,
        calls: AtomicUsize::new(0),
    });
    let (_, _, snapshots, service) = fixture(ContextConfig::default(), provider.clone());
    let error = service
        .prepare(preparation_request(
            vec![document_message((0..5).map(|index| document(index, 1)))],
            false,
        ))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("newest user turn exceeds"));
    let error = service
        .prepare(preparation_request(
            vec![document_message([document(0, 16_384)])],
            false,
        ))
        .await
        .unwrap_err();
    assert!(matches!(error, ContextError::BudgetExceeded(_)));
    assert!(snapshots.list("session-1").unwrap().is_empty());
    assert_eq!(provider.calls.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn frozen_replay_preserves_document_prefix_and_new_human_guidance_exactly() {
    let provider = Arc::new(SummaryProvider {
        output: None,
        calls: AtomicUsize::new(0),
    });
    let (_, _, snapshots, service) = fixture(ContextConfig::default(), provider.clone());
    let service = service.with_memory_retriever(Arc::new(UnexpectedReplayRetrieval));
    let messages = vec![
        message(ModelMessageRole::System, "frozen bindings"),
        document_message([document(0, 1)]),
        message(ModelMessageRole::Assistant, "retained assistant prefix"),
        document_message([document(1, 1)]),
    ];
    let prepared = service
        .prepare_replay(replay_request(messages.clone()))
        .await
        .unwrap();
    assert_eq!(prepared.messages, messages);
    assert_eq!(prepared.continuation_id.as_deref(), Some("document-replay"));
    assert_eq!(prepared.context_binding_hash, "f".repeat(64));
    assert!(snapshots.list("session-1").unwrap().is_empty());
    assert_eq!(provider.calls.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn replay_refuses_to_compact_an_over_budget_document_prefix() {
    let provider = Arc::new(SummaryProvider {
        output: None,
        calls: AtomicUsize::new(0),
    });
    let (_, _, snapshots, service) = fixture(ContextConfig::default(), provider.clone());
    let service = service.with_memory_retriever(Arc::new(UnexpectedReplayRetrieval));
    let request = replay_request(vec![
        document_message((0..4).map(|index| document(index, 1))),
        document_message([document(4, 1)]),
    ]);
    for error in [
        service.prepare_replay(request.clone()).await.unwrap_err(),
        service.prepare(request).await.unwrap_err(),
    ] {
        assert!(
            error
                .to_string()
                .contains("unchanged provider replay prefix")
        );
    }
    assert!(snapshots.list("session-1").unwrap().is_empty());
    assert_eq!(provider.calls.load(Ordering::Acquire), 0);
}
