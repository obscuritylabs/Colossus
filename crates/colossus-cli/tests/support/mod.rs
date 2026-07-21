use serde_json::{Map, Value};
use std::{collections::BTreeSet, fs, path::Path};

pub fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repository root")
}

pub fn workflow(name: &str) -> Value {
    let path = repository_root().join(".github/workflows").join(name);
    let source = fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {path:?}: {error}"));
    serde_saphyr::from_str(&source).unwrap_or_else(|error| panic!("parse {path:?}: {error}"))
}

pub fn mapping<'a>(value: &'a Value, context: &str) -> &'a Map<String, Value> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("{context} must be a mapping"))
}

pub fn field<'a>(value: &'a Map<String, Value>, name: &str) -> &'a Value {
    value
        .get(name)
        .unwrap_or_else(|| panic!("missing field {name}"))
}

#[allow(dead_code)]
pub fn strings(value: &Value, context: &str) -> BTreeSet<String> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("{context} must be a sequence"))
        .iter()
        .map(|item| {
            item.as_str()
                .unwrap_or_else(|| panic!("{context} entries must be strings"))
                .to_owned()
        })
        .collect()
}

pub fn jobs(workflow: &Value) -> &Map<String, Value> {
    mapping(field(mapping(workflow, "workflow"), "jobs"), "jobs")
}

pub fn job<'a>(jobs: &'a Map<String, Value>, name: &str) -> &'a Map<String, Value> {
    mapping(field(jobs, name), name)
}

pub fn named_step<'a>(job: &'a Map<String, Value>, name: &str) -> &'a Map<String, Value> {
    field(job, "steps")
        .as_array()
        .expect("job steps must be a sequence")
        .iter()
        .map(|step| mapping(step, "job step"))
        .find(|step| field(step, "name").as_str() == Some(name))
        .unwrap_or_else(|| panic!("missing job step {name}"))
}
