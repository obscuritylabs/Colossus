//! Bounded remembered-window lifetime seals, including the pre-Open phase.
use colossus_ports::ComputerDriverError as Error;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
const CAPACITY: usize = 512;
struct Slot {
    handle: AtomicU64,
    generation: AtomicU64,
    destroyed: AtomicBool,
}
pub(crate) struct Table {
    slots: [Slot; CAPACITY],
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Stamp {
    index: usize,
    generation: u64,
}
pub(crate) struct Admitted<T> {
    pub value: T,
    pub stamp: Stamp,
}
impl Table {
    fn new() -> Self {
        Self {
            slots: std::array::from_fn(|_| Slot {
                handle: AtomicU64::new(0),
                generation: AtomicU64::new(0),
                destroyed: AtomicBool::new(false),
            }),
        }
    }
    // Called only by the pumping broker thread. The callback also runs on that
    // thread; other threads may only inspect immutable, generation-fenced seals.
    fn track(&self, handle: u64) -> Result<(Stamp, bool), Error> {
        if handle == 0 {
            return Err(Error::InvalidInput);
        }
        for (index, slot) in self.slots.iter().enumerate() {
            if slot.handle.load(Ordering::Acquire) == handle
                && !slot.destroyed.load(Ordering::Acquire)
            {
                return Ok((
                    Stamp {
                        index,
                        generation: slot.generation.load(Ordering::Acquire),
                    },
                    false,
                ));
            }
        }
        for (index, slot) in self.slots.iter().enumerate() {
            if slot.handle.load(Ordering::Acquire) != 0 && !slot.destroyed.load(Ordering::Acquire) {
                continue;
            }
            let Some(generation) = slot.generation.load(Ordering::Acquire).checked_add(1) else {
                continue;
            };
            // Invalidate old readers before publishing this slot's new lifetime.
            slot.handle.store(0, Ordering::Release);
            slot.generation.store(generation, Ordering::Release);
            slot.destroyed.store(false, Ordering::Release);
            slot.handle.store(handle, Ordering::Release);
            return Ok((Stamp { index, generation }, true));
        }
        Err(Error::Busy)
    }
    fn admit<T>(
        &self,
        handle: u64,
        inspect: impl FnOnce() -> Result<T, Error>,
        drain: impl FnOnce(),
    ) -> Result<Admitted<T>, Error> {
        // The seal MUST precede every query producing the admitted facts. A
        // queued destroy during inspection must invalidate those exact facts,
        // even when Windows recycles the handle in the same process and bounds.
        let (stamp, provisional) = self.track(handle)?;
        let value = inspect();
        drain();
        let result = value.and_then(|value| {
            if self.alive(stamp, handle) {
                Ok(Admitted { value, stamp })
            } else {
                Err(Error::Stale)
            }
        });
        if result.is_err() && provisional {
            self.release(stamp, handle);
        }
        result
    }
    fn release(&self, stamp: Stamp, handle: u64) {
        if let Some(slot) = self.slots.get(stamp.index)
            && slot.generation.load(Ordering::Acquire) == stamp.generation
            && slot.handle.load(Ordering::Acquire) == handle
        {
            slot.handle.store(0, Ordering::Release);
        }
    }
    fn alive(&self, stamp: Stamp, handle: u64) -> bool {
        self.alive_during(stamp, handle, || {})
    }
    fn alive_during(&self, stamp: Stamp, handle: u64, between_reads: impl FnOnce()) -> bool {
        self.slots.get(stamp.index).is_some_and(|slot| {
            let same_generation = slot.handle.load(Ordering::Acquire) == handle
                && slot.generation.load(Ordering::Acquire) == stamp.generation;
            // Deterministic tests inject recycling at this read boundary. The
            // production empty closure is optimized away.
            between_reads();
            same_generation && !slot.destroyed.load(Ordering::Acquire)
                // A concurrent fence reader must not combine the old generation
                // with destroyed=false published by a recycled slot.
                && slot.generation.load(Ordering::Acquire) == stamp.generation
                && slot.handle.load(Ordering::Acquire) == handle
        })
    }
    fn destroyed(&self, handle: u64) {
        if handle == 0 {
            return;
        }
        for slot in &self.slots {
            if slot.handle.load(Ordering::Acquire) == handle {
                slot.destroyed.store(true, Ordering::Release);
            }
        }
    }
}
pub(crate) struct Fence {
    table: Arc<Table>,
    root: (Stamp, u64),
    target: (Stamp, u64),
}
impl Fence {
    pub fn valid(&self) -> bool {
        self.table.alive(self.root.0, self.root.1) && self.table.alive(self.target.0, self.target.1)
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{ptr::null_mut, sync::atomic::AtomicPtr};
    use windows_sys::Win32::{
        Foundation::HWND,
        UI::{
            Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
            WindowsAndMessaging::{EVENT_OBJECT_DESTROY, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT},
        },
    };
    static TABLE: AtomicPtr<Table> = AtomicPtr::new(null_mut());
    pub(crate) struct Tracker {
        table: Arc<Table>,
        hook: HWINEVENTHOOK,
    }
    impl Tracker {
        pub fn new() -> Result<Self, Error> {
            let table = Arc::new(Table::new());
            TABLE
                .compare_exchange(
                    null_mut(),
                    Arc::as_ptr(&table).cast_mut(),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .map_err(|_| Error::Busy)?;
            // SAFETY: out-of-context callbacks execute on this pumping broker
            // thread. The retained Arc outlives the hook and global pointer.
            let hook = unsafe {
                SetWinEventHook(
                    EVENT_OBJECT_DESTROY,
                    EVENT_OBJECT_DESTROY,
                    null_mut(),
                    Some(destroyed),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                )
            };
            if hook.is_null() {
                TABLE.store(null_mut(), Ordering::Release);
                return Err(Error::Unavailable);
            }
            Ok(Self { table, hook })
        }
        pub fn admit<T>(
            &self,
            handle: u64,
            inspect: impl FnOnce() -> Result<T, Error>,
            drain: impl FnOnce(),
        ) -> Result<Admitted<T>, Error> {
            self.table.admit(handle, inspect, drain)
        }
        pub fn alive(&self, stamp: Stamp, handle: u64) -> bool {
            self.table.alive(stamp, handle)
        }
        pub fn fence(&self, root: (Stamp, u64), target: (Stamp, u64)) -> Fence {
            Fence {
                table: Arc::clone(&self.table),
                root,
                target,
            }
        }
    }
    impl Drop for Tracker {
        fn drop(&mut self) {
            TABLE.store(null_mut(), Ordering::Release); // SAFETY: own this hook, and clear the borrowed pointer before releasing its Arc.
            unsafe { UnhookWinEvent(self.hook) };
        }
    }
    unsafe extern "system" fn destroyed(
        _: HWINEVENTHOOK,
        _: u32,
        hwnd: HWND,
        object: i32,
        child: i32,
        _: u32,
        _: u32,
    ) {
        if object != OBJID_WINDOW || child != 0 {
            return;
        }
        let pointer = TABLE.load(Ordering::Acquire);
        // SAFETY: callback belongs to the owning broker thread and its live hook;
        // teardown clears the pointer before releasing the retained table.
        if let Some(table) = unsafe { pointer.as_ref() } {
            table.destroyed(hwnd as usize as u64);
        }
    }
}
#[cfg(windows)]
pub(crate) use native::Tracker;

#[cfg(test)]
#[path = "lifetime_tests.rs"]
mod tests;
