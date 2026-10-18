//! Audited Windows FFI boundary. No Desktop UI thread enters this module.
mod broker;
mod capture;
pub(crate) mod identity;
mod indicator;
mod input;
mod lease;
mod read_worker;
mod semantics;

use colossus_ports::ComputerDriverError as Error;

pub(crate) fn entry() -> Result<(), Error> {
    broker::entry()
}

pub(crate) fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
pub(crate) fn failed<T>(_: T) -> Error {
    Error::Failed
}
