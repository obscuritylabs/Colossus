use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[cfg(windows)]
use std::{
    env,
    fs::File,
    io::{BufRead as _, BufReader},
};

pub(super) struct Task {
    root: PathBuf,
    program: OsString,
    args: Vec<OsString>,
    current_dir: PathBuf,
    env: Vec<(OsString, OsString)>,
    quiet_stdout: bool,
}

impl Task {
    pub(super) fn new(root: &Path, program: impl Into<OsString>) -> Self {
        Self {
            root: root.to_path_buf(),
            program: program.into(),
            args: Vec::new(),
            current_dir: root.to_path_buf(),
            env: Vec::new(),
            quiet_stdout: false,
        }
    }

    pub(super) fn arg(mut self, value: impl Into<OsString>) -> Self {
        self.args.push(value.into());
        self
    }

    pub(super) fn args<I, S>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(values.into_iter().map(Into::into));
        self
    }

    pub(super) fn current_dir(mut self, relative: impl AsRef<Path>) -> Self {
        self.current_dir = self.root.join(relative);
        self
    }

    pub(super) fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub(super) fn quiet_stdout(mut self) -> Self {
        self.quiet_stdout = true;
        self
    }

    pub(super) fn run(self) -> Result<(), String> {
        self.print();
        let mut command = self.command()?;
        if self.quiet_stdout {
            command.stdout(Stdio::null());
        }
        let status = command
            .status()
            .map_err(|error| format!("could not start {}: {error}", display(&self.program)))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "{} exited with {status}",
                display_command(&self.program, &self.args)
            ))
        }
    }

    pub(super) fn output(self) -> Result<String, String> {
        self.print();
        let mut command = self.command()?;
        let output = command
            .stderr(Stdio::inherit())
            .output()
            .map_err(|error| format!("could not start {}: {error}", display(&self.program)))?;
        if !output.status.success() {
            return Err(format!(
                "{} exited with {}",
                display_command(&self.program, &self.args),
                output.status
            ));
        }
        String::from_utf8(output.stdout)
            .map_err(|_| format!("{} produced non-UTF-8 output", display(&self.program)))
    }

    pub(super) fn optional_output(self) -> Result<Option<String>, String> {
        self.print();
        let mut command = self.command()?;
        let output = command
            .stderr(Stdio::null())
            .output()
            .map_err(|error| format!("could not start {}: {error}", display(&self.program)))?;
        if !output.status.success() {
            return Ok(None);
        }
        String::from_utf8(output.stdout)
            .map(Some)
            .map_err(|_| format!("{} produced non-UTF-8 output", display(&self.program)))
    }

    fn command(&self) -> Result<Command, String> {
        #[cfg(windows)]
        let mut command = if is_posix_shell_script(&self.resolved_program()) {
            let mut command = Command::new(git_bash()?);
            command.arg(&self.program);
            command
        } else if let Some(script) = windows_batch_script(
            &self.program,
            &self.current_dir,
            env::var_os("PATH").as_deref(),
        ) {
            let mut command =
                Command::new(env::var_os("COMSPEC").unwrap_or_else(|| OsString::from("cmd.exe")));
            command.args(["/d", "/c"]).arg(script);
            command
        } else {
            Command::new(&self.program)
        };
        #[cfg(not(windows))]
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .current_dir(&self.current_dir)
            .envs(self.env.iter().map(|(key, value)| (key, value)));
        Ok(command)
    }

    #[cfg(windows)]
    fn resolved_program(&self) -> PathBuf {
        let program = Path::new(&self.program);
        if program.is_absolute() {
            program.to_path_buf()
        } else {
            self.current_dir.join(program)
        }
    }

    fn print(&self) {
        eprintln!("+ {}", display_command(&self.program, &self.args));
    }
}

