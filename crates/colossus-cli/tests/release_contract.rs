//! Repository contracts for release readiness and six-platform CLI plus Desktop drafts.

mod support;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, process::Command};
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
            ("ubuntu-latest-m", "x86_64-unknown-linux-musl", "tar.gz"),
            ("ubuntu-24.04-arm", "aarch64-unknown-linux-musl", "tar.gz"),
            ("windows-latest-l", "x86_64-pc-windows-msvc", "zip",),
            ("windows-11-arm", "aarch64-pc-windows-msvc", "zip"),
        ]
        .into_iter()
        .map(|(runner, target, archive)| (runner.into(), target.into(), archive.into()))
        .collect()
    );
}

#[test]
fn internal_rust_packages_are_not_registry_publishable() {
    for manifest in [
        "Cargo.toml",
        "apps/desktop/src-tauri/Cargo.toml",
        "fuzz/Cargo.toml",
    ] {
        let output = Command::new("cargo")
            .args([
                "metadata",
                "--locked",
                "--no-deps",
                "--format-version",
                "1",
                "--manifest-path",
                manifest,
            ])
            .current_dir(repository_root())
            .output()
            .expect("run cargo metadata");
        assert!(
            output.status.success(),
            "cargo metadata failed for {manifest}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let metadata: Value = serde_json::from_slice(&output.stdout).expect("parse cargo metadata");
        for package in field(mapping(&metadata, "cargo metadata"), "packages")
            .as_array()
            .expect("cargo packages")
        {
            let package = mapping(package, "cargo package");
            assert_eq!(
                field(package, "publish").as_array().map(Vec::len),
                Some(0),
                "{} must remain publish=false",
                field(package, "name").as_str().expect("package name")
            );
        }
    }
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
        "release_channel=validation_only",
        "--draft --verify-tag --generate-notes",
        "refusing to retain unexpected draft asset",
        "test \"$(find dist -maxdepth 1 -type f | wc -l | tr -d ' ')\" -eq 21",
        "test \"$(find dist -maxdepth 1 -type f | wc -l | tr -d ' ')\" -eq 22",
    ] {
        assert!(
            source.contains(required),
            "release workflow is missing {required}"
        );
    }
    let draft = job(jobs, "draft-release");
    let permissions = mapping(field(draft, "permissions"), "draft permissions");
    assert_eq!(field(permissions, "contents").as_str(), Some("write"));
    // `!cancelled()` keeps a status function so the intentionally skipped Desktop
    // preview jobs cannot skip this job before its explicit gate check runs, while
    // still refusing to publish a draft once the release workflow is cancelled.
    assert_eq!(
        field(draft, "if").as_str(),
        Some(
            "${{ !cancelled() && needs.validate.outputs.publish_draft == 'true' && needs.gate.result == 'success' }}"
        )
    );
    named_step(draft, "Check out the exact release verifier");
    named_step(draft, "Verify complete release asset set");
}

#[test]
fn unsigned_desktop_builds_do_not_receive_updater_configuration() {
    let source = fs::read_to_string(repository_root().join(".github/workflows/release.yml"))
        .expect("read release workflow");
    for required in [
        "COLOSSUS_DESKTOP_UPDATE_ENDPOINT: \"\"",
        "COLOSSUS_DESKTOP_UPDATE_PUBLIC_KEY: \"\"",
    ] {
        assert!(
            source.contains(required),
            "unsigned Desktop workflow is missing {required}"
        );
    }
    for forbidden in [
        "TAURI_SIGNING_PRIVATE_KEY",
        "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
        "DESKTOP_UPDATE_PRIVATE_KEY",
        "MACOS_DEVELOPER_ID_P12",
        "MACOS_NOTARY",
        "secrets.MACOS",
        "vars.MACOS",
    ] {
        assert!(
            !source.contains(forbidden),
            "core release workflow must not require Desktop credential {forbidden}"
        );
    }
}

