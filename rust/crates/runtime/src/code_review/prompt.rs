use super::{ReviewContext, ReviewScope};

const DEFAULT_MAX_DIFF_CHARS: usize = 24_000;
const DEFAULT_MAX_RULE_CHARS: usize = 6_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReviewPromptOptions {
    pub max_diff_chars: usize,
    pub max_rule_chars: usize,
}

impl Default for ReviewPromptOptions {
    fn default() -> Self {
        Self { max_diff_chars: DEFAULT_MAX_DIFF_CHARS, max_rule_chars: DEFAULT_MAX_RULE_CHARS }
    }
}

#[must_use]
pub fn build_review_prompt(context: &ReviewContext, options: ReviewPromptOptions) -> String {
    let mut prompt = vec![
        "You are Sego Review Agent.".to_string(),
        "Mode: read-only code review. Do not modify files, run write operations, commit, or change permissions.".to_string(),
        format!("Review scope: {}", context.target.scope.label()),
        String::new(),
        "Task: review the current git diff for correctness, regressions, security risks, data-flow drift, missing tests, and maintainability issues.".to_string(),
        "Findings must be specific and evidence-based. Avoid broad summaries unless there are no findings.".to_string(),
        String::new(),
        "Output contract:".to_string(),
        "- Prefer JSON only. Do not wrap the JSON in prose.".to_string(),
        "- Shape: {\"findings\":[{\"severity\":\"critical|high|medium|low|info\",\"file\":\"path\",\"line\":123,\"title\":\"short title\",\"evidence\":\"specific diff evidence\",\"risk\":\"why it matters\",\"suggestion\":\"specific fix\",\"confidence\":0.0,\"verification_hint\":\"test or command to run\"}]}.".to_string(),
        "- Order findings by severity.".to_string(),
        "- If no issues are found, say exactly: No findings.".to_string(),
        "- Do not claim tests passed unless evidence is provided in the context.".to_string(),
        String::new(),
    ];

    if !context.project_rules.is_empty() {
        prompt.push("Project rules:".to_string());
        prompt
            .push(truncate_for_prompt(&context.project_rules.join("\n\n"), options.max_rule_chars));
        prompt.push(String::new());
    }

    if !context.target.git_status.trim().is_empty() {
        prompt.push("Git status:".to_string());
        prompt.push(fenced("text", context.target.git_status.trim()));
        prompt.push(String::new());
    }

    let diff = diff_for_scope(context);
    if diff.trim().is_empty() {
        prompt.push("Diff: no current changes for this review scope.".to_string());
    } else {
        prompt.push("Diff:".to_string());
        prompt.push(fenced("diff", &truncate_for_prompt(&diff, options.max_diff_chars)));
    }

    prompt.join("\n")
}

fn diff_for_scope(context: &ReviewContext) -> String {
    match &context.target.scope {
        ReviewScope::Workspace | ReviewScope::Path(_) => {
            let mut sections = Vec::new();
            if !context.target.staged_diff.trim().is_empty() {
                sections
                    .push(format!("Staged changes:\n{}", context.target.staged_diff.trim_end()));
            }
            if !context.target.unstaged_diff.trim().is_empty() {
                sections.push(format!(
                    "Unstaged changes:\n{}",
                    context.target.unstaged_diff.trim_end()
                ));
            }
            sections.join("\n\n")
        }
        ReviewScope::Staged => context.target.staged_diff.clone(),
        ReviewScope::Unstaged => context.target.unstaged_diff.clone(),
    }
}

fn fenced(language: &str, body: &str) -> String {
    format!("```{language}\n{body}\n```")
}

fn truncate_for_prompt(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    let truncated = value.chars().take(max_chars).collect::<String>();
    format!("{truncated}\n\n[truncated: original content exceeded {max_chars} characters]")
}

#[cfg(test)]
mod tests {
    use super::{build_review_prompt, ReviewPromptOptions};
    use crate::code_review::{ReviewContext, ReviewScope, ReviewTarget};

    #[test]
    fn prompt_declares_read_only_review_contract() {
        let context = ReviewContext::new(ReviewTarget {
            scope: ReviewScope::Workspace,
            git_status: "## main\n M src/lib.rs".to_string(),
            staged_diff: String::new(),
            unstaged_diff: "diff --git a/src/lib.rs b/src/lib.rs\n".to_string(),
        });

        let prompt = build_review_prompt(&context, ReviewPromptOptions::default());

        assert!(prompt.contains("You are Sego Review Agent."));
        assert!(prompt.contains("Mode: read-only code review."));
        assert!(prompt.contains("Prefer JSON only."));
        assert!(prompt.contains("\"severity\":\"critical|high|medium|low|info\""));
        assert!(prompt.contains("diff --git a/src/lib.rs b/src/lib.rs"));
    }

    #[test]
    fn prompt_handles_empty_diff() {
        let context = ReviewContext::new(ReviewTarget {
            scope: ReviewScope::Staged,
            git_status: "## main".to_string(),
            staged_diff: String::new(),
            unstaged_diff: "diff --git a/src/lib.rs b/src/lib.rs\n".to_string(),
        });

        let prompt = build_review_prompt(&context, ReviewPromptOptions::default());

        assert!(prompt.contains("Review scope: staged"));
        assert!(prompt.contains("Diff: no current changes for this review scope."));
        assert!(!prompt.contains("diff --git a/src/lib.rs b/src/lib.rs"));
    }

    #[test]
    fn prompt_truncates_large_diff() {
        let context = ReviewContext::new(ReviewTarget {
            scope: ReviewScope::Unstaged,
            git_status: String::new(),
            staged_diff: String::new(),
            unstaged_diff: "x".repeat(100),
        });

        let prompt = build_review_prompt(
            &context,
            ReviewPromptOptions { max_diff_chars: 12, max_rule_chars: 12 },
        );

        assert!(prompt.contains("[truncated: original content exceeded 12 characters]"));
    }
}
