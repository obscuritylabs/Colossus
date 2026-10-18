//! Closed native desktop contracts. Opaque identities never encode operating-system handles.

mod identity;
mod input;
mod types;

pub use identity::*;
pub use input::*;
pub use types::*;

#[cfg(test)]
mod tests;
