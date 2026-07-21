use colossus_sdk::Colossus;
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedSemaphorePermit, RwLock, Semaphore};

const MAX_NATIVE_WATCHES: usize = 8;
const MAX_NATIVE_UNARY_CALLS: usize = 16;

/// Native-only authenticated client state shared by narrow Tauri commands.
pub(crate) struct AppState {
    client: RwLock<Option<Colossus>>,
    connect_guard: Mutex<()>,
    watch_slots: Arc<Semaphore>,
    unary_slots: Arc<Semaphore>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            client: RwLock::new(None),
            connect_guard: Mutex::new(()),
            watch_slots: Arc::new(Semaphore::new(MAX_NATIVE_WATCHES)),
            unary_slots: Arc::new(Semaphore::new(MAX_NATIVE_UNARY_CALLS)),
        }
    }
}

impl AppState {
    pub(crate) async fn client(&self) -> Option<Colossus> {
        self.client.read().await.clone()
    }

    pub(crate) async fn replace_client(&self, client: Colossus) -> Option<Colossus> {
        self.client.write().await.replace(client)
    }

    pub(crate) fn try_connect_guard(&self) -> Option<tokio::sync::MutexGuard<'_, ()>> {
        self.connect_guard.try_lock().ok()
    }

    pub(crate) fn try_watch_slot(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.watch_slots).try_acquire_owned().ok()
    }

    pub(crate) fn try_unary_slot(&self) -> Option<OwnedSemaphorePermit> {
        Arc::clone(&self.unary_slots).try_acquire_owned().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_watch_admission_is_bounded_and_released_on_drop() {
        let state = AppState::default();
        let permits: Vec<_> = (0..MAX_NATIVE_WATCHES)
            .map(|_| state.try_watch_slot().expect("watch slot"))
            .collect();
        assert!(state.try_watch_slot().is_none());
        drop(permits);
        assert!(state.try_watch_slot().is_some());
    }

    #[test]
    fn concurrent_connect_attempts_fail_fast() {
        let state = AppState::default();
        let guard = state.try_connect_guard().expect("first connect guard");
        assert!(state.try_connect_guard().is_none());
        drop(guard);
        assert!(state.try_connect_guard().is_some());
    }

    #[test]
    fn native_unary_admission_is_bounded_and_released_on_drop() {
        let state = AppState::default();
        let permits: Vec<_> = (0..MAX_NATIVE_UNARY_CALLS)
            .map(|_| state.try_unary_slot().expect("unary slot"))
            .collect();
        assert!(state.try_unary_slot().is_none());
        drop(permits);
        assert!(state.try_unary_slot().is_some());
    }
}
