//! Repository-level contract for the mandatory Rust cutover matrices.

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::Path};

const REQUIRED_CUTOVER_JOBS: &[&str] = &[
    "rust-preflight",
    "rust",
    "rust-native-sandbox",
    "rust-windows-runtime",
    "rust-fuzz",
    "rust-supply-chain",
    "rust-release-smoke",
    "rust-live-chroma",
    "rust-live-storage",
    "rust-live-security",
];

const REQUIRED_PULL_REQUEST_JOBS: &[&str] = &[
    "rust-preflight",
    "rust",
    "rust-native-sandbox",
    "rust-windows-runtime",
    "rust-fuzz",
    "rust-supply-chain",
    "rust-live-chroma",
    "rust-live-storage",
    "rust-live-security",
];

const FULL_VALIDATION_CONDITION: &str = "${{ github.event_name == 'pull_request' || github.event_name == 'merge_group' || github.event_name == 'workflow_dispatch' }}";

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repository root")
}

fn workflow() -> Value {
    let path = repository_root().join(".github/workflows/ci.yml");
    let source = fs::read_to_string(&path).expect("read CI workflow");
    serde_saphyr::from_str(&source).expect("CI workflow must be valid YAML")
}

#[test]
fn canonical_source_and_release_binary_is_colossus() {
    let manifest = fs::read_to_string(repository_root().join("crates/colossus-cli/Cargo.toml"))
        .expect("read CLI manifest");
    assert!(manifest.contains("name = \"colossus\""));
    assert!(!manifest.contains("colossus-rs"));

    let workflow = fs::read_to_string(repository_root().join(".github/workflows/ci.yml"))
        .expect("read CI workflow");
    assert!(workflow.contains("--package colossus-cli --bin colossus"));
    assert!(!workflow.contains("colossus-rs"));
}

#[test]
fn release_bundle_publisher_identity_is_self_consistent() {
    let source = fs::read_to_string(repository_root().join("release/bundle-publisher.json"))
        .expect("read release bundle publisher identity");
    let identity: Value = serde_json::from_str(&source).expect("publisher identity JSON");
    let identity = mapping(&identity, "publisher identity");
    assert_eq!(field(identity, "publisher").as_str(), Some("colossus"));
    assert_eq!(field(identity, "algorithm").as_str(), Some("ed25519"));
    assert_eq!(
        field(identity, "purpose").as_str(),
        Some("offline-bundle-manifest-signing")
    );
    let public_key = BASE64
        .decode(
            field(identity, "public_key")
                .as_str()
                .expect("publisher public_key"),
        )
        .expect("publisher public_key must be base64");
    assert_eq!(public_key.len(), 32, "Ed25519 public keys contain 32 bytes");
    let key_id = hex::encode(Sha256::digest(public_key));
    assert_eq!(
        field(identity, "key_id").as_str(),
        Some(key_id.as_str()),
        "key_id must be the SHA-256 digest of the decoded public key"
    );
}

#[test]
fn oci_proxy_build_context_contains_only_the_static_proxy_artifact() {
    let dockerignore = fs::read_to_string(repository_root().join(".dockerignore"))
        .expect("read Docker ignore rules");
    let rules = dockerignore.lines().collect::<BTreeSet<_>>();

    assert!(
        !rules.contains("target/"),
        "target/ prevents negated children from being included"
    );
    for required in [
        "target/*",
        "!target/x86_64-unknown-linux-musl/",
        "target/x86_64-unknown-linux-musl/*",
        "!target/x86_64-unknown-linux-musl/release/",
        "target/x86_64-unknown-linux-musl/release/*",
        "!target/x86_64-unknown-linux-musl/release/colossus-oci-proxy",
    ] {
        assert!(
            rules.contains(required),
            "Docker context is missing exact proxy rule {required:?}"
        );
    }
}

#[test]
fn local_cutover_verifier_is_complete_and_tool_version_pinned() {
    let script = fs::read_to_string(repository_root().join("release/verify-local-cutover.sh"))
        .expect("read local cutover verifier");
    for required in [
        "rustc 1.96.0",
        "cargo-deny 0.20.2",
        "cargo-audit 0.22.2",
        "cargo fmt --all -- --check",
        "cargo clippy --locked --workspace --all-targets -- -D warnings",
        "cargo test --locked --workspace",
        "cargo check --locked --manifest-path fuzz/Cargo.toml --all-targets",
        "cargo deny --locked check",
        "cargo audit -D warnings",
        "--file Cargo.lock",
        "--file fuzz/Cargo.lock",
        "git ls-files '*.py'",
    ] {
        assert!(
            script.contains(required),
            "local cutover verifier is missing {required:?}"
        );
    }
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

fn named_step<'a>(job: &'a Map<String, Value>, name: &str) -> &'a Map<String, Value> {
    field(job, "steps")
        .as_array()
        .expect("job steps must be a sequence")
        .iter()
        .map(|step| mapping(step, "job step"))
        .find(|step| field(step, "name").as_str() == Some(name))
        .unwrap_or_else(|| panic!("missing job step {name}"))
}

