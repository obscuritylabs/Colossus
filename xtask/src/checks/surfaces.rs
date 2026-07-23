use std::{env, ffi::OsString};

use crate::repository::Repository;

pub(super) fn sidecar(repository: &Repository) -> Result<(), String> {
    repository
        .task(cargo_program())
        .args([
            "test",
            "--locked",
            "--package",
            "colossus-sidecar-protocol",
            "--package",
            "colossus-sidecar",
        ])
        .run()
}

pub(super) fn desktop(repository: &Repository) -> Result<(), String> {
    repository
        .task("npm")
        .args(["ci", "--ignore-scripts"])
        .current_dir("apps/desktop")
        .run()?;
    repository
        .task("npm")
        .args(["audit", "--audit-level=high"])
        .current_dir("apps/desktop")
        .run()?;
    repository
        .task("npm")
        .args(["run", "check"])
        .current_dir("apps/desktop")
        .run()?;
    repository
        .task("npm")
        .args(["run", "build"])
        .current_dir("apps/desktop")
        .run()
}

pub(super) fn docs(repository: &Repository) -> Result<(), String> {
    repository.task("./scripts/docs-site").arg("build").run()
}

pub(super) fn workflows(repository: &Repository) -> Result<(), String> {
    repository.task("./scripts/ci/test-contracts.sh").run()?;
    repository.task("actionlint").run()
}

pub(super) fn dependencies(repository: &Repository) -> Result<(), String> {
    cargo(
        repository,
        [
            "deny",
            "--locked",
            "check",
            "-A",
            "license-not-encountered",
            "licenses",
            "sources",
            "bans",
        ],
    )?;
    cargo(
        repository,
        ["deny", "--locked", "check", "-D", "warnings", "advisories"],
    )?;
    cargo(
        repository,
        ["audit", "-D", "warnings", "--file", "Cargo.lock"],
    )?;
    cargo(
        repository,
        [
            "deny",
            "--manifest-path",
            "fuzz/Cargo.toml",
            "--config",
            "deny.toml",
            "--locked",
            "check",
            "-A",
            "license-not-encountered",
            "licenses",
            "sources",
            "bans",
        ],
    )?;
    cargo(
        repository,
        [
            "deny",
            "--manifest-path",
            "fuzz/Cargo.toml",
            "--config",
            "deny.toml",
            "--locked",
            "check",
            "-D",
            "warnings",
            "advisories",
        ],
    )?;
    cargo(
        repository,
        [
            "audit",
            "--no-fetch",
            "-D",
            "warnings",
            "--file",
            "fuzz/Cargo.lock",
        ],
    )?;
    cargo(
        repository,
        [
            "deny",
            "--manifest-path",
            "apps/desktop/src-tauri/Cargo.toml",
            "--config",
            "deny.toml",
            "--locked",
            "check",
            "-A",
            "duplicate",
            "-A",
            "license-not-encountered",
            "licenses",
            "sources",
            "bans",
        ],
    )?;
    cargo(
        repository,
        [
            "audit",
            "--no-fetch",
            "--file",
            "apps/desktop/src-tauri/Cargo.lock",
        ],
    )
}

fn cargo<const N: usize>(repository: &Repository, args: [&str; N]) -> Result<(), String> {
    repository.task(cargo_program()).args(args).run()
}

fn cargo_program() -> OsString {
    env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}
