use super::ReviewScope;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewTarget {
    pub scope: ReviewScope,
    pub git_status: String,
    pub staged_diff: String,
    pub unstaged_diff: String,
}

impl ReviewTarget {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.staged_diff.trim().is_empty() && self.unstaged_diff.trim().is_empty()
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
