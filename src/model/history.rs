//! Snapshot-based undo/redo for project arrangement edits.

use std::collections::VecDeque;

use super::Project;

pub const DEFAULT_UNDO_LIMIT: usize = 50;
pub const MIN_UNDO_LIMIT: usize = 1;
pub const MAX_UNDO_LIMIT: usize = 200;

/// Project-snapshot undo/redo stacks with optional open drag transaction.
#[derive(Debug, Clone)]
pub struct EditHistory {
    undo: VecDeque<Project>,
    redo: VecDeque<Project>,
    limit: usize,
    /// Pre-gesture snapshot; committed on [`Self::commit`] if the project changed.
    open: Option<Project>,
}

impl Default for EditHistory {
    fn default() -> Self {
        Self::new(DEFAULT_UNDO_LIMIT)
    }
}

impl EditHistory {
    pub fn new(limit: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: VecDeque::new(),
            limit: clamp_undo_limit(limit),
            open: None,
        }
    }

    #[allow(dead_code)] // useful for Settings / status UI
    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Record the project state before a discrete edit. Clears redo.
    pub fn push_before(&mut self, before: Project) {
        self.open = None;
        self.redo.clear();
        self.undo.push_back(before);
        self.trim_undo();
    }

    /// Start a coalesced gesture (move/resize / Shift+duplicate+drag).
    /// Call before any Shift-duplicate so one undo undoes the whole gesture.
    pub fn begin(&mut self, project: &Project) {
        self.open = Some(project.clone());
    }

    /// Finish a gesture: push the open snapshot if the project changed.
    pub fn commit(&mut self, project: &Project) {
        let Some(before) = self.open.take() else {
            return;
        };
        if &before != project {
            self.redo.clear();
            self.undo.push_back(before);
            self.trim_undo();
        }
    }

    /// Drop an open gesture without recording history.
    #[allow(dead_code)] // available when a gesture should cancel without commit
    pub fn discard(&mut self) {
        self.open = None;
    }

    pub fn undo(&mut self, project: &mut Project) -> bool {
        self.open = None;
        let Some(previous) = self.undo.pop_back() else {
            return false;
        };
        self.redo.push_back(std::mem::replace(project, previous));
        self.trim_redo();
        true
    }

    pub fn redo(&mut self, project: &mut Project) -> bool {
        self.open = None;
        let Some(next) = self.redo.pop_back() else {
            return false;
        };
        self.undo.push_back(std::mem::replace(project, next));
        self.trim_undo();
        true
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.open = None;
    }

    pub fn set_limit(&mut self, limit: usize) {
        self.limit = clamp_undo_limit(limit);
        self.trim_undo();
        self.trim_redo();
    }

    fn trim_undo(&mut self) {
        while self.undo.len() > self.limit {
            self.undo.pop_front();
        }
    }

    fn trim_redo(&mut self) {
        while self.redo.len() > self.limit {
            self.redo.pop_front();
        }
    }
}

pub fn clamp_undo_limit(limit: usize) -> usize {
    limit.clamp(MIN_UNDO_LIMIT, MAX_UNDO_LIMIT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::instrument::TrackInstrument;

    fn tiny_project(n: u64) -> Project {
        let mut project = Project::default();
        // Distinguish snapshots via next ids / clip count.
        for _ in 0..n {
            let track_id = project.tracks[0].id;
            project.add_clip_to_track(track_id, n as f32, 1.0);
        }
        let _ = TrackInstrument::BuiltInPiano;
        project
    }

    #[test]
    fn push_undo_redo_roundtrip() {
        let mut history = EditHistory::new(10);
        let mut project = tiny_project(0);
        let before = project.clone();
        project = tiny_project(1);
        history.push_before(before.clone());
        assert!(history.can_undo());
        assert!(history.undo(&mut project));
        assert_eq!(project.tracks[0].clips.len(), before.tracks[0].clips.len());
        assert!(history.can_redo());
        assert!(history.redo(&mut project));
        assert_eq!(project.tracks[0].clips.len(), tiny_project(1).tracks[0].clips.len());
    }

    #[test]
    fn push_clears_redo() {
        let mut history = EditHistory::new(10);
        let mut project = tiny_project(0);
        history.push_before(project.clone());
        project = tiny_project(1);
        history.undo(&mut project);
        history.push_before(project.clone());
        let _next = tiny_project(2);
        assert!(!history.can_redo());
    }

    #[test]
    fn limit_trims_oldest() {
        let mut history = EditHistory::new(2);
        let mut project = tiny_project(0);
        history.push_before(project.clone());
        project = tiny_project(1);
        history.push_before(project.clone());
        project = tiny_project(2);
        history.push_before(project.clone());
        project = tiny_project(3);
        // Only two undos available.
        assert!(history.undo(&mut project));
        assert!(history.undo(&mut project));
        assert!(!history.undo(&mut project));
    }

    #[test]
    fn begin_commit_coalesces() {
        let mut history = EditHistory::new(10);
        let mut project = tiny_project(0);
        history.begin(&project);
        project = tiny_project(1);
        history.commit(&project);
        assert!(history.can_undo());
        history.begin(&project);
        history.commit(&project); // no change
        // Still only one undo step.
        assert!(history.undo(&mut project));
        assert!(!history.undo(&mut project));
    }

    #[test]
    fn set_limit_trims() {
        let mut history = EditHistory::new(5);
        let mut project = tiny_project(0);
        for i in 0..5 {
            history.push_before(project.clone());
            project = tiny_project(i + 1);
        }
        history.set_limit(2);
        assert!(history.undo(&mut project));
        assert!(history.undo(&mut project));
        assert!(!history.undo(&mut project));
    }
}
