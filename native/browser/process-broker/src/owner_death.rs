//! Portable categorical lifetime evidence; it holds no native rights.

/// First native owner whose lifetime lease was lost.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum OwnerDeath {
    /// The authenticated Core/sidecar producer exited or closed its lease.
    Core = 1,
    /// The authenticated Desktop parent exited or closed its lease.
    Desktop = 2,
    /// The independent signed keeper peer exited or closed its lease.
    Keeper = 3,
}