fn has_named_step(job: &Map<String, Value>, name: &str) -> bool {
    field(job, "steps")
        .as_array()
        .expect("job steps must be a sequence")
        .iter()
        .map(|step| mapping(step, "job step"))
        .any(|step| field(step, "name").as_str() == Some(name))
}

#[test]
fn actions_cost_policy_runs_full_validation_only_before_merge_or_on_manual_dispatch() {
    let workflow = workflow();
    let root = mapping(&workflow, "workflow");
    let triggers = mapping(field(root, "on"), "workflow triggers");
    assert_eq!(
        triggers.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        ["merge_group", "pull_request", "push", "workflow_dispatch"]
            .into_iter()
            .collect()
    );
    let push = mapping(field(triggers, "push"), "push trigger");
    assert_eq!(
        strings(field(push, "branches"), "push branches"),
        ["main"].into_iter().map(str::to_owned).collect()
    );

    let jobs = jobs(&workflow);
    assert_eq!(
        field(job(jobs, "rust-quick"), "if").as_str(),
        Some("${{ github.event_name == 'push' && github.ref == 'refs/heads/main' }}")
    );
    assert_eq!(
        field(job(jobs, "rust-preflight"), "if").as_str(),
        Some(FULL_VALIDATION_CONDITION)
    );
    assert_eq!(
        field(job(jobs, "rust"), "if").as_str(),
        Some(FULL_VALIDATION_CONDITION)
    );
    assert!(
        !jobs.contains_key("rust-portability"),
        "standalone macOS/Windows portability duplicates native matrix runners"
    );
    assert!(
        !has_named_step(
            job(jobs, "rust-native-sandbox"),
            "Compile every target on the native platform"
        ),
        "native acceptance must not compile the whole workspace before its focused test"
    );
    assert!(
        !has_named_step(
            job(jobs, "rust-windows-runtime"),
            "Compile every target on the native platform"
        ),
        "Windows acceptance must not compile the whole workspace before its focused tests"
    );
    assert_eq!(
        field(job(jobs, "rust-release-smoke"), "if").as_str(),
        Some("${{ github.event_name == 'workflow_dispatch' }}"),
        "six-target packaging must run only for explicit release validation"
    );
}

#[test]
fn full_matrix_fans_out_after_a_cached_fast_preflight() {
    let workflow = workflow();
    let root = mapping(&workflow, "workflow");
    let jobs = jobs(&workflow);
    let preflight = job(jobs, "rust-preflight");

    assert_eq!(field(preflight, "timeout-minutes").as_u64(), Some(5));
    for step in [
        "Check formatting",
        "Check independent fuzz harness formatting",
        "Validate locked workspace metadata",
        "Validate locked fuzz metadata",
    ] {
        assert!(
            has_named_step(preflight, step),
            "preflight is missing {step}"
        );
    }

    for heavy_job in REQUIRED_CUTOVER_JOBS
        .iter()
        .copied()
        .filter(|name| *name != "rust-preflight")
    {
        assert_eq!(
            field(job(jobs, heavy_job), "needs").as_str(),
            Some("rust-preflight"),
            "{heavy_job} must start immediately after the fast preflight"
        );
    }

    let workflow_wrapper = root
        .get("env")
        .and_then(Value::as_object)
        .and_then(|environment| environment.get("RUSTC_WRAPPER"))
        .and_then(Value::as_str);
    assert_ne!(
        workflow_wrapper,
        Some("sccache"),
        "the compiler wrapper must not leak into jobs that do not install it"
    );
    for compile_job_name in [
        "rust-quick",
        "rust",
        "rust-native-sandbox",
        "rust-windows-runtime",
        "rust-fuzz",
        "rust-release-smoke",
        "rust-live-chroma",
        "rust-live-storage",
        "rust-live-security",
    ] {
        let compile_job = job(jobs, compile_job_name);
        let environment = mapping(field(compile_job, "env"), "compile job environment");
        assert_eq!(
            field(environment, "SCCACHE_GHA_ENABLED").as_str(),
            Some("true")
        );
        assert_eq!(
            field(environment, "RUSTC_WRAPPER").as_str(),
            Some("sccache")
        );
        let cache = named_step(compile_job, "Enable shared compiler cache");
        assert_eq!(
            field(cache, "uses").as_str(),
            Some("mozilla-actions/sccache-action@v0.0.10"),
            "{compile_job_name} must use the pinned shared compiler cache action"
        );
    }
    for (job_name, job_definition) in jobs {
        let job = mapping(job_definition, job_name);
        let wrapper = job
            .get("env")
            .and_then(Value::as_object)
            .and_then(|environment| environment.get("RUSTC_WRAPPER"))
            .and_then(Value::as_str);
        if wrapper == Some("sccache") {
            assert!(
                has_named_step(job, "Enable shared compiler cache"),
                "{job_name} configures sccache without installing it"
            );
        }
    }

    let install = named_step(
        job(jobs, "rust-supply-chain"),
        "Install checksum-verified pinned supply-chain tools",
    );
    assert_eq!(
        field(install, "uses").as_str(),
        Some("taiki-e/install-action@v2.83.2")
    );
    let inputs = mapping(field(install, "with"), "supply-chain installer inputs");
    assert_eq!(
        field(inputs, "tool").as_str(),
        Some("cargo-deny@0.20.2,cargo-audit@0.22.2")
    );
    assert_eq!(field(inputs, "fallback").as_str(), Some("none"));
}

