use std::{
    env, fs,
    path::{Path, PathBuf},
};

use crate::repository::Repository;

const GENERATED_PATHS: [&str; 7] = [
    "sdk/generated-inputs.sha256",
    "sdk/go/gen",
    "sdk/go/generated-output.sha256",
    "sdk/python/generated",
    "sdk/python/generated-output.sha256",
    "sdk/typescript/src/gen",
    "sdk/typescript/generated-output.sha256",
];

pub(super) fn check(repository: &Repository, base: &str) -> Result<(), String> {
    if cfg!(windows) {
        return Err(
            "SDK generation currently requires the repository's POSIX generator scripts".to_owned(),
        );
    }

    install_toolchains(repository)?;
    reject_hosted_generators(repository)?;
    repository
        .task("./sdk/scripts/check-breaking")
        .arg(base)
        .run()?;
    let generated_before = generated_binding_state(repository)?;
    repository.task("./sdk/scripts/generate").run()?;
    repository.task("./sdk/scripts/check-generated").run()?;
    require_unchanged_generated_bindings(repository, &generated_before)?;
    repository
        .task("node")
        .args(["--test", "scripts/ci/sdk-release.test.mjs"])
        .run()?;
    repository
        .task("cargo")
        .args([
            "check",
            "--locked",
            "-p",
            "colossus-cli",
            "--example",
            "sdk_ephemeral_local",
        ])
        .run()?;
    check_typescript(repository)?;
    check_python(repository)?;
    check_go(repository)
}

fn install_toolchains(repository: &Repository) -> Result<(), String> {
    repository
        .task("./sdk/scripts/install-codegen-tools")
        .run()?;
    let python = python(repository);
    repository
        .task(&python)
        .args([
            "-m",
            "pip",
            "install",
            "--disable-pip-version-check",
            "--no-deps",
            "--requirement",
            "requirements-dev.txt",
        ])
        .current_dir("sdk/python")
        .run()?;
    repository
        .task(python)
        .args(["-m", "pip", "check"])
        .current_dir("sdk/python")
        .run()
}

fn reject_hosted_generators(repository: &Repository) -> Result<(), String> {
    let contents = fs::read_to_string(repository.path("sdk/buf.gen.yaml"))
        .map_err(|error| format!("could not read sdk/buf.gen.yaml: {error}"))?;
    if has_hosted_generator(&contents) {
        Err("hosted Buf plugins may not receive the Colossus schema".to_owned())
    } else {
        Ok(())
    }
}

fn has_hosted_generator(contents: &str) -> bool {
    contents.lines().any(|line| {
        let field = line.trim_start();
        let field = field.strip_prefix("- ").unwrap_or(field);
        field.starts_with("remote:")
    })
}

#[derive(Debug, Eq, PartialEq)]
struct GeneratedBindingState {
    status: String,
    diff: String,
}

fn generated_binding_state(repository: &Repository) -> Result<GeneratedBindingState, String> {
    let status = repository
        .task("git")
        .args(["status", "--porcelain", "--untracked-files=all", "--"])
        .args(GENERATED_PATHS)
        .output()?;
    let diff = repository
        .task("git")
        .args(["diff", "--binary", "HEAD", "--"])
        .args(GENERATED_PATHS)
        .output()?;
    Ok(GeneratedBindingState { status, diff })
}

fn require_unchanged_generated_bindings(
    repository: &Repository,
    before: &GeneratedBindingState,
) -> Result<(), String> {
    let after = generated_binding_state(repository)?;
    if &after == before {
        Ok(())
    } else {
        Err(format!(
            "SDK generation changed the checked-in binding state; regenerate the SDKs before running this gate:\n{}",
            after.status
        ))
    }
}

fn check_typescript(repository: &Repository) -> Result<(), String> {
    for args in [
        ["run", "check"].as_slice(),
        ["test"].as_slice(),
        ["run", "build"].as_slice(),
    ] {
        repository
            .task("npm")
            .args(args)
            .current_dir("sdk/typescript")
            .run()?;
    }
    repository
        .task("npm")
        .args([
            "exec",
            "--",
            "tsc",
            "-p",
            "tsconfig.examples.json",
            "--noEmit",
        ])
        .current_dir("sdk/typescript")
        .run()?;
    repository
        .task("node")
        .args(["--test", "../scripts/check-typescript-package.test.mjs"])
        .current_dir("sdk/typescript")
        .run()?;
    repository
        .task("node")
        .arg("../scripts/check-typescript-package.mjs")
        .current_dir("sdk/typescript")
        .run()
}

