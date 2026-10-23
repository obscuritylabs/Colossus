use std::{env, ffi::OsString};

use super::stages::{CompletedStages, Stage};
use crate::{cli::RustCommand, repository::Repository};

pub(super) fn focused(
    repository: &Repository,
    command: RustCommand,
    packages: &[String],
    arguments: &[String],
) -> Result<(), String> {
    repository
        .task(cargo())
        .args(focused_arguments(command, packages, arguments))
        .run()
}

fn focused_arguments(
    command: RustCommand,
    packages: &[String],
    arguments: &[String],
) -> Vec<OsString> {
    let mut selected = vec![
        OsString::from(command.to_string()),
        OsString::from("--locked"),
    ];
    for package in packages {
        selected.extend([OsString::from("-p"), OsString::from(package)]);
    }
    selected.extend(arguments.iter().map(OsString::from));
    selected
}

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

pub(super) fn full(repository: &Repository, completed: &mut CompletedStages) -> Result<(), String> {
    pre_commit(repository)?;
    super::surfaces::browser_brokers(repository)?;
    completed.run(Stage::Computer, || super::surfaces::computer(repository))?;
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

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use crate::cli::RustCommand;

    use super::focused_arguments;

    #[test]
    fn focused_commands_lock_and_select_only_requested_packages() {
        let packages = ["colossus-policy".to_owned(), "colossus-domain".to_owned()];
        for (command, name) in [
            (RustCommand::Check, "check"),
            (RustCommand::Clippy, "clippy"),
            (RustCommand::Test, "test"),
        ] {
            assert_eq!(
                focused_arguments(command, &packages, &[]),
                [
                    name,
                    "--locked",
                    "-p",
                    "colossus-policy",
                    "-p",
                    "colossus-domain"
                ]
                .map(OsString::from)
            );
        }
    }

    #[test]
    fn focused_arguments_preserve_features_targets_and_test_filters() {
        let arguments = [
            "--features",
            "special-feature",
            "--test",
            "selected_target",
            "a test filter",
            "--",
            "--exact",
        ]
        .map(str::to_owned);
        assert_eq!(
            focused_arguments(
                RustCommand::Test,
                &["colossus-policy".to_owned()],
                &arguments
            ),
            ["test", "--locked", "-p", "colossus-policy"]
                .into_iter()
                .map(OsString::from)
                .chain(arguments.iter().map(OsString::from))
                .collect::<Vec<_>>()
        );
    }
}