#[test]
fn developer_preview_is_explicitly_ad_hoc_labeled_and_prerelease() {
    let workflow = workflow("release.yml");
    let release_jobs = jobs(&workflow);
    let build = job(release_jobs, "desktop_macos_build");
    let signing = job(release_jobs, "desktop_macos");

    assert_eq!(
        field(build, "if").as_str(),
        Some("needs.validate.outputs.target_channel != 'stable'")
    );
    assert_eq!(
        field(signing, "if").as_str(),
        Some("needs.validate.outputs.target_channel != 'stable'")
    );
    assert!(
        !named_step(build, "Configure the non-production Desktop code identity").contains_key("if")
    );
    assert!(
        !named_step(
            signing,
            "Configure Developer Preview or validation-only ad-hoc signing"
        )
        .contains_key("if")
    );

    let source = fs::read_to_string(repository_root().join(".github/workflows/release.yml"))
        .expect("read release workflow");
    for required in [
        "-preview\\.([1-9][0-9]*)$",
        "tag_channel=developer_preview",
        "COLOSSUS_DESKTOP_RELEASE_CHANNEL: ${{ needs.validate.outputs.release_channel }}",
        "Colossus-Desktop-DEVELOPER-PREVIEW-${RELEASE_TAG}-aarch64-apple-darwin.zip",
        "--draft --prerelease --verify-tag --generate-notes",
        "Developer Preview (Unnotarized)",
        "ad-hoc signed and not notarized by Apple",
        "preview_checksum=\"Colossus-Desktop-DEVELOPER-PREVIEW-${RELEASE_TAG}-aarch64-apple-darwin.zip.sha256\"",
        "shasum -a 256 --check $preview_checksum",
        "System Settings > Privacy & Security > Open Anyway",
        "Do not disable Gatekeeper globally",
        "Automatic Desktop updates are disabled in this unsigned preview",
        "test ! -e \"dist/developer_preview.json\"",
        ".prerelease == $prerelease",
    ] {
        assert!(
            source.contains(required),
            "Developer Preview contract is missing {required}"
        );
    }
    assert_eq!(
        source.matches("--prerelease").count(),
        1,
        "only the explicit Developer Preview branch may mark a GitHub release as prerelease"
    );
}

