use super::{exercise_contracts_json, exercise_workflow_condition, exercise_workflow_yaml};

#[test]
fn committed_contract_corpus_never_panics() {
    for seed in [
        include_bytes!("../../../fuzz/corpus/contracts_json/event.json").as_slice(),
        include_bytes!("../../../fuzz/corpus/contracts_json/policy.json").as_slice(),
        include_bytes!("../../../fuzz/corpus/contracts_json/malformed.json").as_slice(),
    ] {
        exercise_contracts_json(seed);
    }
}

#[test]
fn committed_workflow_corpus_never_panics() {
    for seed in [
        include_bytes!("../../../fuzz/corpus/workflow_yaml/valid.yaml").as_slice(),
        include_bytes!("../../../fuzz/corpus/workflow_yaml/executable.yaml").as_slice(),
        include_bytes!("../../../fuzz/corpus/workflow_yaml/anchors.yaml").as_slice(),
    ] {
        exercise_workflow_yaml(seed);
    }
}

#[test]
fn committed_condition_corpus_never_panics() {
    for seed in [
        include_bytes!("../../../fuzz/corpus/workflow_condition/valid.txt").as_slice(),
        include_bytes!("../../../fuzz/corpus/workflow_condition/executable.txt").as_slice(),
        include_bytes!("../../../fuzz/corpus/workflow_condition/malformed.txt").as_slice(),
    ] {
        exercise_workflow_condition(seed);
    }
}
