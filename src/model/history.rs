//! Undo and redo as whole-layout snapshots.

use super::layout::Layout;

/// Undo/redo stacks of layout clones, capped at [`History::CAP`] undo steps.
#[derive(Clone, Debug, Default)]
pub struct History {
    undo: Vec<Layout>,
    redo: Vec<Layout>,
}

impl History {
    pub const CAP: usize = 200;

    /// Records `before` as an undo step if the edit changed anything, and clears the redo
    /// stack. A failed or no-op edit adds no step. Returns whether a step was added.
    pub fn record(&mut self, before: Layout, after: &Layout) -> bool {
        if before == *after {
            return false;
        }
        self.undo.push(before);
        if self.undo.len() > Self::CAP {
            self.undo.remove(0);
        }
        self.redo.clear();
        true
    }

    /// Steps back; returns false when there is nothing to undo.
    pub fn undo(&mut self, current: &mut Layout) -> bool {
        let Some(previous) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(current, previous));
        true
    }

    /// Steps forward again; returns false when there is nothing to redo.
    pub fn redo(&mut self, current: &mut Layout) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(current, next));
        true
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }
}