#[test]
fn release_readiness_allows_only_approved_python_sources() {
    let source = fs::read_to_string(repository_root().join("release/verify-release-readiness.sh"))
        .expect("read release readiness script");
    for required in [
        "legacy_python_sources=$(git ls-files -- '*.py' ':(exclude)sdk/python/**' \\",
        "':(exclude)scripts/ci/normalize_python_sdist.py' \\",
        "':(exclude)examples/sdk/integration/server.py' \\",
        "':(exclude)examples/sdk/provider-failure/server.py')",
        "[ -e pyproject.toml ]",
        "[ -n \"$legacy_python_sources\" ]",
        "tracked Python source outside the maintained public Python SDK and SDK fixtures",
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
    assert!(
        !source.contains("':(exclude)examples/sdk/**'"),
        "release readiness must exempt only the maintained SDK fixture files"
    );
}

#[test]
fn platform_jobs_combine_acceptance_packaging_install_and_bundle_smoke() {
    let workflow = workflow("release.yml");
    let artifacts = job(jobs(&workflow), "artifacts");
    for step in [
        "Run Unix native sandbox acceptance",
        "Run Windows native runtime acceptance",
        "Run Windows worker and sandbox acceptance",
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
    assert!(unix.contains("schemaVersion: 2"));
    assert!(!unix.contains("schemaVersion: 1"));
    assert!(unix.contains("providerProfile: echo"));
    assert!(windows.contains("allow = @(\"bundle.verify\")"));
    assert!(windows.contains(
        "requireApproval = @(\"bundle.key.inspect\", \"pack.trust.add\", \"bundle.build\", \"bundle.install\")"
    ));
    assert!(windows.contains("schemaVersion = 2"));
    assert!(!windows.contains("schemaVersion = 1"));
    assert!(windows.contains("providerProfile = \"echo\""));
    assert!(windows.contains("[IO.File]::WriteAllText("));
    assert!(windows.contains("$hash  $package.zip`n"));
    assert!(!windows.contains("| Set-Content -Encoding ascii \"${archive}.sha256\""));
}

#[test]
fn public_bootstrap_installers_are_fixed_origin_bounded_and_release_owned() {
    let workflow = workflow("release.yml");
    let release_jobs = jobs(&workflow);
    let bootstrap = job(release_jobs, "bootstrap_installers");
    named_step(bootstrap, "Validate bootstrap installer syntax");
    named_step(bootstrap, "Stage immutable bootstrap installer assets");
    named_step(bootstrap, "Upload bootstrap installers and checksums");

    let workflow_source =
        fs::read_to_string(repository_root().join(".github/workflows/release.yml"))
            .expect("read release workflow");
    for required in [
        "bootstrap_installers=${{ needs.bootstrap_installers.result }}",
        "dist/colossus-install.sh",
        "dist/colossus-install.ps1",
        "colossus-install.sh.sha256",
        "colossus-install.ps1.sha256",
    ] {
        assert!(
            workflow_source.contains(required),
            "bootstrap release contract is missing {required}"
        );
    }

    let unix = fs::read_to_string(repository_root().join("release/bootstrap/install.sh"))
        .expect("read Unix bootstrap");
    let windows = fs::read_to_string(repository_root().join("release/bootstrap/install.ps1"))
        .expect("read PowerShell bootstrap");
    for source in [&unix, &windows] {
        for required in [
            "obscuritylabs/Colossus",
            "api.github.com",
            "release-assets.githubusercontent.com",
        ] {
            assert!(source.contains(required), "bootstrap is missing {required}");
        }
        assert!(!source.contains("COLOSSUS_DIST_ORIGIN"));
    }
    assert!(unix.matches("--noproxy '*'").count() >= 2);
    for required in [
        "maximum_metadata_bytes=1048576",
        "maximum_archive_bytes=268435456",
        "maximum_expanded_bytes=268435456",
        "expanded archive is larger than its fixed limit",
        "archive contains a link or special file",
        "archive checksum mismatch",
        "package metadata version mismatch",
        "--version",
        "--prefix",
        "--channel",
        "--dry-run",
        "--no-modify-path",
        "--yes",
    ] {
        assert!(
            unix.contains(required),
            "Unix bootstrap is missing {required}"
        );
    }
    for required in [
        "$maximumMetadataBytes = 1MB",
        "$maximumArchiveBytes = 256MB",
        "ResponseHeadersRead",
        "archive contains a link or reparse point",
        "archive checksum mismatch",
        "package metadata mismatch",
        "[string]$Version",
        "[string]$Prefix",
        "[string]$Channel",
        "[switch]$DryRun",
        "[switch]$NoModifyPath",
        "[switch]$Yes",
    ] {
        assert!(
            windows.contains(required),
            "PowerShell bootstrap is missing {required}"
        );
    }

    for installer_path in ["release/install.sh", "release/install.ps1"] {
        let installer = fs::read_to_string(repository_root().join(installer_path))
            .unwrap_or_else(|error| panic!("read {installer_path}: {error}"));
        for required in [
            "install-metadata",
            "install.json",
            "schemaVersion",
            "distributionOrigin",
            "installerKind",
        ] {
            assert!(
                installer.contains(required),
                "{installer_path} is missing receipt contract {required}"
            );
        }
    }
}

#[test]
fn published_stable_releases_are_installed_anonymously_on_all_host_classes() {
    let workflow = workflow("public-distribution.yml");
    let distribution_jobs = jobs(&workflow);
    let unix = job(distribution_jobs, "unix");
    let windows = job(distribution_jobs, "windows");
    named_step(
        unix,
        "Download and verify the public Unix bootstrap anonymously",
    );
    named_step(
        windows,
        "Download and verify the public PowerShell bootstrap anonymously",
    );
    assert_eq!(field(unix, "timeout-minutes").as_u64(), Some(15));
    assert_eq!(field(windows, "timeout-minutes").as_u64(), Some(15));

    let source =
        fs::read_to_string(repository_root().join(".github/workflows/public-distribution.yml"))
            .expect("read public distribution workflow");
    for required in [
        "permissions: {}",
        "types: [published]",
        "ubuntu-latest",
        "macos-latest",
        "windows-latest",
        "releases/latest/download",
        "colossus-install.sh.sha256",
        "colossus-install.ps1.sha256",
        "--noproxy '*'",
        "installerKind",
        "--output json update check",
    ] {
        assert!(
            source.contains(required),
            "public distribution verification is missing {required}"
        );
    }
    for forbidden in ["GH_TOKEN", "github.token", "Authorization"] {
        assert!(
            !source.contains(forbidden),
            "anonymous distribution verification must not use {forbidden}"
        );
    }
}

#[test]
fn package_manager_definitions_pin_prebuilt_assets_and_refuse_self_ownership() {
    let formula =
        fs::read_to_string(repository_root().join("packaging/homebrew/Formula/colossus.rb"))
            .expect("read Homebrew formula");
    for required in [
        "Hardware::CPU.arm?",
        "aarch64-apple-darwin.tar.gz",
        "x86_64-apple-darwin.tar.gz",
        "sha256",
        "COLOSSUS_INSTALLER_KIND: \"homebrew\"",
        "colossus --version",
    ] {
        assert!(
            formula.contains(required),
            "Homebrew formula is missing {required}"
        );
    }
    for forbidden in ["system \"cargo\"", "cargo install", "installerKind: direct"] {
        assert!(
            !formula.contains(forbidden),
            "Homebrew formula must not contain {forbidden}"
        );
    }

    let flake = fs::read_to_string(repository_root().join("flake.nix")).expect("read Nix flake");
    for required in [
        "aarch64-darwin",
        "x86_64-darwin",
        "aarch64-linux",
        "x86_64-linux",
        "COLOSSUS_INSTALLER_KIND nix",
        "sourceProvenance",
        "binaryNativeCode",
        "--version",
    ] {
        assert!(flake.contains(required), "Nix flake is missing {required}");
    }
    for forbidden in ["cargo build", "cargo install", "installerKind = \"direct\""] {
        assert!(
            !flake.contains(forbidden),
            "Nix package must not contain {forbidden}"
        );
    }
    let lock: Value = serde_json::from_str(
        &fs::read_to_string(repository_root().join("flake.lock")).expect("read flake lock"),
    )
    .expect("parse flake lock");
    assert_eq!(
        field(mapping(&lock, "flake lock"), "version").as_u64(),
        Some(7)
    );
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

    for workflow_path in [".github/workflows/pr.yml", ".github/workflows/release.yml"] {
        let source = fs::read_to_string(repository_root().join(workflow_path))
            .unwrap_or_else(|error| panic!("read {workflow_path}: {error}"));
        let staging_directory =
            r#"install_dir="/colossus-ci-${GITHUB_RUN_ID}-${GITHUB_RUN_ATTEMPT}""#;
        let expected_staging_count = if workflow_path.ends_with("/pr.yml") {
            1
        } else {
            2
        };
        assert!(
            source.contains(staging_directory),
            "{workflow_path} must stage the exact-path attachment in a run-unique root-level directory"
        );
        assert_eq!(
            source.matches(staging_directory).count(),
            expected_staging_count,
            "{workflow_path} must use the hardened staging directory for every Linux AppArmor setup"
        );
        assert!(
            source.contains(r#"staged_binary="$install_dir/colossus""#),
            "{workflow_path} must install and profile the root-controlled binary"
        );
        assert_eq!(
            source
                .matches("sudo install -o root -g root -m 0755 /bin/true \"$staged_binary\"")
                .count(),
            expected_staging_count,
            "{workflow_path} must fail fast on AppArmor path or parser errors before compiling"
        );
        for forbidden in [
            "/usr/local/libexec/colossus-ci",
            "/usr/lib/colossus-ci",
            "/opt/colossus-ci",
        ] {
            assert!(
                !source.contains(forbidden),
                "{workflow_path} must not rely on runner-controlled ancestor {forbidden}"
            );
        }
    }
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
fn canonical_binary_and_container_build_contexts_remain_bounded() {
    let manifest = fs::read_to_string(repository_root().join("crates/colossus-cli/Cargo.toml"))
        .expect("read CLI manifest");
    assert!(manifest.contains("name = \"colossus\""));
    assert!(!manifest.contains("colossus-rs"));

    let dockerignore = fs::read_to_string(repository_root().join(".dockerignore"))
        .expect("read Docker ignore rules");
    let rules = dockerignore
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<BTreeSet<_>>();
    for required in [
        ".git/",
        ".colossus/",
        "**/.env",
        "**/.env.*",
        "**/.cargo/credentials.toml",
        "**/target/",
        "**/node_modules/",
        "**/.venv/",
        "**/.codegen/",
        "apps/desktop/src-tauri/binaries/",
    ] {
        assert!(
            rules.contains(required),
            "root Docker context is missing exclusion {required}"
        );
    }
    assert!(
        rules.iter().all(|rule| !rule.starts_with('!')),
        "the general build context must not re-admit generated or credential paths"
    );

    let proxy_ignore =
        fs::read_to_string(repository_root().join("oci-proxy.Dockerfile.dockerignore"))
            .expect("read OCI proxy Docker ignore rules");
    let proxy_rules = proxy_ignore
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>();
    assert_eq!(
        proxy_rules,
        [
            "*",
            "!oci-proxy.Dockerfile",
            "!target/",
            "target/*",
            "!target/*-unknown-linux-musl/",
            "target/*-unknown-linux-musl/*",
            "!target/*-unknown-linux-musl/release/",
            "target/*-unknown-linux-musl/release/*",
            "!target/*-unknown-linux-musl/release/colossus-oci-proxy",
        ],
        "the OCI proxy build context must remain default-deny and artifact-only"
    );

    let premerge = fs::read_to_string(repository_root().join(".github/workflows/premerge.yml"))
        .expect("read pre-merge workflow");
    assert_eq!(
        premerge
            .matches("--ignorefile oci-proxy.Dockerfile.dockerignore")
            .count(),
        1,
        "Podman must explicitly use the Dockerfile-specific ignore rules"
    );
}