#[test]
fn pull_request_gate_fails_closed_without_scheduling_release_artifacts() {
    let workflow = workflow();
    let jobs = jobs(&workflow);
    let gate = job(jobs, "rust-pr-gate");
    assert_eq!(
        field(gate, "if").as_str(),
        Some(
            "${{ always() && (github.event_name == 'pull_request' || github.event_name == 'merge_group') }}"
        )
    );
    assert_eq!(
        strings(field(gate, "needs"), "pull request needs"),
        REQUIRED_PULL_REQUEST_JOBS
            .iter()
            .map(|job| (*job).to_owned())
            .collect()
    );
    assert!(!REQUIRED_PULL_REQUEST_JOBS.contains(&"rust-release-smoke"));
    let script = field(
        mapping(
            &field(gate, "steps").as_array().expect("gate steps")[0],
            "pull request gate step",
        ),
        "run",
    )
    .as_str()
    .expect("pull request gate script");
    assert!(script.contains("details.get(\"result\") != \"success\""));
    assert!(script.contains("raise SystemExit(bool(failed))"));
}

#[test]
fn cutover_gate_fails_closed_over_every_required_rust_job() {
    let workflow = workflow();
    let jobs = jobs(&workflow);
    let gate = job(jobs, "rust-cutover-gate");
    assert_eq!(
        field(gate, "if").as_str(),
        Some("${{ always() && github.event_name == 'workflow_dispatch' }}"),
        "the release gate must run after every dependency on explicit validation"
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

#[test]
fn bounded_fuzzing_executes_with_the_pinned_nightly_toolchain() {
    let workflow = workflow();
    let jobs = jobs(&workflow);
    let fuzz = job(jobs, "rust-fuzz");
    let install = named_step(fuzz, "Install pinned nightly Rust");
    let toolchain = mapping(field(install, "with"), "nightly install inputs");
    assert_eq!(
        field(toolchain, "toolchain").as_str(),
        Some("nightly-2026-07-10")
    );

    let run = field(
        named_step(fuzz, "Run bounded security parser fuzzing"),
        "run",
    )
    .as_str()
    .expect("fuzz run script");
    assert!(run.contains("cargo +nightly-2026-07-10 fuzz run"));
    for bound in [
        "-runs=5000",
        "-max_len=65536",
        "-timeout=10",
        "-rss_limit_mb=2048",
    ] {
        assert!(run.contains(bound), "fuzz run must retain {bound}");
    }
}

#[test]
fn unix_release_install_smoke_is_compatible_with_macos_bash() {
    let workflow = workflow();
    let jobs = jobs(&workflow);
    let release = job(jobs, "rust-release-smoke");
    let run = field(
        named_step(release, "Install packaged Unix artifact offline"),
        "run",
    )
    .as_str()
    .expect("Unix install smoke script");
    assert!(run.contains("packages=(\"$extract\"/colossus-*)"));
    assert!(run.contains("test -d \"$package\""));
    assert!(
        !run.contains("mapfile"),
        "macOS ships Bash 3.2 without mapfile"
    );
}

#[test]
fn conventional_commit_checker_is_python_free_and_preserves_the_contract() {
    let checker = repository_root().join("scripts/check_conventional_commit.sh");
    for valid in [
        "feat: add tui themes",
        "fix(tui): clear approved prompt",
        "security!: tighten approval policy",
        "Merge branch 'main' into feature",
    ] {
        let mut child = std::process::Command::new(&checker)
            .arg("--stdin")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("start Conventional Commit checker");
        use std::io::Write as _;
        child
            .stdin
            .as_mut()
            .expect("checker stdin")
            .write_all(valid.as_bytes())
            .expect("write valid subject");
        assert!(child.wait().expect("wait for checker").success(), "{valid}");
    }

    let mut child = std::process::Command::new(&checker)
        .arg("--stdin")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("start Conventional Commit checker");
    use std::io::Write as _;
    child
        .stdin
        .as_mut()
        .expect("checker stdin")
        .write_all(b"Update docs")
        .expect("write invalid subject");
    assert!(!child.wait().expect("wait for checker").success());
}
