use crate::{Frame, Lease, PresentationError};
use std::collections::VecDeque;

/// Two bounded latest frames; slow presenters drop superseded page images.
pub struct LatestFrames {
    lease: Lease,
    sequence: u64,
    frames: VecDeque<Frame>,
}
impl LatestFrames {
    /// Create an empty two-slot queue for the active lease.
    pub fn new(lease: Lease) -> Result<Self, PresentationError> {
        lease.validate()?;
        Ok(Self {
            lease,
            sequence: 0,
            frames: VecDeque::with_capacity(2),
        })
    }
    /// Replace the oldest frame after exact ownership and sequence validation.
    pub fn push(&mut self, frame: Frame) -> Result<(), PresentationError> {
        frame.validate()?;
        if frame.lease != self.lease || frame.sequence <= self.sequence {
            return Err(PresentationError::Stale);
        }
        self.sequence = frame.sequence;
        if self.frames.len() == 2 {
            self.frames.pop_front();
        }
        self.frames.push_back(frame);
        Ok(())
    }
    /// Consume the newest frame and discard obsolete images.
    pub fn take_latest(&mut self) -> Option<Frame> {
        let frame = self.frames.pop_back();
        self.frames.clear();
        frame
    }
    /// Replace trusted native ownership and discard every old frame.
    pub fn replace(&mut self, lease: Lease) -> Result<(), PresentationError> {
        lease.validate()?;
        if lease.tab == self.lease.tab
            && lease.session_generation == self.lease.session_generation
            && (lease.control_generation < self.lease.control_generation
                || lease.viewport_generation <= self.lease.viewport_generation)
        {
            return Err(PresentationError::Stale);
        }
        self.lease = lease;
        self.sequence = 0;
        self.frames.clear();
        Ok(())
    }
    /// Discard pixels immediately on visibility or focus revocation.
    pub fn clear(&mut self) {
        self.frames.clear();
    }
    /// Total bounded pixel memory awaiting presentation.
    pub fn retained_bytes(&self) -> usize {
        self.frames.iter().map(|frame| frame.pixels.len()).sum()
    }
}
