use super::SavedComposerDraft;
use std::{collections::VecDeque, fmt};

const MAX_EDIT_FRAMES: usize = 32;
// One maximum-size UTF-8 paste fits, including its visible marker and frame metadata.
const MAX_EDIT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy)]
pub(super) enum EditDirection {
    Undo,
    Redo,
}

impl EditDirection {
    pub(super) fn opposite(self) -> Self {
        match self {
            Self::Undo => Self::Redo,
            Self::Redo => Self::Undo,
        }
    }
}

struct EditFrame {
    draft: SavedComposerDraft,
    bytes: usize,
}

#[derive(Default)]
pub(super) struct ComposerEdits {
    undo: VecDeque<EditFrame>,
    redo: VecDeque<EditFrame>,
}

impl fmt::Debug for ComposerEdits {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ComposerEdits")
            .field("undo_frames", &self.undo.len())
            .field("redo_frames", &self.redo.len())
            .finish()
    }
}

impl ComposerEdits {
    pub(super) fn is_empty(&self) -> bool {
        self.undo.is_empty() && self.redo.is_empty()
    }

    pub(super) fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub(super) fn discard(&mut self, direction: EditDirection) {
        self.stack(direction).clear();
    }

    pub(super) fn pop(&mut self, direction: EditDirection) -> Option<SavedComposerDraft> {
        self.stack(direction).pop_back().map(|frame| frame.draft)
    }

    pub(super) fn push(
        &mut self,
        direction: EditDirection,
        bytes: usize,
        snapshot: impl FnOnce() -> SavedComposerDraft,
    ) {
        if bytes > MAX_EDIT_BYTES {
            // Skipping this frame would leave a gap on this side of the history.
            self.discard(direction);
            return;
        }

        let mut retained_bytes = self.retained_bytes();
        while self.undo.len() + self.redo.len() >= MAX_EDIT_FRAMES
            || retained_bytes > MAX_EDIT_BYTES - bytes
        {
            // Remove the farthest frame, preserving the nearest undo and redo targets.
            let removed = self
                .stack(direction)
                .pop_front()
                .or_else(|| self.stack(direction.opposite()).pop_front())
                .expect("an over-budget history contains at least one frame");
            retained_bytes -= removed.bytes;
        }
        self.stack(direction).push_back(EditFrame {
            draft: snapshot(),
            bytes,
        });
    }

    fn retained_bytes(&self) -> usize {
        self.undo
            .iter()
            .chain(&self.redo)
            .map(|frame| frame.bytes)
            .sum()
    }

    fn stack(&mut self, direction: EditDirection) -> &mut VecDeque<EditFrame> {
        match direction {
            EditDirection::Undo => &mut self.undo,
            EditDirection::Redo => &mut self.redo,
        }
    }
}

#[cfg(test)]
mod tests;