fn check_python(repository: &Repository) -> Result<(), String> {
    let python = python(repository);
    let python_path = env::join_paths(["src", "generated"])
        .map_err(|error| format!("could not construct PYTHONPATH: {error}"))?;
    for args in [
        ["-m", "ruff", "format", "--check", "."].as_slice(),
        ["-m", "ruff", "check", "."].as_slice(),
        ["-m", "mypy"].as_slice(),
    ] {
        repository
            .task(&python)
            .args(args)
            .current_dir("sdk/python")
            .run()?;
    }
    repository
        .task(&python)
        .args([
            "-m",
            "mypy",
            "--strict",
            "examples/durable_run.py",
            "examples/live_run.py",
        ])
        .env("PYTHONPATH", &python_path)
        .current_dir("sdk/python")
        .run()?;
    for fixture in [
        "examples/sdk/integration/server.py",
        "examples/sdk/provider-failure/server.py",
    ] {
        repository
            .task(&python)
            .args(["-m", "mypy", "--strict", fixture])
            .run()?;
    }
    for args in [
        [
            "-m",
            "ruff",
            "format",
            "--check",
            "../../examples/sdk/integration/server.py",
            "../../examples/sdk/provider-failure/server.py",
        ]
        .as_slice(),
        [
            "-m",
            "ruff",
            "check",
            "../../examples/sdk/integration/server.py",
            "../../examples/sdk/provider-failure/server.py",
        ]
        .as_slice(),
    ] {
        repository
            .task(&python)
            .args(args)
            .current_dir("sdk/python")
            .run()?;
    }
    repository
        .task(&python)
        .args(["-m", "unittest", "discover", "-s", "tests"])
        .env("PYTHONPATH", python_path)
        .current_dir("sdk/python")
        .run()?;
    repository
        .task(&python)
        .args(["-m", "build", "--no-isolation"])
        .current_dir("sdk/python")
        .run()?;
    repository
        .task(python)
        .arg("check_package.py")
        .current_dir("sdk/python")
        .run()
}

fn check_go(repository: &Repository) -> Result<(), String> {
    let root = repository.path("sdk/go");
    let mut files = Vec::new();
    collect_go_files(&root, &root, &mut files)?;
    if files.is_empty() {
        return Err("the Go SDK contains no Go files".to_owned());
    }
    let unformatted = repository
        .task("gofmt")
        .arg("-l")
        .args(files)
        .current_dir("sdk/go")
        .output()?;
    if !unformatted.trim().is_empty() {
        return Err(format!("unformatted Go SDK files:\n{unformatted}"));
    }
    repository
        .task("go")
        .args(["test", "-mod=readonly", "./..."])
        .current_dir("sdk/go")
        .run()?;
    repository
        .task("go")
        .args(["vet", "-mod=readonly", "./..."])
        .current_dir("sdk/go")
        .run()
}

fn collect_go_files(root: &Path, directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("could not read {}: {error}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "could not read an entry under {}: {error}",
                directory.display()
            )
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
        if file_type.is_dir() {
            collect_go_files(root, &path, files)?;
        } else if file_type.is_file() && path.extension().is_some_and(|extension| extension == "go")
        {
            files.push(
                path.strip_prefix(root)
                    .expect("collected Go file remains under its root")
                    .to_path_buf(),
            );
        }
    }
    files.sort();
    Ok(())
}

fn python(repository: &Repository) -> PathBuf {
    repository.path("sdk/python/.codegen/bin/python")
}

#[cfg(test)]
mod tests {
    use super::has_hosted_generator;

    #[test]
    fn hosted_buf_plugins_are_rejected_without_matching_comments() {
        assert!(has_hosted_generator(
            "plugins:\n  - remote: buf.build/example/plugin\n"
        ));
        assert!(!has_hosted_generator(
            "# remote: is forbidden\nplugins:\n  - local: protoc-gen-example\n"
        ));
    }
}
