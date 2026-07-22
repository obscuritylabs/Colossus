//! Repository contracts for cost-bounded PR and pre-merge validation.

mod support;

use std::{collections::BTreeSet, fs, process::Command};
use support::{field, job, jobs, mapping, named_step, repository_root, strings, workflow};

#[test]
fn workflows_are_split_and_the_catch_all_is_removed() {
    let root = repository_root().join(".github/workflows");
    assert!(!root.join("ci.yml").exists());
    for name in [
        "pr.yml",
        "premerge.yml",
        "release.yml",
        "docs.yml",
        "docs-external-links.yml",
    ] {
        workflow(name);
    }
}

#[test]
fn pr_workflow_selects_only_the_required_validation_tier() {
    let workflow = workflow("pr.yml");
    let root = mapping(&workflow, "PR workflow");
    let triggers = mapping(field(root, "on"), "PR triggers");
    assert_eq!(triggers.keys().collect::<Vec<_>>(), [&"pull_request"]);
    let pull_request = mapping(field(triggers, "pull_request"), "pull request trigger");
    assert_eq!(
        strings(field(pull_request, "types"), "PR event types"),
        [
            "edited",
            "opened",
            "ready_for_review",
            "reopened",
            "synchronize",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    );

    let jobs = jobs(&workflow);
    assert_eq!(
        jobs.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        [
            "classify",
            "dependency-policy",
            "documentation",
            "gate",
            "rust"
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(
        field(job(jobs, "rust"), "runs-on").as_str(),
        Some("ubuntu-latest")
    );
    assert_eq!(
        field(job(jobs, "documentation"), "if").as_str(),
        Some("needs.classify.outputs.docs_required == 'true'")
    );
    assert_eq!(
        field(job(jobs, "gate"), "name").as_str(),
        Some("Colossus PR gate")
    );
    assert_eq!(field(job(jobs, "gate"), "if").as_str(), Some("always()"));

    let source = fs::read_to_string(repository_root().join(".github/workflows/pr.yml"))
        .expect("read PR workflow");
    for forbidden in ["macos-", "windows-", "ubuntu-24.04-arm", "merge_group"] {
        assert!(!source.contains(forbidden), "PR tier contains {forbidden}");
    }
    for required in [
        "./scripts/check_crate_roots.sh",
        "cargo clippy --locked --workspace --all-targets -- -D warnings",
        "cargo test --locked --workspace",
        "release/install-apparmor.sh",
        "ACTIONLINT_VERSION: 1.7.12",
        "ACTIONLINT_SHA256: 8aca8db96f1b94770f1b0d72b6dddcb1ebb8123cb3712530b08cc387b349a3d8",
        "sha256sum --check --strict",
        "--diff-filter=ACDMRTUXB",
        "ref: ${{ github.event.pull_request.base.sha }}",
        "path: .ci-trusted",
        ".ci-trusted/scripts/ci/classify-changes.sh",
        ".ci-trusted/scripts/ci/require-pr-results.sh",
        "rust_required=true",
        "docs_required=true",
        "dependency_required=true",
        "sdk_required: ${{ steps.changes.outputs.sdk_required }}",
        "desktop_required: ${{ steps.changes.outputs.desktop_required }}",
        "! grep -q '^sdk_required='",
        "! grep -q '^desktop_required='",
        "sdk_required=true",
        "desktop_required=true",
        "actions/setup-node@49933ea5288caeca8642d1e84afbd3f7d6820020 # v4.4.0",
        "actions/setup-python@a26af69be951a213d495a4c3e4e4022e16d87065 # v5.6.0",
        "actions/setup-go@40f1582b2485089dde7abd97c1529aa768e1baff # v5.6.0",
        "./sdk/scripts/check-breaking \"$EVENT_BASE_SHA\"",
        "./sdk/scripts/generate",
        "npm pack --dry-run --json --silent",
        "go test -mod=readonly ./...",
        "working-directory: apps/desktop",
        "npm audit --audit-level=high",
        "--package colossus-sidecar-protocol",
        "--package colossus-sidecar",
        "cargo deny --manifest-path apps/desktop/src-tauri/Cargo.toml",
        "cargo audit --no-fetch --file apps/desktop/src-tauri/Cargo.lock",
        "true:success",
    ] {
        assert!(source.contains(required), "PR tier is missing {required}");
    }
}

#[test]
fn premerge_requires_an_authorized_label_and_representative_platforms() {
    let workflow = workflow("premerge.yml");
    let root = mapping(&workflow, "pre-merge workflow");
    let permissions = mapping(field(root, "permissions"), "pre-merge permissions");
    assert_eq!(
        field(permissions, "pull-requests").as_str(),
        Some("read"),
        "eligibility must be able to read the pull request without a broad write grant"
    );
    let pull_request = mapping(
        field(
            mapping(field(root, "on"), "pre-merge triggers"),
            "pull_request",
        ),
        "pre-merge pull request trigger",
    );
    assert_eq!(
        strings(field(pull_request, "types"), "pre-merge event types"),
        ["labeled", "synchronize"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );

    let jobs = jobs(&workflow);
    assert_eq!(
        field(job(jobs, "macos-native"), "runs-on").as_str(),
        Some("macos-14")
    );
    assert_eq!(
        field(job(jobs, "macos-desktop"), "runs-on").as_str(),
        Some("macos-14")
    );
    assert_eq!(
        field(job(jobs, "windows-runtime"), "runs-on").as_str(),
        Some("windows-2025")
    );
    assert_eq!(
        field(job(jobs, "gate"), "name").as_str(),
        Some("Colossus pre-merge gate")
    );
    assert_eq!(
        field(job(jobs, "gate"), "if").as_str(),
        Some("always()"),
        "the required gate must fail closed on synchronize and non-ci:full label events"
    );
    for name in [
        "macos-native",
        "macos-desktop",
        "windows-runtime",
        "fuzz",
        "supply-chain",
        "chroma",
        "storage",
        "live-security",
    ] {
        assert_eq!(
            field(job(jobs, name), "needs").as_str(),
            Some("eligibility"),
            "{name} must not allocate a runner before eligibility"
        );
    }

    let source = fs::read_to_string(repository_root().join(".github/workflows/premerge.yml"))
        .expect("read pre-merge workflow");
    for required in [
        "github.event.label.name == 'ci:full'",
        "repos/$GITHUB_REPOSITORY/pulls/$PR_NUMBER",
        "jq -r .draft",
        "EXPECTED_HEAD_SHA: ${{ github.event.pull_request.head.sha }}",
        "head_sha=$(jq",
        "test \"$head_sha\" = \"$EXPECTED_HEAD_SHA\"",
        "commits/$head_sha/check-runs?per_page=100",
        "collaborators/$LABEL_ACTOR/permission",
        "Colossus PR gate",
        "pull-requests: write",
        "github.event.action == 'synchronize'",
        "--method DELETE",
        ".ci-trusted/scripts/ci/require-success.sh",
        "actions/setup-node@49933ea5288caeca8642d1e84afbd3f7d6820020 # v4.4.0",
        "components: clippy,rustfmt",
        "CARGO_INCREMENTAL: \"0\"",
        "CARGO_TARGET_DIR: ${{ github.workspace }}/apps/desktop/src-tauri/target",
        "./scripts/prepare-desktop-binaries debug",
        "npm run tauri:build",
        "npm run tauri:bundle:macos",
        "COLOSSUS_DESKTOP_SIGNING_IDENTITY: \"-\"",
        "COLOSSUS_DESKTOP_TEAM_ID: \"ADHOC\"",
        "--package colossus-sidecar-protocol",
        "--package colossus-sidecar",
        "--test native_lifecycle -- --ignored --nocapture",
        "cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml -- --check",
        "cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings",
        "cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --lib",
        "test \"$CARGO_TARGET_DIR\" = \"$expected\"",
        "rm -rf \"$expected/debug\"",
        "cargo deny --manifest-path apps/desktop/src-tauri/Cargo.toml",
        "cargo audit --no-fetch --file apps/desktop/src-tauri/Cargo.lock",
    ] {
        assert!(
            source.contains(required),
            "pre-merge tier is missing {required}"
        );
    }
    for forbidden in ["macos-15-intel", "windows-11-arm", "ubuntu-24.04-arm"] {
        assert!(
            !source.contains(forbidden),
            "pre-merge tier contains {forbidden}"
        );
    }
    assert!(!source.contains(">/dev/null 2>&1 || true"));

    let native_source =
        serde_json::to_string(job(jobs, "macos-native")).expect("serialize macOS native job");
    assert!(!native_source.contains("npm run tauri:"));
    assert!(!native_source.contains("apps/desktop/src-tauri/target"));

    let desktop_source =
        serde_json::to_string(job(jobs, "macos-desktop")).expect("serialize macOS desktop job");
    assert!(desktop_source.contains("npm run tauri:build"));
    assert!(desktop_source.contains("npm run tauri:bundle:macos"));
    assert!(desktop_source.contains("CARGO_TARGET_DIR"));

    let concurrency = mapping(field(root, "concurrency"), "pre-merge concurrency");
    assert_eq!(
        field(concurrency, "cancel-in-progress").as_bool(),
        Some(true)
    );
}

#[test]
fn release_includes_a_signed_notarized_apple_silicon_desktop() {
    let workflow = workflow("release.yml");
    let release_jobs = jobs(&workflow);
    let desktop_build = job(release_jobs, "desktop_macos_build");
    let desktop = job(release_jobs, "desktop_macos");
    assert_eq!(field(desktop_build, "runs-on").as_str(), Some("macos-14"));
    assert_eq!(field(desktop_build, "needs").as_str(), Some("validate"));
    assert_eq!(field(desktop, "runs-on").as_str(), Some("macos-14"));
    assert_eq!(
        strings(field(desktop, "needs"), "Desktop signing needs"),
        ["desktop_macos_build", "validate"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
    assert_eq!(
        strings(
            field(job(release_jobs, "gate"), "needs"),
            "release gate needs"
        ),
        [
            "artifacts",
            "desktop_macos",
            "desktop_macos_build",
            "validate",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    );

    let source = fs::read_to_string(repository_root().join(".github/workflows/release.yml"))
        .expect("read release workflow");
    for required in [
        "needs.validate.outputs.publish_draft != 'true'",
        "COLOSSUS_DESKTOP_SIGNING_IDENTITY=-",
        "COLOSSUS_DESKTOP_TEAM_ID=ADHOC",
        "needs.validate.outputs.publish_draft == 'true'",
        "secrets.MACOS_DEVELOPER_ID_P12_BASE64",
        "secrets.MACOS_DEVELOPER_ID_P12_PASSWORD",
        "secrets.MACOS_NOTARY_API_KEY_BASE64",
        "secrets.MACOS_NOTARY_KEY_ID",
        "secrets.MACOS_NOTARY_ISSUER_ID",
        "secrets.MACOS_TEAM_ID",
        "vars.MACOS_TEAM_ID",
        "[[ \"$MACOS_TEAM_ID\" =~ ^[A-Z0-9]{10}$ ]]",
        "security create-keychain",
        "security import",
        "security set-key-partition-list",
        "xcrun notarytool store-credentials",
        "COLOSSUS_DESKTOP_NOTARY_KEYCHAIN",
        "COLOSSUS_DESKTOP_TEAM_ID=%s",
        "COLOSSUS_DESKTOP_RELEASE_VERSION: ${{ needs.validate.outputs.version }}",
        "./scripts/package-desktop-macos build",
        "./scripts/package-desktop-macos sign \"$COLOSSUS_DESKTOP_UNSIGNED_APP\"",
        "Colossus Desktop.unsigned.zip",
        "desktop-macos-unsigned-aarch64-apple-darwin",
        "/usr/bin/ditto -x -k",
        "verify-desktop-unsigned-archive.mjs",
        "--extracted-root \"$destination\"",
        "protected_hashes=()",
        "realpathSync(process.execPath)",
        "Colossus-Desktop-${RELEASE_TAG}-aarch64-apple-darwin.zip",
        "Colossus-Desktop-VALIDATION-ONLY-ADHOC-${RELEASE_TAG}-aarch64-apple-darwin.zip",
        "colossus-desktop-validation-only-adhoc-aarch64-apple-darwin",
        "Upload non-runnable ADHOC validation archive and checksum",
        "shasum -a 256",
        "desktop_macos_build=${{ needs.desktop_macos_build.result }}",
        "desktop_macos=${{ needs.desktop_macos.result }}",
        "-eq 14",
    ] {
        assert!(
            source.contains(required),
            "Desktop release is missing {required}"
        );
    }

    let build_start = source.find("  desktop_macos_build:").expect("build job");
    let sign_start = source[build_start..]
        .find("  desktop_macos:")
        .map(|offset| build_start + offset)
        .expect("sign job");
    let gate_start = source[sign_start..]
        .find("  gate:")
        .map(|offset| sign_start + offset)
        .expect("gate job");
    let build_job = &source[build_start..sign_start];
    let sign_job = &source[sign_start..gate_start];
    for forbidden in [
        "${{ secrets.",
        "MACOS_DEVELOPER_ID_P12",
        "MACOS_NOTARY_API_KEY",
        "security import",
    ] {
        assert!(
            !build_job.contains(forbidden),
            "credential-free Desktop build contains {forbidden}"
        );
    }
    for forbidden in [
        "actions/setup-node@",
        "rust-toolchain@",
        "npm ci",
        "cargo build",
        "tauri build",
    ] {
        assert!(
            !sign_job.contains(forbidden),
            "Desktop signing job contains build authority {forbidden}"
        );
    }
}

#[test]
fn change_classifier_and_gates_fail_closed() {
    let status = Command::new(repository_root().join("scripts/ci/test-contracts.sh"))
        .status()
        .expect("run CI shell contract tests");
    assert!(status.success());
}

#[test]
fn every_workflow_action_is_immutably_pinned() {
    let workflows = repository_root().join(".github/workflows");
    for entry in fs::read_dir(workflows).expect("read workflows") {
        let path = entry.expect("workflow entry").path();
        if path.extension().and_then(|value| value.to_str()) != Some("yml") {
            continue;
        }
        let source = fs::read_to_string(&path).expect("read workflow");
        for line in source.lines() {
            let Some(action) = line.trim().strip_prefix("uses: ") else {
                continue;
            };
            let reference = action
                .split_once('@')
                .unwrap_or_else(|| panic!("action is missing a reference in {path:?}: {action}"))
                .1
                .split_whitespace()
                .next()
                .expect("action reference");
            assert_eq!(reference.len(), 40, "action is not SHA-pinned: {action}");
            assert!(
                reference.bytes().all(|byte| byte.is_ascii_hexdigit()),
                "action is not SHA-pinned: {action}"
            );
            assert!(
                action.contains(" # "),
                "action pin is missing its audited release comment: {action}"
            );
        }
    }
}

#[test]
fn workflows_have_bounded_jobs_and_deterministic_concurrency() {
    for name in [
        "docs-external-links.yml",
        "docs.yml",
        "pr.yml",
        "premerge.yml",
        "release.yml",
    ] {
        let workflow = workflow(name);
        let root = mapping(&workflow, name);
        let concurrency = mapping(field(root, "concurrency"), "workflow concurrency");
        assert!(field(concurrency, "group").as_str().is_some());
        assert!(field(concurrency, "cancel-in-progress").is_boolean());
        for (job_name, value) in jobs(&workflow) {
            let job = mapping(value, job_name);
            assert!(
                field(job, "timeout-minutes").as_u64().is_some(),
                "{name}:{job_name} is missing a timeout"
            );
        }
    }
}

#[test]
fn tracked_ruleset_starts_in_evaluation_and_has_no_bypass() {
    let path = repository_root().join(".github/rulesets/main.json");
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).expect("read ruleset"))
            .expect("parse ruleset");
    let ruleset = mapping(&value, "main ruleset");
    assert_eq!(field(ruleset, "enforcement").as_str(), Some("evaluate"));
    assert_eq!(
        field(ruleset, "bypass_actors").as_array().map(Vec::len),
        Some(0)
    );
    let source = value.to_string();
    for required in [
        "refs/heads/main",
        "required_review_thread_resolution",
        "strict_required_status_checks_policy",
        "Colossus PR gate",
        "Colossus pre-merge gate",
        "non_fast_forward",
        "deletion",
    ] {
        assert!(source.contains(required), "ruleset is missing {required}");
    }

    let bootstrap =
        fs::read_to_string(repository_root().join("scripts/ci/configure-repository.sh"))
            .expect("read ruleset bootstrap");
    for required in [
        "ci:full",
        "evaluate",
        "enforcement=active",
        "gh label create",
    ] {
        assert!(
            bootstrap.contains(required),
            "bootstrap is missing {required}"
        );
    }
}

#[test]
fn documentation_deployment_no_longer_duplicates_pr_validation() {
    let workflow = workflow("docs.yml");
    let triggers = mapping(
        field(mapping(&workflow, "docs workflow"), "on"),
        "docs triggers",
    );
    assert!(!triggers.contains_key("pull_request"));
    assert!(triggers.contains_key("push"));
    assert_eq!(triggers.len(), 1, "documentation deploys only from main");
}

#[test]
fn local_test_tiers_and_sccache_wrapper_remain_optional() {
    let cargo_config = fs::read_to_string(repository_root().join(".cargo/config.toml"))
        .expect("read Cargo configuration");
    assert!(cargo_config.contains("test-fast = \"test --workspace --lib\""));
    assert!(cargo_config.contains("test-full = \"test --workspace\""));
    assert!(!cargo_config.contains("rustc-wrapper"));

    let wrapper = fs::read_to_string(repository_root().join("scripts/cargo-sccache"))
        .expect("read local sccache wrapper");
    for required in [
        "RUSTC_WRAPPER",
        "SCCACHE_BASEDIRS",
        "exec \"$cargo_bin\" \"$@\"",
    ] {
        assert!(wrapper.contains(required));
    }
}

#[test]
fn conventional_commit_checker_remains_python_free() {
    let checker = repository_root().join("scripts/check_conventional_commit.sh");
    for valid in [
        "ci: split validation tiers",
        "fix(ci): fail closed on stale labels",
        "security!: tighten release policy",
        "Merge branch 'main' into feature",
    ] {
        let mut child = Command::new(&checker)
            .arg("--stdin")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("start commit checker");
        use std::io::Write as _;
        child
            .stdin
            .as_mut()
            .expect("checker stdin")
            .write_all(valid.as_bytes())
            .expect("write valid title");
        assert!(child.wait().expect("wait for checker").success(), "{valid}");
    }

    let mut invalid = Command::new(&checker)
        .arg("--stdin")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("start commit checker");
    use std::io::Write as _;
    invalid
        .stdin
        .as_mut()
        .expect("checker stdin")
        .write_all(b"Update CI")
        .expect("write invalid title");
    assert!(!invalid.wait().expect("wait for checker").success());
}

#[test]
fn bounded_fuzzing_uses_the_pinned_nightly_and_limits() {
    let workflow = workflow("premerge.yml");
    let fuzz = job(jobs(&workflow), "fuzz");
    let install = named_step(fuzz, "Install pinned nightly Rust");
    assert_eq!(
        field(
            mapping(field(install, "with"), "nightly inputs"),
            "toolchain"
        )
        .as_str(),
        Some("nightly-2026-07-10")
    );
    let run = field(
        named_step(fuzz, "Run bounded security parser fuzzing"),
        "run",
    )
    .as_str()
    .expect("fuzz command");
    for required in [
        "cargo +nightly-2026-07-10 fuzz run",
        "-runs=5000",
        "-max_len=65536",
        "-timeout=10",
        "-rss_limit_mb=2048",
    ] {
        assert!(run.contains(required), "fuzz command is missing {required}");
    }
}

#[test]
fn pull_request_decisions_use_base_revision_contracts() {
    let pr = workflow("pr.yml");
    let premerge = workflow("premerge.yml");

    for (workflow, job_name, step_name) in [
        (&pr, "classify", "Check out trusted CI contracts"),
        (&pr, "gate", "Check out gate contract"),
        (&premerge, "gate", "Check out gate contract"),
    ] {
        let checkout = named_step(job(jobs(workflow), job_name), step_name);
        let inputs = mapping(field(checkout, "with"), "trusted checkout inputs");
        assert_eq!(
            field(inputs, "ref").as_str(),
            Some("${{ github.event.pull_request.base.sha }}")
        );
        assert_eq!(field(inputs, "path").as_str(), Some(".ci-trusted"));
    }

    assert!(
        field(
            named_step(
                job(jobs(&pr), "gate"),
                "Require every selected PR validation"
            ),
            "run"
        )
        .as_str()
        .is_some_and(|run| run.contains(".ci-trusted/scripts/ci/require-pr-results.sh"))
    );
    assert!(
        field(
            named_step(
                job(jobs(&premerge), "gate"),
                "Require every pre-merge acceptance job"
            ),
            "run"
        )
        .as_str()
        .is_some_and(|run| run.contains(".ci-trusted/scripts/ci/require-success.sh"))
    );

    let classify = field(
        named_step(job(jobs(&pr), "classify"), "Classify changed paths"),
        "run",
    )
    .as_str()
    .expect("classification command");
    assert!(classify.contains(".ci-trusted/scripts/ci/classify-changes.sh"));
    assert!(!classify.contains("./scripts/ci/classify-changes.sh"));
}
