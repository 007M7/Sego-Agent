use std::path::PathBuf;

use super::ReviewScope;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewTarget {
    pub scope: ReviewScope,
    pub git_status: String,
    pub staged_diff: String,
    pub unstaged_diff: String,
    /// For `FullRepo` scope: pre-collected file tree + key-file contents.
    /// Empty for diff-based scopes.
    pub full_tree: String,
    /// For `FullRepo` scope: the root directory of the audited repository.
    /// Used to place `.sego/reviews/` artifacts into the target repo, not cwd.
    /// `None` for diff-based scopes (falls back to cwd).
    pub workspace_root: Option<PathBuf>,
}

impl ReviewTarget {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match &self.scope {
            ReviewScope::FullRepo(_) => self.full_tree.trim().is_empty(),
            _ => self.staged_diff.trim().is_empty() && self.unstaged_diff.trim().is_empty(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewContext {
    pub target: ReviewTarget,
    pub project_rules: Vec<String>,
}

impl ReviewContext {
    #[must_use]
    pub fn new(target: ReviewTarget) -> Self {
        Self { target, project_rules: Vec::new() }
    }

    #[must_use]
    pub fn with_project_rules(mut self, project_rules: Vec<String>) -> Self {
        self.project_rules = project_rules;
        self
    }
}