#[cfg(windows)]
fn windows_batch_script(
    program: &OsStr,
    current_dir: &Path,
    search_path: Option<&OsStr>,
) -> Option<PathBuf> {
    let program = Path::new(program);
    let has_directory = program.components().count() > 1;
    let candidates = if program.is_absolute() {
        vec![program.to_path_buf()]
    } else if has_directory {
        vec![current_dir.join(program)]
    } else {
        std::iter::once(current_dir.to_path_buf())
            .chain(search_path.into_iter().flat_map(env::split_paths))
            .map(|directory| directory.join(program))
            .collect()
    };

    for candidate in candidates {
        if is_batch_extension(&candidate) && candidate.is_file() {
            return Some(candidate);
        }
        if candidate.extension().is_some() {
            continue;
        }
        if candidate.with_extension("exe").is_file() || candidate.with_extension("com").is_file() {
            return None;
        }
        for extension in ["cmd", "bat"] {
            let script = candidate.with_extension(extension);
            if script.is_file() {
                return Some(script);
            }
        }
    }
    None
}

#[cfg(windows)]
fn is_batch_extension(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        })
}

#[cfg(windows)]
fn is_posix_shell_script(path: &Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let mut first_line = String::new();
    if BufReader::new(file).read_line(&mut first_line).is_err() {
        return false;
    }
    matches!(
        first_line.trim_end(),
        "#!/bin/sh" | "#!/bin/bash" | "#!/usr/bin/env sh" | "#!/usr/bin/env bash"
    )
}

#[cfg(windows)]
fn git_bash() -> Result<PathBuf, String> {
    if let Ok(output) = Command::new("git").arg("--exec-path").output()
        && output.status.success()
        && let Ok(exec_path) = String::from_utf8(output.stdout)
    {
        for ancestor in Path::new(exec_path.trim()).ancestors() {
            let candidate = ancestor.join("bin").join("bash.exe");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    if let Some(path) = env::var_os("PATH") {
        for directory in env::split_paths(&path) {
            let candidate = directory.join("bash.exe");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    Err(
        "could not locate a POSIX shell for repository scripts; install Git for Windows with Git Bash"
            .to_owned(),
    )
}

fn display_command(program: &OsStr, args: &[OsString]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(OsString::as_os_str))
        .map(display)
        .collect::<Vec<_>>()
        .join(" ")
}

fn display(value: &OsStr) -> String {
    format!("{value:?}")
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn repository_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask repository parent")
            .to_path_buf()
    }

    #[test]
    fn detects_posix_shell_scripts_with_and_without_extensions() {
        let root = repository_root();
        assert!(is_posix_shell_script(
            &root.join("scripts/check_crate_roots.sh")
        ));
        assert!(is_posix_shell_script(&root.join("scripts/docs-site")));
        assert!(!is_posix_shell_script(&root.join("Cargo.toml")));
    }

    #[test]
    fn repository_shell_tasks_use_git_bash_on_windows() {
        let root = repository_root();
        let task = Task::new(&root, "./scripts/check_crate_roots.sh");
        let command = task.command().expect("Git Bash command");

        assert_eq!(
            Path::new(command.get_program())
                .file_name()
                .and_then(OsStr::to_str),
            Some("bash.exe")
        );
        assert_eq!(
            command.get_args().next(),
            Some(OsStr::new("./scripts/check_crate_roots.sh"))
        );
    }

    #[test]
    fn path_batch_tasks_use_cmd_on_windows() {
        let scratch = env::temp_dir().join(format!(
            "colossus-xtask-command-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&scratch).expect("create command test directory");
        fs::write(scratch.join("npm.cmd"), "@echo off\r\n").expect("write batch shim");

        let root = repository_root();
        let search_path = env::join_paths([&scratch]).expect("join search path");
        let task = Task::new(&root, "npm").arg("--version");
        let script = windows_batch_script(
            task.program.as_os_str(),
            &task.current_dir,
            Some(search_path.as_os_str()),
        )
        .expect("batch script should resolve");

        assert_eq!(script, scratch.join("npm.cmd"));
        fs::remove_dir_all(scratch).expect("remove command test directory");
    }
}
