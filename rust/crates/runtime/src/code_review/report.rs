use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{ReviewSeverity, ReviewTarget};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewFinding {
    pub severity: ReviewSeverity,
    pub file: String,
    pub line: Option<u32>,
    pub title: String,
    pub evidence: String,
    pub risk: String,
    pub suggestion: String,
    pub confidence: f32,
    pub verification_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewReport {
    pub findings: Vec<ReviewFinding>,
    pub raw_text: String,
}

impl ReviewReport {
    #[must_use]
    pub fn no_findings(raw_text: impl Into<String>) -> Self {
        Self { findings: Vec::new(), raw_text: raw_text.into() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedReviewArtifact {
    pub id: String,
    pub diff_hash: String,
    pub json_path: PathBuf,
    pub markdown_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ReviewArtifact {
    schema_version: u32,
    id: String,
    created_at_epoch_seconds: u64,
    scope: String,
    diff_hash: String,
    finding_count: usize,
    git_status: String,
    raw_text: String,
}

pub fn persist_review_artifact(
    workspace_root: &Path,
    target: &ReviewTarget,
    report: &ReviewReport,
) -> std::io::Result<PersistedReviewArtifact> {
    let reviews_dir = workspace_root.join(".sego").join("reviews");
    fs::create_dir_all(&reviews_dir)?;

    let diff_hash = review_diff_hash(target);
    let created_at_epoch_seconds = current_epoch_seconds();
    let id = format!("review-{created_at_epoch_seconds}-{}", short_hash(&diff_hash));

    let artifact = ReviewArtifact {
        schema_version: 1,
        id: id.clone(),
        created_at_epoch_seconds,
        scope: target.scope.label().clone(),
        diff_hash: diff_hash.clone(),
        finding_count: report.findings.len(),
        git_status: target.git_status.clone(),
        raw_text: report.raw_text.clone(),
    };

    let json_path = reviews_dir.join(format!("{id}.json"));
    let markdown_path = reviews_dir.join(format!("{id}.md"));

    fs::write(&json_path, serde_json::to_string_pretty(&artifact)? + "\n")?;
    fs::write(&markdown_path, render_review_markdown(&artifact, report))?;

    Ok(PersistedReviewArtifact { id, diff_hash, json_path, markdown_path })
}

#[must_use]
pub fn review_diff_hash(target: &ReviewTarget) -> String {
    let mut hasher = Sha256::new();
    hasher.update(target.scope.label().as_bytes());
    hasher.update(b"\n---staged---\n");
    hasher.update(target.staged_diff.as_bytes());
    hasher.update(b"\n---unstaged---\n");
    hasher.update(target.unstaged_diff.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn render_review_markdown(artifact: &ReviewArtifact, report: &ReviewReport) -> String {
    let mut output = String::new();
    output.push_str("# Sego Review Report\n\n");
    let _ = writeln!(output, "- ID: `{}`", artifact.id);
    let _ = writeln!(output, "- Scope: `{}`", artifact.scope);
    let _ = writeln!(output, "- Diff hash: `{}`", artifact.diff_hash);
    let _ = writeln!(output, "- Findings: `{}`", artifact.finding_count);
    let _ =
        writeln!(output, "- Created at epoch seconds: `{}`\n", artifact.created_at_epoch_seconds);

    if report.raw_text.trim().is_empty() {
        output.push_str("## Review Output\n\nNo review output captured.\n");
    } else {
        output.push_str("## Review Output\n\n");
        output.push_str(report.raw_text.trim());
        output.push('\n');
    }

    if !artifact.git_status.trim().is_empty() {
        output.push_str("\n## Git Status\n\n```text\n");
        output.push_str(artifact.git_status.trim());
        output.push_str("\n```\n");
    }

    output
}

fn current_epoch_seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn short_hash(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
}

#[cfg(test)]
mod tests {
    use super::{persist_review_artifact, review_diff_hash, ReviewReport};
    use crate::code_review::{ReviewScope, ReviewTarget};

    #[test]
    fn diff_hash_changes_with_diff_content() {
        let base = target_with_diff("diff --git a/a b/a\n");
        let changed = target_with_diff("diff --git a/b b/b\n");

        assert_ne!(review_diff_hash(&base), review_diff_hash(&changed));
        assert_eq!(
            review_diff_hash(&base),
            review_diff_hash(&target_with_diff("diff --git a/a b/a\n"))
        );
    }

    #[test]
    fn persists_json_and_markdown_artifacts() {
        let root = temp_path("review-artifact");
        let target = target_with_diff("diff --git a/src/lib.rs b/src/lib.rs\n");
        let report = ReviewReport::no_findings("No findings.");

        let artifact =
            persist_review_artifact(&root, &target, &report).expect("artifact should persist");

        assert!(artifact.json_path.exists());
        assert!(artifact.markdown_path.exists());
        let markdown = std::fs::read_to_string(&artifact.markdown_path).expect("read markdown");
        assert!(markdown.contains("# Sego Review Report"));
        assert!(markdown.contains("No findings."));

        let _ = std::fs::remove_dir_all(root);
    }

    fn target_with_diff(diff: &str) -> ReviewTarget {
        ReviewTarget {
            scope: ReviewScope::Workspace,
            git_status: "## main\n M src/lib.rs".to_string(),
            staged_diff: String::new(),
            unstaged_diff: diff.to_string(),
        }
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("sego-{name}-{unique}"))
    }
}
