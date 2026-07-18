use super::*;

/// Shared lifecycle, citation, validation, and reconstruction checks for research adapters.
pub fn assert_research_repository_conformance<F>(factory: F)
where
    F: Fn() -> Box<dyn ResearchRepository>,
{
    let repository = factory();
    assert!(
        repository
            .get_run("research-conformance")
            .expect("missing run")
            .is_none()
    );
    let mut run = ResearchRun {
        id: "research-conformance".into(),
        session_id: "session-conformance".into(),
        question: "What is reconstructed?".into(),
        depth: ResearchDepth::Standard,
        source_kinds: vec![ResearchSourceKind::Repo],
        status: ResearchStatus::Running,
        queries: Vec::new(),
        lanes: Vec::new(),
        progress: Vec::new(),
        limitations: Vec::new(),
        report: String::new(),
        error: String::new(),
        created_at: "2026-07-11T12:00:00Z".into(),
        updated_at: "2026-07-11T12:00:00Z".into(),
        completed_at: None,
    };
    assert_eq!(
        repository
            .create_run(run.clone(), conformance_actor("research-user"))
            .expect("create run"),
        run
    );
    assert!(
        repository
            .create_run(run.clone(), conformance_actor("research-user"))
            .is_err(),
        "duplicate creation must fail"
    );
    let mut changed_provenance = run.clone();
    changed_provenance.question = "Changed".into();
    assert!(
        repository
            .update_run(changed_provenance, conformance_actor("research-user"))
            .is_err(),
        "research provenance must be immutable"
    );
    let source = ResearchSource {
        id: "source-conformance".into(),
        run_id: run.id.clone(),
        label: "R1".into(),
        kind: ResearchSourceKind::Repo,
        title: "Architecture".into(),
        uri: "docs/develop/architecture.md".into(),
        content: "The runtime is event sourced.".into(),
        query: "architecture".into(),
        metadata: BTreeMap::new(),
        created_at: "2026-07-11T12:01:00Z".into(),
    };
    let mut skipped_label = source.clone();
    skipped_label.label = "R2".into();
    assert!(
        repository
            .add_source(skipped_label, conformance_actor("research-user"))
            .is_err(),
        "source labels must be sequential"
    );
    repository
        .add_source(source.clone(), conformance_actor("research-user"))
        .expect("add source");
    assert!(
        repository
            .add_source(source, conformance_actor("research-user"))
            .is_err(),
        "source identity and URI must be unique"
    );
    let claim = ResearchClaim {
        id: "claim-conformance".into(),
        run_id: run.id.clone(),
        text: "The runtime is event sourced.".into(),
        source_labels: vec!["R1".into()],
        created_at: "2026-07-11T12:02:00Z".into(),
    };
    let mut dangling = claim.clone();
    dangling.source_labels = vec!["R2".into()];
    assert!(
        repository
            .add_claim(dangling, conformance_actor("research-user"))
            .is_err(),
        "claim labels must resolve"
    );
    repository
        .add_claim(claim.clone(), conformance_actor("research-user"))
        .expect("add claim");
    assert!(
        repository
            .add_claim(claim, conformance_actor("research-user"))
            .is_err(),
        "claim identity must be unique"
    );
    run.status = ResearchStatus::Completed;
    run.report = "The runtime is event sourced [R1].".into();
    run.updated_at = "2026-07-11T12:03:00Z".into();
    run.completed_at = Some(run.updated_at.clone());
    repository
        .update_run(run.clone(), conformance_actor("research-user"))
        .expect("complete run");
    assert!(
        repository
            .update_run(run.clone(), conformance_actor("research-user"))
            .is_err(),
        "terminal runs must be immutable"
    );
    drop(repository);

    let reopened = factory();
    assert_eq!(reopened.get_run(&run.id).expect("reopened run"), Some(run));
    assert_eq!(
        reopened
            .list_sources("research-conformance")
            .expect("sources")
            .len(),
        1
    );
    assert_eq!(
        reopened
            .list_claims("research-conformance")
            .expect("claims")
            .len(),
        1
    );
    assert_eq!(
        reopened
            .list_runs(Some("session-conformance"), 10)
            .expect("session runs")
            .len(),
        1
    );
    assert!(
        reopened
            .list_runs(Some("another-session"), 10)
            .expect("filtered runs")
            .is_empty()
    );
}
