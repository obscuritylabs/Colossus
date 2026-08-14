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
fn actionlint_recognizes_the_provisioned_larger_runner() {
    let path = repository_root().join(".github/actionlint.yaml");
    let source = fs::read_to_string(&path).expect("read actionlint configuration");
    let config: serde_json::Value =
        serde_saphyr::from_str(&source).expect("parse actionlint configuration");
    let labels = strings(
        field(
            mapping(
                field(
                    mapping(&config, "actionlint configuration"),
                    "self-hosted-runner",
                ),
                "custom runner configuration",
            ),
            "labels",
        ),
        "custom runner labels",
    );
    assert_eq!(
        labels,
        ["ubuntu-latest-m".to_owned(), "windows-latest-l".to_owned(),]
            .into_iter()
            .collect()
    );
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
        Some("ubuntu-latest-m")
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
        "cargo xtask check rust",
        "cargo xtask check sidecar",
        "cargo xtask check sdk --base \"$EVENT_BASE_SHA\"",
        "cargo xtask check desktop",
        "cargo xtask check dependencies",
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
        "true:success",
    ] {
        assert!(source.contains(required), "PR tier is missing {required}");
    }
}

#[test]
fn rust_codegen_uses_an_exact_vendored_protoc_on_every_runner() {
    let workspace =
        fs::read_to_string(repository_root().join("Cargo.toml")).expect("read workspace manifest");
    assert!(
        workspace.contains("protoc-bin-vendored = \"=3.2.0\""),
        "the build-time protoc must remain exact and cross-platform"
    );

    let manifest =
        fs::read_to_string(repository_root().join("crates/colossus-api-proto/Cargo.toml"))
            .expect("read public API proto manifest");
    assert!(manifest.contains("protoc-bin-vendored.workspace = true"));

    let build = fs::read_to_string(repository_root().join("crates/colossus-api-proto/build.rs"))
        .expect("read public API proto build script");
    for required in [
        "protoc_bin_vendored::protoc_bin_path()",
        "prost_config.protoc_executable(protoc_path)",
        "compile_with_config(prost_config",
    ] {
        assert!(
            build.contains(required),
            "public API proto build is missing {required}"
        );
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
        Some("windows-latest-l")
    );
    assert_eq!(
        field(job(jobs, "windows-runtime"), "timeout-minutes").as_u64(),
        Some(75),
        "Windows acceptance must allow the native and Desktop checks to finish"
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
        "cargo xtask check desktop",
        "./scripts/prepare-desktop-binaries debug",
        "npm run tauri:build",
        "npm run tauri:bundle:macos",
        "COLOSSUS_DESKTOP_SIGNING_IDENTITY: \"-\"",
        "COLOSSUS_DESKTOP_TEAM_ID: \"ADHOC\"",
        "COLOSSUS_DESKTOP_RELEASE_CHANNEL: \"validation_only\"",
        "--package colossus-sidecar-protocol",
        "--package colossus-sidecar",
        "--test native_lifecycle -- --ignored --nocapture",
        "cargo clippy --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings",
        "cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --lib",
        "test \"$CARGO_TARGET_DIR\" = \"$expected\"",
        "rm -rf \"$expected/debug\"",
        "for attempt in 1 2 3",
        "docker pull \"${{ matrix.image }}\"",
        "cargo xtask check dependencies",
        "timeout --foreground --kill-after=10s 300s",
        "env -i PATH=/usr/bin /usr/bin/podman run",
        "--name colossus-podman-readiness",
        "printf ready > podman-readiness.txt && cat podman-readiness.txt",
        "test \"$(cat \"$warmup/podman-readiness.txt\")\" = ready",
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
    assert_eq!(
        field(job(jobs, "live-security"), "runs-on").as_str(),
        Some("ubuntu-latest-m")
    );
    let podman_readiness = field(
        named_step(
            job(jobs, "live-security"),
            "Build immutable Podman acceptance images",
        ),
        "run",
    )
    .as_str()
    .expect("Podman image preparation must be a script");
    for required in [
        "timeout --foreground --kill-after=10s 300s",
        "env -i PATH=/usr/bin /usr/bin/podman run",
        "--name colossus-podman-readiness",
        "test \"$(cat \"$warmup/podman-readiness.txt\")\" = ready",
    ] {
        assert!(
            podman_readiness.contains(required),
            "Podman readiness is missing the sandbox launch contract {required}"
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
fn release_separates_the_stable_core_from_the_desktop_preview() {
    let workflow = workflow("release.yml");
    let release_jobs = jobs(&workflow);
    assert_eq!(
        field(job(release_jobs, "validate"), "runs-on").as_str(),
        Some("ubuntu-latest-m")
    );
    let desktop_build = job(release_jobs, "desktop_macos_build");
    let desktop = job(release_jobs, "desktop_macos");
    let sdk = job(release_jobs, "sdk_release");
    assert_eq!(field(sdk, "runs-on").as_str(), Some("ubuntu-latest-m"));
    assert_eq!(field(sdk, "needs").as_str(), Some("validate"));
    assert_eq!(
        field(sdk, "if").as_str(),
        Some("needs.validate.outputs.target_channel == 'stable'")
    );
    assert_eq!(field(desktop_build, "runs-on").as_str(), Some("macos-14"));
    assert_eq!(field(desktop_build, "needs").as_str(), Some("validate"));
    assert_eq!(
        field(desktop_build, "if").as_str(),
        Some("needs.validate.outputs.target_channel != 'stable'")
    );
    assert_eq!(field(desktop, "runs-on").as_str(), Some("macos-14"));
    assert_eq!(
        field(desktop, "if").as_str(),
        Some("needs.validate.outputs.target_channel != 'stable'")
    );
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
            "bootstrap_installers",
            "desktop_macos",
            "desktop_macos_build",
            "desktop_windows_preview",
            "sdk_release",
            "validate",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    );

    let source = fs::read_to_string(repository_root().join(".github/workflows/release.yml"))
        .expect("read release workflow");
    for required in [
        "target_channel: ${{ steps.request.outputs.target_channel }}",
        "printf 'target_channel=%s\\n' \"$tag_channel\"",
        "if: needs.validate.outputs.target_channel == 'stable'",
        "if: needs.validate.outputs.target_channel != 'stable'",
        "sdk_base_tag: ${{ steps.request.outputs.sdk_base_tag }}",
        "source_date_epoch: ${{ steps.request.outputs.source_date_epoch }}",
        "git tag --merged \"$source_commit\"",
        "cargo xtask check sdk --base \"$SDK_BASE_TAG\"",
        "npm install --global npm@11.5.1",
        "node scripts/ci/package-sdk-release.mjs",
        "SOURCE_DATE_EPOCH",
        "node scripts/ci/verify-sdk-release.mjs",
        "name: colossus-sdk-release",
        "sdk_release=\"$SDK_RELEASE_RESULT\"",
        "test \"$MACOS_DESKTOP_BUILD_RESULT\" = skipped",
        "test \"$MACOS_DESKTOP_RESULT\" = skipped",
        "test \"$WINDOWS_DESKTOP_RESULT\" = skipped",
        "test \"$SDK_RELEASE_RESULT\" = skipped",
        "obscuritylabs-colossus-sdk-${RELEASE_VERSION}.tgz",
        "obscuritylabs_colossus_sdk-${RELEASE_VERSION}-py3-none-any.whl",
        "obscuritylabs_colossus_sdk-${RELEASE_VERSION}.tar.gz",
        "colossus-sdk-${RELEASE_TAG}-manifest.json",
        "colossus-sdk-${RELEASE_TAG}-SHA256SUMS",
        "COLOSSUS_DESKTOP_SIGNING_IDENTITY=-",
        "COLOSSUS_DESKTOP_TEAM_ID=ADHOC",
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
        "Colossus-Desktop-DEVELOPER-PREVIEW-${RELEASE_TAG}-aarch64-apple-darwin.zip",
        "Colossus-Desktop-VALIDATION-ONLY-ADHOC-${RELEASE_TAG}-aarch64-apple-darwin.zip",
        "colossus-desktop-validation-only-adhoc-aarch64-apple-darwin",
        "Upload non-runnable ADHOC validation archive and checksum",
        "- runner: ubuntu-latest-m\n            target: x86_64-unknown-linux-musl",
        "shasum -a 256",
        "runs-on: windows-latest-l",
        "./scripts/package-desktop-windows.ps1",
        "codeSigning = \"unsigned_developer_preview\"",
        "smartScreenWarningExpected = $true",
        "Colossus-Desktop-UNSIGNED-$label-$env:RELEASE_TAG-x86_64-pc-windows-msvc-setup.exe",
        "-eq 21",
        "-eq 22",
    ] {
        assert!(
            source.contains(required),
            "core/preview release split is missing {required}"
        );
    }
    assert!(
        !source.contains("check sdk --base origin/main"),
        "release API compatibility must not be checked against the moving origin/main"
    );

    let build_start = source.find("  desktop_macos_build:").expect("build job");
    let sign_start = source[build_start..]
        .find("  desktop_macos:")
        .map(|offset| build_start + offset)
        .expect("sign job");
    let windows_start = source[sign_start..]
        .find("  desktop_windows_preview:")
        .map(|offset| sign_start + offset)
        .expect("Windows preview job");
    let gate_start = source[windows_start..]
        .find("  gate:")
        .map(|offset| windows_start + offset)
        .expect("gate job");
    let build_job = &source[build_start..sign_start];
    let sign_job = &source[sign_start..windows_start];
    let windows_job = &source[windows_start..gate_start];
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
    assert!(sign_job.contains("actions/setup-node@"));
    for forbidden in ["rust-toolchain@", "cargo build", "tauri build"] {
        assert!(
            !sign_job.contains(forbidden),
            "Desktop signing job contains build authority {forbidden}"
        );
    }
    assert!(windows_job.contains("if: needs.validate.outputs.target_channel != 'stable'"));
    assert!(windows_job.contains("COLOSSUS_DESKTOP_TEAM_ID: UNSIGNED"));
    assert!(!windows_job.contains("AUTHENTICODE"));
    let channel_source =
        fs::read_to_string(repository_root().join(".github/workflows/desktop-update-channels.yml"))
            .expect("read Desktop update channel workflow");
    assert!(channel_source.contains("contains(github.event.release.assets.*.name, 'stable.json')"));
}

#[test]
fn sdk_publication_is_oidc_protected_recoverable_and_byte_exact() {
    let workflow = workflow("publish-sdk.yml");
    let publication_jobs = jobs(&workflow);
    let validate = job(publication_jobs, "validate");
    let validate_permissions = mapping(
        field(validate, "permissions"),
        "SDK candidate validation permissions",
    );
    assert_eq!(
        field(validate_permissions, "actions").as_str(),
        Some("read")
    );
    assert_eq!(
        field(validate_permissions, "contents").as_str(),
        Some("read")
    );
    let publish = job(publication_jobs, "publish");
    assert_eq!(
        field(publish, "environment").as_str(),
        Some("sdk-production")
    );
    let permissions = mapping(field(publish, "permissions"), "SDK publish permissions");
    assert_eq!(field(permissions, "contents").as_str(), Some("write"));
    assert_eq!(field(permissions, "id-token").as_str(), Some("write"));

    let source = fs::read_to_string(repository_root().join(".github/workflows/publish-sdk.yml"))
        .expect("read SDK publication workflow");
    for required in [
        "types: [published]",
        "SDK publication requires a stable vX.Y.Z release tag",
        "git merge-base --is-ancestor \"$source_commit\" origin/main",
        "sdk_base_tag: ${{ steps.release.outputs.sdk_base_tag }}",
        "source_date_epoch: ${{ steps.release.outputs.source_date_epoch }}",
        "cargo xtask check sdk --base \"$SDK_BASE_TAG\"",
        "runs-on: ubuntu-latest-m",
        "timeout-minutes: 45",
        "npm install --global npm@11.5.1",
        "--output dist/sdk-rebuilt",
        "dist/sdk-release \"$RELEASE_VERSION\" \"$SOURCE_COMMIT\" dist/sdk-rebuilt",
        "node scripts/ci/verify-sdk-release.mjs",
        "dist/sdk-release \"$RELEASE_VERSION\" \"$SOURCE_COMMIT\" dist/sdk-trusted",
        "repos/$GH_REPO/actions/workflows/release.yml/runs?event=push&status=success&head_sha=$SOURCE_COMMIT",
        "--name colossus-sdk-release --dir dist/sdk-trusted",
        "node scripts/ci/check-sdk-registry-state.mjs npm",
        "node scripts/ci/check-sdk-registry-state.mjs pypi",
        "--access public --provenance",
        "pypa/gh-action-pypi-publish@dc37677b2e1c63e2034f94d8a5b11f265b73ba33",
        "skip-existing: true",
        "GO_TAG: sdk/go/${{ needs.validate.outputs.tag }}",
        "test \"$(git rev-list -n 1 \"$GO_TAG\")\" = \"$SOURCE_COMMIT\"",
    ] {
        assert!(
            source.contains(required),
            "SDK publication is missing {required}"
        );
    }
    let packager = fs::read_to_string(repository_root().join("scripts/ci/package-sdk-release.mjs"))
        .expect("read SDK packager");
    for required in ["SOURCE_DATE_EPOCH", "normalize_python_sdist.py"] {
        assert!(
            packager.contains(required),
            "SDK packager is missing {required}"
        );
    }
    let sdk_check = fs::read_to_string(repository_root().join("xtask/src/checks/sdk.rs"))
        .expect("read SDK check");
    assert!(sdk_check.contains("scripts/ci/sdk-release.test.mjs"));
    for forbidden in [
        "NODE_AUTH_TOKEN",
        "NPM_TOKEN",
        "NPM_CONFIG_PROVENANCE: \"false\"",
        "--provenance=false",
        "PYPI_API_TOKEN",
        "password:",
    ] {
        assert!(
            !source.contains(forbidden),
            "SDK publication must not use long-lived registry secret {forbidden}"
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
        "publish-sdk.yml",
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
    assert!(cargo_config.contains("xtask = \"run --package xtask --\""));
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
fn devcontainer_pins_the_supported_cross_language_toolchains() {
    let root = repository_root();
    let config: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join(".devcontainer/devcontainer.json"))
            .expect("read dev container configuration"),
    )
    .expect("parse dev container configuration");
    let config = mapping(&config, "dev container configuration");
    assert_eq!(
        field(
            mapping(field(config, "build"), "dev container build"),
            "dockerfile"
        )
        .as_str(),
        Some("Dockerfile")
    );

    let features = mapping(field(config, "features"), "dev container features");
    let rust = mapping(
        field(features, "ghcr.io/devcontainers/features/rust:1"),
        "Rust dev container feature",
    );
    assert_eq!(field(rust, "version").as_str(), Some("1.96.0"));
    assert!(features.contains_key("ghcr.io/devcontainers/features/docker-in-docker:4"));
    assert_eq!(
        features.len(),
        2,
        "language runtimes must not pull unpinned global tool suites through features"
    );

    let environment = mapping(field(config, "containerEnv"), "dev container environment");
    assert_eq!(field(environment, "CC").as_str(), Some("clang"));
    assert_eq!(field(environment, "CXX").as_str(), Some("clang++"));

    let lock: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join(".devcontainer/devcontainer-lock.json"))
            .expect("read dev container feature lock"),
    )
    .expect("parse dev container feature lock");
    let locked_features = mapping(
        field(mapping(&lock, "dev container feature lock"), "features"),
        "locked dev container features",
    );
    assert_eq!(locked_features.len(), features.len());
    for feature in features.keys() {
        let locked = mapping(
            field(locked_features, feature),
            "locked dev container feature",
        );
        let resolved = field(locked, "resolved")
            .as_str()
            .expect("locked feature resolution must be a string");
        let integrity = field(locked, "integrity")
            .as_str()
            .expect("locked feature integrity must be a string");
        assert!(
            resolved.contains("@sha256:"),
            "{feature} must resolve to an immutable digest"
        );
        let (feature_name, _) = feature
            .rsplit_once(':')
            .expect("feature reference must include a major version");
        assert_eq!(
            resolved,
            format!("{feature_name}@{integrity}"),
            "{feature} resolution and integrity must match"
        );
    }

    let dockerfile = fs::read_to_string(root.join(".devcontainer/Dockerfile"))
        .expect("read dev container Dockerfile");
    let base_images = dockerfile
        .lines()
        .filter(|line| line.starts_with("FROM "))
        .collect::<Vec<_>>();
    assert_eq!(base_images.len(), 5);
    for image in base_images {
        assert!(
            image.contains("@sha256:"),
            "dev container base image must be immutable: {image}"
        );
    }
    for required in [
        "node:22.18.0-bookworm-slim",
        "python:3.10.18-slim-bookworm",
        "golang:1.25.0-bookworm",
        "rust:1.96.0-bookworm",
        "mcr.microsoft.com/devcontainers/base:2-bookworm",
        "github.com/rhysd/actionlint/cmd/actionlint@v1.7.12",
        "cargo-deny --version 0.20.2 --locked",
        "cargo-audit --version 0.22.2 --locked",
        "clang",
        "libsecret-1-dev",
        "libwebkit2gtk-4.1-dev",
        "libayatana-appindicator3-dev",
    ] {
        assert!(
            dockerfile.contains(required),
            "dev container Dockerfile is missing {required}"
        );
    }
}

#[test]
fn xtask_centralizes_portable_development_and_ci_checks() {
    let root = repository_root();
    let rust = fs::read_to_string(root.join("xtask/src/checks/rust.rs")).expect("read Rust tasks");
    for required in [
        "./scripts/check_crate_roots.sh",
        "\"clippy\"",
        "\"--workspace\"",
        "\"--all-targets\"",
        "\"test\", \"--locked\", \"--workspace\"",
        "fuzz/Cargo.toml",
    ] {
        assert!(rust.contains(required), "Rust xtask is missing {required}");
    }

    let sdk = fs::read_to_string(root.join("xtask/src/checks/sdk.rs")).expect("read SDK tasks");
    for required in [
        "./sdk/scripts/install-codegen-tools",
        "./sdk/scripts/check-breaking",
        "./sdk/scripts/generate",
        "./sdk/scripts/check-generated",
        "npm",
        "ruff",
        "mypy",
        "gofmt",
        "\"test\", \"-mod=readonly\", \"./...\"",
        "\"vet\", \"-mod=readonly\", \"./...\"",
    ] {
        assert!(sdk.contains(required), "SDK xtask is missing {required}");
    }

    let surfaces =
        fs::read_to_string(root.join("xtask/src/checks/surfaces.rs")).expect("read surface tasks");
    for required in [
        "\"fmt\"",
        "apps/desktop/src-tauri/Cargo.toml",
        "\"--check\"",
        "npm",
        "\"audit\", \"--audit-level=high\"",
        "scripts/docs-site",
        "scripts/ci/test-contracts.sh",
        "actionlint",
        "apps/desktop/src-tauri/Cargo.lock",
    ] {
        assert!(
            surfaces.contains(required),
            "surface xtask is missing {required}"
        );
    }

    assert_eq!(
        fs::read_to_string(root.join(".gitattributes"))
            .expect("read repository line-ending policy")
            .lines()
            .find(|line| !line.starts_with('#') && !line.is_empty()),
        Some("* text=auto eol=lf")
    );

    let hook =
        fs::read_to_string(root.join(".githooks/pre-commit")).expect("read local pre-commit hook");
    assert!(hook.contains("cargo xtask pre-commit"));
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
fn windows_native_acceptance_cannot_be_masked_by_a_later_command() {
    let premerge = workflow("premerge.yml");
    let premerge_windows = job(jobs(&premerge), "windows-runtime");
    assert_eq!(
        field(
            named_step(
                premerge_windows,
                "Run authenticated Windows native acceptance"
            ),
            "run"
        )
        .as_str(),
        Some("cargo test --locked -p colossus-windows-native -- --nocapture")
    );
    assert_eq!(
        field(
            named_step(premerge_windows, "Run Windows Colossus-home acceptance"),
            "run"
        )
        .as_str(),
        Some("cargo test --locked -p colossus-home --lib -- --nocapture")
    );
    assert_eq!(
        field(
            named_step(
                premerge_windows,
                "Run Windows Codex credential-store acceptance"
            ),
            "run"
        )
        .as_str(),
        Some("cargo test --locked -p colossus-codex-auth --lib -- --nocapture")
    );
    assert_eq!(
        field(
            named_step(
                premerge_windows,
                "Run Windows worker and AppContainer escape acceptance"
            ),
            "run"
        )
        .as_str(),
        Some(
            "cargo test --locked -p colossus-cli --test worker_smoke --test windows_sandbox -- --nocapture"
        )
    );
    for name in [
        "Run authenticated Windows native acceptance",
        "Run Windows Colossus-home acceptance",
        "Run Windows Codex credential-store acceptance",
        "Run Windows worker and AppContainer escape acceptance",
        "Prepare Windows Managed Local executables",
        "Lint the Windows native Desktop bridge",
        "Test the Windows native Desktop bridge",
    ] {
        assert_eq!(
            field(named_step(premerge_windows, name), "continue-on-error").as_bool(),
            Some(true),
            "{name} must preserve later independent outcomes"
        );
    }
    let aggregate = named_step(premerge_windows, "Require every Windows acceptance check");
    let aggregate_run = field(aggregate, "run")
        .as_str()
        .expect("Windows acceptance aggregate must be a script");
    for outcome in [
        "TYPECHECK_OUTCOME",
        "RENDERER_TEST_OUTCOME",
        "CONTRACT_OUTCOME",
        "NATIVE_OUTCOME",
        "HOME_OUTCOME",
        "CODEX_AUTH_OUTCOME",
        "WORKER_OUTCOME",
        "PREPARE_OUTCOME",
        "CLIPPY_OUTCOME",
        "NATIVE_TEST_OUTCOME",
    ] {
        assert!(
            aggregate_run.contains(outcome),
            "Windows acceptance aggregate is missing {outcome}"
        );
    }

    let release = workflow("release.yml");
    let release_job = job(jobs(&release), "artifacts");
    assert_eq!(
        field(
            named_step(release_job, "Run Windows native runtime acceptance"),
            "run"
        )
        .as_str(),
        Some("cargo test --locked -p colossus-windows-native -- --nocapture")
    );
    assert_eq!(
        field(
            named_step(release_job, "Run Windows Colossus-home acceptance"),
            "run"
        )
        .as_str(),
        Some("cargo test --locked -p colossus-home --lib -- --nocapture")
    );
    assert_eq!(
        field(
            named_step(release_job, "Run Windows Codex credential-store acceptance"),
            "run"
        )
        .as_str(),
        Some("cargo test --locked -p colossus-codex-auth --lib -- --nocapture")
    );
    assert_eq!(
        field(
            named_step(release_job, "Run Windows worker and sandbox acceptance"),
            "run"
        )
        .as_str(),
        Some(
            "cargo test --locked -p colossus-cli --test worker_smoke --test windows_sandbox -- --nocapture"
        )
    );
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
