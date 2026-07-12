//! Repository-level contract for the mandatory Rust cutover matrices.

use serde_json::{Map, Value};
use std::{collections::BTreeSet, fs, path::Path};

const REQUIRED_CUTOVER_JOBS: &[&str] = &[
    "rust",
    "rust-portability",
    "rust-native-sandbox",
    "rust-windows-runtime",
    "rust-fuzz",
    "rust-supply-chain",
    "rust-release-smoke",
    "rust-live-chroma",
    "rust-live-security",
];

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("repository root")
}

fn workflow() -> Value {
    let path = repository_root().join(".github/workflows/ci.yml");
    let source = fs::read_to_string(&path).expect("read CI workflow");
    serde_saphyr::from_str(&source).expect("CI workflow must be valid YAML")
}

fn mapping<'a>(value: &'a Value, context: &str) -> &'a Map<String, Value> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("{context} must be a mapping"))
}

fn field<'a>(mapping: &'a Map<String, Value>, name: &str) -> &'a Value {
    mapping
        .get(name)
        .unwrap_or_else(|| panic!("missing field {name}"))
}

fn strings(value: &Value, context: &str) -> BTreeSet<String> {
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

fn jobs(workflow: &Value) -> &Map<String, Value> {
    mapping(field(mapping(workflow, "workflow"), "jobs"), "jobs")
}

fn job<'a>(jobs: &'a Map<String, Value>, name: &str) -> &'a Map<String, Value> {
    mapping(field(jobs, name), name)
}

fn matrix_includes<'a>(jobs: &'a Map<String, Value>, name: &str) -> &'a Vec<Value> {
    let strategy = mapping(field(job(jobs, name), "strategy"), "strategy");
    let matrix = mapping(field(strategy, "matrix"), "matrix");
    field(matrix, "include")
        .as_array()
        .expect("matrix include must be a sequence")
}

#[test]
fn cutover_gate_fails_closed_over_every_required_rust_job() {
    let workflow = workflow();
    let jobs = jobs(&workflow);
    let gate = job(jobs, "rust-cutover-gate");
    assert_eq!(
        field(gate, "if").as_str(),
        Some("${{ always() }}"),
        "the gate must run even when a dependency fails or is skipped"
    );
    assert_eq!(
        strings(field(gate, "needs"), "cutover needs"),
        REQUIRED_CUTOVER_JOBS
            .iter()
            .map(|job| (*job).to_owned())
            .collect(),
        "the cutover gate must aggregate exactly the mandatory Rust jobs"
    );

    let steps = field(gate, "steps")
        .as_array()
        .expect("cutover steps must be a sequence");
    let environment = mapping(
        field(mapping(&steps[0], "cutover step"), "env"),
        "cutover environment",
    );
    assert_eq!(
        environment.len(),
        1,
        "the complete needs object must be checked as one immutable result set"
    );
    assert_eq!(
        field(environment, "RUST_ACCEPTANCE_RESULTS").as_str(),
        Some("${{ toJSON(needs) }}"),
        "the gate must inspect every declared dependency result"
    );
    let script = field(mapping(&steps[0], "cutover step"), "run")
        .as_str()
        .expect("cutover script");
    assert!(script.contains("details.get(\"result\") != \"success\""));
    assert!(script.contains("raise SystemExit(bool(failed))"));
}

#[test]
fn hosted_platform_and_release_matrices_cover_every_supported_architecture() {
    let workflow = workflow();
    let jobs = jobs(&workflow);

    let native = job(jobs, "rust-native-sandbox");
    let native_matrix = mapping(
        field(
            mapping(field(native, "strategy"), "native strategy"),
            "matrix",
        ),
        "native matrix",
    );
    assert_eq!(
        strings(field(native_matrix, "runner"), "native runners"),
        [
            "macos-15-intel",
            "macos-14",
            "ubuntu-24.04",
            "ubuntu-24.04-arm"
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    );

    let windows = job(jobs, "rust-windows-runtime");
    let windows_matrix = mapping(
        field(
            mapping(field(windows, "strategy"), "Windows strategy"),
            "matrix",
        ),
        "Windows matrix",
    );
    assert_eq!(
        strings(field(windows_matrix, "runner"), "Windows runners"),
        ["windows-2025", "windows-11-arm"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );

    let releases: BTreeSet<(String, String, String)> = matrix_includes(jobs, "rust-release-smoke")
        .iter()
        .map(|entry| {
            let entry = mapping(entry, "release matrix entry");
            (
                field(entry, "runner")
                    .as_str()
                    .expect("release runner")
                    .into(),
                field(entry, "target")
                    .as_str()
                    .expect("release target")
                    .into(),
                field(entry, "archive")
                    .as_str()
                    .expect("release archive")
                    .into(),
            )
        })
        .collect();
    assert_eq!(
        releases,
        [
            ("macos-15-intel", "x86_64-apple-darwin", "tar.gz"),
            ("macos-14", "aarch64-apple-darwin", "tar.gz"),
            ("ubuntu-24.04", "x86_64-unknown-linux-musl", "tar.gz"),
            ("ubuntu-24.04-arm", "aarch64-unknown-linux-musl", "tar.gz"),
            ("windows-2025", "x86_64-pc-windows-msvc", "zip"),
            ("windows-11-arm", "aarch64-pc-windows-msvc", "zip"),
        ]
        .into_iter()
        .map(|(runner, target, archive)| (runner.into(), target.into(), archive.into()))
        .collect()
    );
}
