use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{Command, Stdio},
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
        let mut command = self.command();
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
        let output = self
            .command()
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
        let output = self
            .command()
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

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .current_dir(&self.current_dir)
            .envs(self.env.iter().map(|(key, value)| (key, value)));
        command
    }

    fn print(&self) {
        eprintln!("+ {}", display_command(&self.program, &self.args));
    }
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
