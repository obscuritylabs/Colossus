use std::{env, ffi::OsString};

use crate::repository::Repository;

pub(super) fn pre_commit(repository: &Repository) -> Result<(), String> {
    repository.task("git").args(["diff", "--check"]).run()?;
    repository
        .task("git")
        .args(["diff", "--cached", "--check"])
        .run()?;
    format_and_structure(repository)
}

pub(super) fn dev(repository: &Repository) -> Result<(), String> {
    pre_commit(repository)?;
    repository
        .task(cargo())
        .args(["test", "--locked", "--workspace", "--lib"])
        .run()
}

pub(super) fn full(repository: &Repository) -> Result<(), String> {
    pre_commit(repository)?;
    repository
        .task(cargo())
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .quiet_stdout()
        .run()?;
    repository
        .task(cargo())
        .args([
            "metadata",
            "--locked",
            "--manifest-path",
            "fuzz/Cargo.toml",
            "--no-deps",
            "--format-version",
            "1",
        ])
        .quiet_stdout()
        .run()?;
    repository
        .task(cargo())
        .args([
            "clippy",
            "--locked",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ])
        .run()?;
    repository
        .task(cargo())
        .args(["test", "--locked", "--workspace"])
        .run()?;
    repository
        .task(cargo())
        .args([
            "clippy",
            "--locked",
            "--manifest-path",
            "fuzz/Cargo.toml",
            "--bins",
            "--",
            "-D",
            "warnings",
        ])
        .run()
}

fn format_and_structure(repository: &Repository) -> Result<(), String> {
    repository
        .task(cargo())
        .args(["fmt", "--all", "--", "--check"])
        .run()?;
    repository
        .task(cargo())
        .args([
            "fmt",
            "--manifest-path",
            "fuzz/Cargo.toml",
            "--all",
            "--",
            "--check",
        ])
        .run()?;
    repository.task("./scripts/check_crate_roots.sh").run()
}

fn cargo() -> OsString {
    env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}
