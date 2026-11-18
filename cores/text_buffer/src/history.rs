use std::collections::VecDeque;

use crate::buffer::EditDelta;
use crate::rope::TextSnapshot;

/// One edit, kept so that it can be undone and done again.
#[derive(Clone)]
pub(crate) struct Step {
    /// Where the edit began.
    pub(crate) start: usize,
    /// What was there before.
    pub(crate) removed: TextSnapshot,
    /// What the edit put there.
    pub(crate) inserted: TextSnapshot,
}

impl Step {
    /// Bytes of text this step holds (counted as if none of it were shared).
    fn bytes(&self) -> usize {
        self.removed.len() + self.inserted.len()
    }
}

/// The steps that can be undone and redone, under a limit on the text they hold.
///
/// A step holds snapshots of the text it took out and put in, which share their chunks with the
/// buffer, so what a step costs is the edit and not the text around it.
pub(crate) struct History {
    undo: VecDeque<Step>,
    redo: Vec<Step>,
    bytes: usize,
    limit: usize,
}

impl Default for History {
    fn default() -> Self {
        Self { undo: VecDeque::new(), redo: Vec::new(), bytes: 0, limit: 64 * 1024 * 1024 }
    }
}

/// Typing joins the step before it only while the text stays short: a line, not a page.
const CONTINUED_STEP_BYTES: usize = 256;

impl History {
    pub(crate) fn record(&mut self, delta: &EditDelta, continuing: bool) {
        self.redo.clear();
        if continuing
            && let Some(last) = self.undo.back_mut()
            && delta.removed.is_empty()
            && last.start + last.inserted.len() == delta.range.start
            // A line feed ends a step: what is typed after it is another.
            && last.inserted.byte_at(last.inserted.len().saturating_sub(1)) != Some(b'\n')
            && last.inserted.len() + delta.inserted.len() <= CONTINUED_STEP_BYTES
            && !delta.inserted.to_string().contains('\n')
        {
            self.bytes -= last.bytes();
            let mut inserted = last.inserted.rope.clone();
            inserted.append(delta.inserted.rope.clone());
            last.inserted = TextSnapshot { rope: inserted };
            self.bytes += last.bytes();
            return;
        }
        let step = Step { start: delta.range.start, removed: delta.removed.clone(), inserted: delta.inserted.clone() };
        self.bytes += step.bytes();
        self.undo.push_back(step);
        self.trim();
    }

    fn trim(&mut self) {
        // The latest step stays however large it is: an undo of the paste just made must work.
        while self.bytes > self.limit && self.undo.len() > 1 {
            let oldest = self.undo.pop_front().expect("there is an oldest step");
            self.bytes -= oldest.bytes();
        }
    }

    pub(crate) fn pop_undo(&mut self) -> Option<Step> {
        let step = self.undo.pop_back()?;
        self.bytes -= step.bytes();
        Some(step)
    }

    pub(crate) fn pop_redo(&mut self) -> Option<Step> {
        let step = self.redo.pop()?;
        Some(step)
    }

    pub(crate) fn push_redo(&mut self, step: Step) {
        self.redo.push(step);
    }

    pub(crate) fn push_undo(&mut self, step: Step) {
        self.bytes += step.bytes();
        self.undo.push_back(step);
        self.trim();
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub(crate) fn set_limit(&mut self, bytes: usize) {
        self.limit = bytes;
        self.trim();
    }

    pub(crate) fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.bytes = 0;
    }
}
