use std::collections::BTreeSet;

/// Only identical suites shared by more than one component belong here.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum Stage {
    Computer,
}

/// Successful stages from one invocation; no results survive the invocation.
#[derive(Default)]
pub(super) struct CompletedStages {
    completed: BTreeSet<Stage>,
}

impl CompletedStages {
    pub(super) fn run(
        &mut self,
        stage: Stage,
        check: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        if self.completed.contains(&stage) {
            eprintln!("Reusing completed {stage:?} checks from this invocation.");
            return Ok(());
        }
        check()?;
        self.completed.insert(stage);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{CompletedStages, Stage};

    #[test]
    fn successful_shared_stage_runs_once_per_invocation() {
        let calls = Cell::new(0);
        let check = || {
            calls.set(calls.get() + 1);
            Ok(())
        };
        let mut completed = CompletedStages::default();
        completed.run(Stage::Computer, check).expect("first check");
        completed.run(Stage::Computer, check).expect("shared check");
        assert_eq!(calls.get(), 1);

        CompletedStages::default()
            .run(Stage::Computer, check)
            .expect("new invocation check");
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn failed_stage_is_not_recorded_as_completed() {
        let mut completed = CompletedStages::default();
        let failure = completed.run(Stage::Computer, || Err("failed check".to_owned()));
        assert_eq!(failure, Err("failed check".to_owned()));

        let retried = Cell::new(false);
        completed
            .run(Stage::Computer, || {
                retried.set(true);
                Ok(())
            })
            .expect("retry check");
        assert!(retried.get());
    }
}
