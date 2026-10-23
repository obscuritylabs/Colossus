use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[derive(Debug)]
pub(super) struct Timings {
    started: Instant,
    entries: RefCell<Vec<Entry>>,
}

#[derive(Debug)]
struct Entry {
    command: String,
    directory: PathBuf,
    elapsed: Duration,
    outcome: String,
}

impl Timings {
    pub(super) fn new() -> Self {
        Self {
            started: Instant::now(),
            entries: RefCell::new(Vec::new()),
        }
    }

    pub(super) fn start(&self, command: String, directory: &Path) -> Timer<'_> {
        Timer {
            timings: self,
            command,
            directory: directory.to_path_buf(),
            started: Instant::now(),
            outcome: "could not start".to_owned(),
        }
    }

    pub(super) fn report(&self) {
        let entries = self.entries.borrow();
        eprintln!(
            "\nxtask elapsed: {:.2}s across {} commands (excludes Cargo bootstrap)",
            self.started.elapsed().as_secs_f64(),
            entries.len(),
        );
        let mut ordered: Vec<_> = entries.iter().collect();
        ordered.sort_by_key(|entry| std::cmp::Reverse(entry.elapsed));
        for entry in ordered.into_iter().take(10) {
            eprintln!(
                "  {:>8.2}s [{}] {} (cwd: {})",
                entry.elapsed.as_secs_f64(),
                entry.outcome,
                entry.command,
                entry.directory.display(),
            );
        }
    }
}

pub(super) struct Timer<'a> {
    timings: &'a Timings,
    command: String,
    directory: PathBuf,
    started: Instant,
    outcome: String,
}

impl Timer<'_> {
    pub(super) fn outcome(&mut self, outcome: impl Into<String>) {
        self.outcome = outcome.into();
    }
}

impl Drop for Timer<'_> {
    fn drop(&mut self) {
        let elapsed = self.started.elapsed();
        eprintln!(
            "  [xtask {:.2}s, {}] {} (cwd: {})",
            elapsed.as_secs_f64(),
            self.outcome,
            self.command,
            self.directory.display(),
        );
        self.timings.entries.borrow_mut().push(Entry {
            command: self.command.clone(),
            directory: self.directory.clone(),
            elapsed,
            outcome: self.outcome.clone(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_start_failures_and_actual_probe_outcomes_separately() {
        let timings = Timings::new();
        {
            let _timer = timings.start("missing-program".to_owned(), Path::new("."));
        }
        {
            let mut timer = timings.start("git probe".to_owned(), Path::new("nested"));
            timer.outcome("exit code: 1");
        }
        let entries = timings.entries.borrow();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].outcome, "could not start");
        assert_eq!(entries[1].outcome, "exit code: 1");
        assert_eq!(entries[1].directory, Path::new("nested"));
    }
}
