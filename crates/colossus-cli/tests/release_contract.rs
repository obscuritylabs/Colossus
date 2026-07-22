//! Repository contracts for release readiness and six-platform CLI plus Desktop drafts.

mod support;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs};
use support::{field, job, jobs, mapping, named_step, repository_root, workflow};

#[test]
fn release_workflow_has_exactly_six_native_cli_targets() {
    let workflow = workflow("release.yml");
    let jobs = jobs(&workflow);
    let artifacts = job(jobs, "artifacts");
    let matrix = mapping(
        field(mapping(field(artifacts, "strategy"), "strategy"), "matrix"),
        "matrix",
    );
    let targets = field(matrix, "include")
        .as_array()
        .expect("release matrix")
        .iter()
        .map(|entry| {
            let entry = mapping(entry, "release entry");
            (
                field(entry, "runner").as_str().expect("runner").to_owned(),
                field(entry, "target").as_str().expect("target").to_owned(),
                field(entry, "archive")
                    .as_str()
                    .expect("archive")
                    .to_owned(),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        targets,
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
fn tag_validation_and_draft_publication_fail_closed() {
    let workflow = workflow("release.yml");
    let jobs = jobs(&workflow);
    assert_eq!(
        field(job(jobs, "gate"), "name").as_str(),
        Some("Colossus release gate")
    );
    let source = fs::read_to_string(repository_root().join(".github/workflows/release.yml"))
        .expect("read release workflow");
    for required in [
        "git cat-file -t",
        "git merge-base --is-ancestor",
        "workspace_version",
        "grep -F \"## [$version]\" CHANGELOG.md",
        "publish_draft=false",
        "--draft --verify-tag --generate-notes",
        "refusing to retain unexpected draft asset",
        "test \"$(find dist -maxdepth 1 -type f | wc -l | tr -d ' ')\" -eq 14",
    ] {
        assert!(
            source.contains(required),
            "release workflow is missing {required}"
        );
    }
    let draft = job(jobs, "draft-release");
    let permissions = mapping(field(draft, "permissions"), "draft permissions");
    assert_eq!(field(permissions, "contents").as_str(), Some("write"));
    assert!(
        field(draft, "if")
            .as_str()
            .is_some_and(|condition| condition.contains("publish_draft == 'true'"))
    );
}

#[test]
fn release_readiness_allows_only_the_public_python_sdk() {
    let source = fs::read_to_string(repository_root().join("release/verify-release-readiness.sh"))
        .expect("read release readiness script");
    for required in [
        "legacy_python_sources=$(git ls-files -- '*.py' ':(exclude)sdk/python/**')",
        "[ -e pyproject.toml ]",
        "[ -n \"$legacy_python_sources\" ]",
        "tracked Python source outside sdk/python",
    ] {
        assert!(
            source.contains(required),
            "release readiness is missing {required}"
        );
    }
    assert!(
        !source.contains("[ -n \"$(git ls-files '*.py')\" ]"),
        "release readiness must not reject the intentional public Python SDK"
    );
}

#[test]
fn platform_jobs_combine_acceptance_packaging_install_and_bundle_smoke() {
    let workflow = workflow("release.yml");
    let artifacts = job(jobs(&workflow), "artifacts");
    for step in [
        "Run Unix native sandbox acceptance",
        "Run Windows runtime and sandbox acceptance",
        "Build locked release binary",
        "Package and verify Unix release",
        "Package and verify Windows release",
        "Upload release archive and checksum",
    ] {
        named_step(artifacts, step);
    }

    let unix = fs::read_to_string(repository_root().join("scripts/ci/release-unix.sh"))
        .expect("read Unix release script");
    let windows = fs::read_to_string(repository_root().join("scripts/ci/release-windows.ps1"))
        .expect("read Windows release script");
    for source in [&unix, &windows] {
        for required in [
            "access",
            "pinned",
            "bundle.key.inspect",
            "pack.trust.add",
            "bundle build",
            "bundle verify",
            "bundle install",
        ] {
            assert!(
                source.contains(required),
                "release smoke is missing {required}"
            );
        }
        assert!(!source.contains("allow_actions"));
        assert!(!source.contains("approval_actions"));
    }
    assert!(unix.contains("allow: [bundle.verify]"));
    assert!(unix.contains(
        "requireApproval: [bundle.key.inspect, pack.trust.add, bundle.build, bundle.install]"
    ));
    assert!(windows.contains("allow = @(\"bundle.verify\")"));
    assert!(windows.contains(
        "requireApproval = @(\"bundle.key.inspect\", \"pack.trust.add\", \"bundle.build\", \"bundle.install\")"
    ));
}

#[test]
fn release_readiness_verifier_is_evergreen_and_pinned() {
    assert!(
        !repository_root()
            .join("release/verify-local-cutover.sh")
            .exists()
    );
    let script = fs::read_to_string(repository_root().join("release/verify-release-readiness.sh"))
        .expect("read release verifier");
    for required in [
        "rustc 1.96.0",
        "cargo-deny 0.20.2",
        "cargo-audit 0.22.2",
        "cargo fmt --all -- --check",
        "cargo clippy --locked --workspace --all-targets -- -D warnings",
        "cargo test --locked --workspace",
        "git ls-files -- '*.py' ':(exclude)sdk/python/**'",
        "release-readiness verification passed",
    ] {
        assert!(
            script.contains(required),
            "release verifier is missing {required}"
        );
    }
    assert!(!script.contains("cutover verification"));
}

#[test]
fn linux_profile_and_release_package_remain_hardened() {
    let template = fs::read_to_string(repository_root().join("release/colossus.apparmor.in"))
        .expect("read AppArmor template");
    assert!(template.contains("profile colossus \"@COLOSSUS_BINARY@\""));
    assert!(template.contains("  userns,"));
    assert!(!template.contains("/**/colossus"));

    let installer = fs::read_to_string(repository_root().join("release/install-apparmor.sh"))
        .expect("read AppArmor installer");
    for required in [
        "[ ! -L \"$requested_binary\" ]",
        "realpath -e",
        "stat -c '%u'",
        "stat -c '%g'",
        "0$mode & 020",
        "0$mode & 002",
        "apparmor_parser -r",
        "/etc/apparmor.d/colossus",
    ] {
        assert!(installer.contains(required));
    }
    let unix = fs::read_to_string(repository_root().join("scripts/ci/release-unix.sh"))
        .expect("read Unix release script");
    assert!(unix.starts_with("#!/bin/bash\n"));
    assert!(!unix.contains("mapfile"));
    assert!(unix.contains("release/install-apparmor.sh"));
    assert!(unix.contains("release/colossus.apparmor.in"));
}

#[test]
fn release_bundle_publisher_identity_is_self_consistent() {
    let source = fs::read_to_string(repository_root().join("release/bundle-publisher.json"))
        .expect("read publisher identity");
    let identity: Value = serde_json::from_str(&source).expect("publisher JSON");
    let identity = mapping(&identity, "publisher identity");
    assert_eq!(field(identity, "publisher").as_str(), Some("colossus"));
    assert_eq!(field(identity, "algorithm").as_str(), Some("ed25519"));
    assert_eq!(
        field(identity, "purpose").as_str(),
        Some("offline-bundle-manifest-signing")
    );
    let public_key = BASE64
        .decode(field(identity, "public_key").as_str().expect("public key"))
        .expect("base64 public key");
    assert_eq!(public_key.len(), 32);
    let key_id = hex::encode(Sha256::digest(public_key));
    assert_eq!(field(identity, "key_id").as_str(), Some(key_id.as_str()));
}

#[test]
fn canonical_binary_and_oci_proxy_context_remain_bounded() {
    let manifest = fs::read_to_string(repository_root().join("crates/colossus-cli/Cargo.toml"))
        .expect("read CLI manifest");
    assert!(manifest.contains("name = \"colossus\""));
    assert!(!manifest.contains("colossus-rs"));

    let dockerignore = fs::read_to_string(repository_root().join(".dockerignore"))
        .expect("read Docker ignore rules");
    let rules = dockerignore.lines().collect::<BTreeSet<_>>();
    assert!(!rules.contains("target/"));
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
            "Docker context is missing {required}"
        );
    }
}
