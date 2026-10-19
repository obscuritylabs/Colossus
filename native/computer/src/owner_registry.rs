//! Admission and revocation are serialized against the registered run control.
use colossus_contracts::ComputerBinding;
use colossus_ports::{ComputerDriverError as Error, RunControl};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

pub(crate) struct OwnerRegistry<T>(Mutex<HashMap<ComputerBinding, Arc<T>>>);
impl<T> OwnerRegistry<T> {
    pub(crate) fn new() -> Self {
        Self(Mutex::new(HashMap::new()))
    }
    pub(crate) fn admit(
        &self,
        binding: &ComputerBinding,
        control: &RunControl,
        create: impl FnOnce() -> Arc<T>,
    ) -> Result<Arc<T>, Error> {
        let mut owners = self.0.lock().map_err(|_| Error::Failed)?;
        // Runtime cancels this exact registered control BEFORE revoke_run. This
        // check under the registry lock closes the cancellation/admission race.
        if control.is_cancelled() {
            return Err(Error::Denied);
        }
        if let Some(owner) = owners.get(binding) {
            return Ok(Arc::clone(owner));
        }
        // Only actually admitted runs consume permanent revocation tombstones.
        if owners.len() >= 4096 {
            return Err(Error::Busy);
        }
        let owner = create();
        owners.insert(binding.clone(), Arc::clone(&owner));
        Ok(owner)
    }
    pub(crate) fn existing(&self, binding: &ComputerBinding) -> Result<Option<Arc<T>>, Error> {
        Ok(self
            .0
            .lock()
            .map_err(|_| Error::Failed)?
            .get(binding)
            .cloned())
    }
    pub(crate) fn revoke(&self, binding: &ComputerBinding, revoke: impl FnOnce(&T)) {
        if let Ok(owners) = self.0.lock()
            && let Some(owner) = owners.get(binding)
        {
            revoke(owner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use colossus_contracts::ComputerScope;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    fn binding(index: usize) -> ComputerBinding {
        ComputerBinding {
            runtime_id: "runtime".into(),
            workspace_id: "workspace".into(),
            application_id: "application".into(),
            scope: ComputerScope::Conversation {
                id: "conversation".into(),
            },
            run_id: format!("run-{index}"),
        }
    }
    #[test]
    fn unused_cancelled_runs_allocate_no_owner_and_do_not_exhaust_admission() {
        let owners = OwnerRegistry::new();
        let allocations = AtomicUsize::new(0);
        for index in 0..5000 {
            let binding = binding(index);
            let control = RunControl::default();
            control.cancel();
            owners.revoke(&binding, |_: &AtomicBool| panic!("unused owner"));
            assert!(owners.existing(&binding).unwrap().is_none()); // close_run lookup
            assert!(matches!(
                owners.admit(&binding, &control, || {
                    allocations.fetch_add(1, Ordering::Relaxed);
                    Arc::new(AtomicBool::new(false))
                }),
                Err(Error::Denied)
            ));
        }
        assert_eq!(allocations.load(Ordering::Relaxed), 0);
        let binding = binding(5001);
        let control = RunControl::default();
        let owner = owners
            .admit(&binding, &control, || {
                allocations.fetch_add(1, Ordering::Relaxed);
                Arc::new(AtomicBool::new(false))
            })
            .unwrap();
        assert_eq!(allocations.load(Ordering::Relaxed), 1);
        control.cancel();
        owners.revoke(&binding, |owner| owner.store(true, Ordering::Release));
        assert!(owner.load(Ordering::Acquire));
        assert!(matches!(
            owners.admit(&binding, &control, || panic!("cancelled binding recreated")),
            Err(Error::Denied)
        ));
        assert!(Arc::ptr_eq(
            &owners.existing(&binding).unwrap().unwrap(),
            &owner
        ));
    }
}
