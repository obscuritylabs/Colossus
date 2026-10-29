use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Rejects permission changes during foreground work while draining presentation writes.
#[derive(Default)]
pub(super) struct HostOperationGate {
    foreground: RwLock<()>,
    presentation: RwLock<()>,
}

pub(super) enum HostOperationGuard<'a> {
    Foreground {
        _guard: RwLockReadGuard<'a, ()>,
    },
    Permissions {
        _foreground: RwLockWriteGuard<'a, ()>,
        _presentation: RwLockWriteGuard<'a, ()>,
    },
}

impl HostOperationGate {
    pub(super) async fn enter(
        &self,
        permission_change: bool,
    ) -> Result<HostOperationGuard<'_>, String> {
        if permission_change {
            let foreground = self.foreground.try_write().map_err(|_| {
                "Wait for active agent and command operations before changing permissions."
                    .to_owned()
            })?;
            // Reserve foreground admission before draining writes so no run can start
            // with the old mode while a permission change waits for persistence.
            // Both reservations are released if this future is cancelled.
            let presentation = self.presentation.write().await;
            Ok(HostOperationGuard::Permissions {
                _foreground: foreground,
                _presentation: presentation,
            })
        } else {
            Ok(HostOperationGuard::Foreground {
                _guard: self.foreground.read().await,
            })
        }
    }

    pub(super) async fn enter_presentation(&self) -> RwLockReadGuard<'_, ()> {
        self.presentation.read().await
    }
}

impl HostOperationGuard<'_> {
    pub(super) fn apply_permission_change(
        &self,
        control: &colossus_ports::RunControl,
        change: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        if control.is_cancelled() {
            return Err("Permission change cancelled".into());
        }
        change()
    }
}
