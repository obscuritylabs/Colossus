use super::*;

/// Shared canonical lifecycle, atomic supersession, filtering, and reconstruction checks
/// for every memory repository adapter.
pub fn assert_memory_repository_conformance<F>(factory: F)
where
    F: Fn() -> Box<dyn MemoryRepository>,
{
    const AT: &str = "2026-07-12T00:00:00Z";
    const LATER: &str = "2026-07-12T00:01:00Z";
    let actor = conformance_actor("memory-user");
    let repository = factory();
    let record = MemoryRecord {
        id: "memory-conformance".into(),
        scope: MemoryScope::Global,
        kind: "fact".into(),
        confidence: 0.9,
        source: "user".into(),
        status: MemoryStatus::Active,
        text: "Rust owns canonical state.".into(),
        rationale: "Shared repository contract.".into(),
        created_at: AT.into(),
        updated_at: AT.into(),
        expires_at: None,
        superseded_by: None,
    };
    repository
        .create(record.clone(), actor.clone())
        .expect("create memory");
    assert!(repository.create(record.clone(), actor.clone()).is_err());
    let mut updated = record.clone();
    updated.text = "Rust owns canonical auditable state.".into();
    updated.updated_at = LATER.into();
    repository
        .update(updated.clone(), actor.clone())
        .expect("update memory");
    let replacement = MemoryRecord {
        id: "memory-replacement".into(),
        text: "Rust owns canonical event-sourced state.".into(),
        superseded_by: None,
        ..updated.clone()
    };
    let (old, replacement) = repository
        .supersede(&record.id, replacement, actor.clone())
        .expect("supersede memory");
    assert_eq!(old.status, MemoryStatus::Superseded);
    assert_eq!(old.superseded_by.as_deref(), Some(replacement.id.as_str()));
    assert_eq!(replacement.status, MemoryStatus::Active);

    let archived_seed = MemoryRecord {
        id: "memory-archive".into(),
        text: "Archive this canonical record.".into(),
        ..record
    };
    repository
        .create(archived_seed.clone(), actor.clone())
        .expect("create archived seed");
    let archived = repository
        .archive(&archived_seed.id, actor)
        .expect("archive memory");
    assert_eq!(archived.status, MemoryStatus::Archived);

    let reopened = factory();
    assert_eq!(reopened.get_memory(&old.id).expect("old memory"), Some(old));
    assert_eq!(
        reopened
            .get_memory(&replacement.id)
            .expect("replacement memory"),
        Some(replacement.clone())
    );
    assert_eq!(
        reopened.get_memory(&archived.id).expect("archived memory"),
        Some(archived)
    );
    assert_eq!(
        reopened.list_active(10).expect("active memories"),
        vec![replacement]
    );
    assert_eq!(
        reopened
            .list_memories(Some(MemoryStatus::Superseded), 10)
            .expect("superseded memories")
            .len(),
        1
    );
}
