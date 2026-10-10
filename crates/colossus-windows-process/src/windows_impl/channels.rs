use super::{OwnedHandle, last_error, pipe};
use crate::{PrivateProcessChannel, WindowsProcessError};
use windows_sys::Win32::Foundation::{HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation};

pub(super) struct ChannelPipes {
    child_read: OwnedHandle,
    child_write: OwnedHandle,
    parent_read: OwnedHandle,
    parent_write: OwnedHandle,
}

impl ChannelPipes {
    pub(super) fn new() -> Result<Self, WindowsProcessError> {
        let (child_read, parent_write) = pipe("CreatePipe(private input)")?;
        let (parent_read, child_write) = pipe("CreatePipe(private output)")?;
        for handle in [&parent_read, &parent_write] {
            // SAFETY: owned live pipe handle; this only clears its inheritance flag.
            if unsafe { SetHandleInformation(handle.raw(), HANDLE_FLAG_INHERIT, 0) } == 0 {
                return Err(last_error("SetHandleInformation(private parent)"));
            }
        }
        Ok(Self {
            child_read,
            child_write,
            parent_read,
            parent_write,
        })
    }

    pub(super) fn child_handles(&self) -> [HANDLE; 2] {
        [self.child_read.raw(), self.child_write.raw()]
    }

    pub(super) fn into_parent(self) -> PrivateProcessChannel {
        let Self {
            child_read,
            child_write,
            parent_read,
            parent_write,
        } = self;
        drop(child_read);
        drop(child_write);
        PrivateProcessChannel {
            reader: parent_read.into_file(),
            writer: parent_write.into_file(),
        }
    }
}
