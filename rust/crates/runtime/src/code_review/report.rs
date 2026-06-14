use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{ReviewSeverity, ReviewTarget};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewFinding {
    #[serde(default)]
    pub id: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewParseStatus {
    Structured,
    FallbackRawText,
}

impl ReviewParseStatus {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Structured => "structured",
            Self::FallbackRawText => "fallback_raw_text",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewReport {
    pub findings: Vec<ReviewFinding>,
    pub raw_text: String,
    pub parse_status: ReviewParseStatus,
}

impl ReviewReport {
    #[must_use]
    pub fn no_findings(raw_text: impl Into<String>) -> Self {
        Self {
            findings: Vec::new(),
            raw_text: raw_text.into(),
            parse_status: ReviewParseStatus::FallbackRawText,
        }
    }

    #[must_use]
    pub fn from_model_output(raw_text: impl Into<String>) -> Self {
        let raw_text = raw_text.into();
        let trimmed = raw_text.trim();
        if trimmed.eq_ignore_ascii_case("No findings.")
            || trimmed.eq_ignore_ascii_case("No findings")
        {
            return Self {
                findings: Vec::new(),
                raw_text,
                parse_status: ReviewParseStatus::Structured,
            };
        }

        let Some(json_candidate) = extract_json_candidate(trimmed) else {
            return Self::no_findings(raw_text);
        };

        let parsed = serde_json::from_str::<ReviewReportWire>(json_candidate)
            .map(|wire| wire.findings)
            .or_else(|_| serde_json::from_str::<Vec<ReviewFinding>>(json_candidate));

        match parsed {
            Ok(findings) => Self {
                findings: assign_finding_ids(findings),
                raw_text,
                parse_status: ReviewParseStatus::Structured,
            },
            Err(_) => Self::no_findings(raw_text),
        }
    }

    #[must_use]
    pub fn highest_severity(&self) -> Option<ReviewSeverity> {
        self.findings.iter().map(|finding| finding.severity).min()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedReviewArtifact {
    pub id: String,
    pub diff_hash: String,
    pub json_path: PathBuf,
    pub markdown_path: PathBuf,
    pub index_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ReviewArtifact {
    schema_version: u32,
    id: String,
    created_at_epoch_seconds: u64,
    scope: String,
    diff_hash: String,
    finding_count: usize,
    highest_severity: Option<ReviewSeverity>,
    parse_status: ReviewParseStatus,
    git_status: String,
    findings: Vec<ReviewFinding>,
    raw_text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ReviewIndexEntry {
    id: String,
    created_at_epoch_seconds: u64,
    scope: String,
    diff_hash: String,
    finding_count: usize,
    highest_severity: Option<ReviewSeverity>,
    parse_status: ReviewParseStatus,
    json_path: String,
    markdown_path: String,
}

#[derive(Debug, Deserialize)]
struct ReviewReportWire {
    findings: Vec<ReviewFinding>,
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
        highest_severity: report.highest_severity(),
        parse_status: report.parse_status,
        git_status: target.git_status.clone(),
        findings: report.findings.clone(),
        raw_text: report.raw_text.clone(),
    };

    let json_path = reviews_dir.join(format!("{id}.json"));
    let markdown_path = reviews_dir.join(format!("{id}.md"));
    let index_path = reviews_dir.join("index.jsonl");

    fs::write(&json_path, serde_json::to_string_pretty(&artifact)? + "\n")?;
    fs::write(&markdown_path, render_review_markdown(&artifact, report))?;
    append_review_index(&index_path, &artifact, &json_path, &markdown_path)?;

    Ok(PersistedReviewArtifact { id, diff_hash, json_path, markdown_path, index_path })
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
    let _ = writeln!(output, "- Parse status: `{}`", artifact.parse_status.label());
    if let Some(severity) = artifact.highest_severity {
        let _ = writeln!(output, "- Highest severity: `{}`", severity.label());
    }
    let _ =
        writeln!(output, "- Created at epoch seconds: `{}`\n", artifact.created_at_epoch_seconds);

    if report.findings.is_empty() {
        output.push_str("## Findings\n\nNo structured findings.\n");
    } else {
        output.push_str("## Findings\n\n");
        output.push_str("| Severity | File | Line | Title | Confidence |\n");
        output.push_str("|---|---|---:|---|---:|\n");
        for finding in &report.findings {
            let line = finding.line.map_or_else(|| "-".to_string(), |line| line.to_string());
            let _ = writeln!(
                output,
                "| {} | `{}` | {} | {} | {:.2} |",
                finding.severity.label(),
                finding.file,
                line,
                table_escape(&finding.title),
                finding.confidence
            );
        }
        for finding in &report.findings {
            let _ = writeln!(output, "\n### {}", finding.title);
            let _ = writeln!(output, "\n- ID: `{}`", finding.id);
            let _ = writeln!(output, "- Severity: `{}`", finding.severity.label());
            let _ = writeln!(output, "- File: `{}`", finding.file);
            if let Some(line) = finding.line {
                let _ = writeln!(output, "- Line: `{line}`");
            }
            let _ = writeln!(output, "- Evidence: {}", finding.evidence.trim());
            let _ = writeln!(output, "- Risk: {}", finding.risk.trim());
            let _ = writeln!(output, "- Suggestion: {}", finding.suggestion.trim());
            if let Some(hint) = &finding.verification_hint {
                let _ = writeln!(output, "- Verification hint: {}", hint.trim());
            }
        }
        output.push('\n');
    }

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

fn append_review_index(
    index_path: &Path,
    artifact: &ReviewArtifact,
    json_path: &Path,
    markdown_path: &Path,
) -> std::io::Result<()> {
    let entry = ReviewIndexEntry {
        id: artifact.id.clone(),
        created_at_epoch_seconds: artifact.created_at_epoch_seconds,
        scope: artifact.scope.clone(),
        diff_hash: artifact.diff_hash.clone(),
        finding_count: artifact.finding_count,
        highest_severity: artifact.highest_severity,
        parse_status: artifact.parse_status,
        json_path: path_for_index(json_path),
        markdown_path: path_for_index(markdown_path),
    };
    let mut line = serde_json::to_string(&entry)?;
    line.push('\n');
    fs::OpenOptions::new().create(true).append(true).open(index_path)?.write_all(line.as_bytes())
}

fn assign_finding_ids(findings: Vec<ReviewFinding>) -> Vec<ReviewFinding> {
    findings
        .into_iter()
        .map(|mut finding| {
            if finding.id.trim().is_empty() {
                finding.id = stable_finding_id(&finding);
            }
            finding
        })
        .collect()
}

fn stable_finding_id(finding: &ReviewFinding) -> String {
    let mut hasher = Sha256::new();
    hasher.update(finding.severity.label().as_bytes());
    hasher.update(b"\n");
    hasher.update(finding.file.as_bytes());
    hasher.update(b"\n");
    hasher.update(finding.line.map_or_else(String::new, |line| line.to_string()).as_bytes());
    hasher.update(b"\n");
    hasher.update(finding.title.as_bytes());
    hasher.update(b"\n");
    hasher.update(finding.evidence.as_bytes());
    format!("finding-{}", short_hash(&format!("{:x}", hasher.finalize())))
}

fn extract_json_candidate(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return Some(trimmed);
    }

    let fence_start = trimmed.find("```json").or_else(|| trimmed.find("```JSON"))?;
    let after_fence = &trimmed[fence_start..];
    let content_start = after_fence.find('\n')? + 1;
    let content = &after_fence[content_start..];
    let content_end = content.find("```")?;
    Some(content[..content_end].trim())
}

fn path_for_index(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn table_escape(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

fn current_epoch_seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn short_hash(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
}

#[cfg(test)]
mod tests {
    use super::{persist_review_artifact, review_diff_hash, ReviewParseStatus, ReviewReport};
    use crate::code_review::{ReviewScope, ReviewSeverity, ReviewTarget};

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
        assert!(artifact.index_path.exists());
        let markdown = std::fs::read_to_string(&artifact.markdown_path).expect("read markdown");
        assert!(markdown.contains("# Sego Review Report"));
        assert!(markdown.contains("No findings."));
        let index = std::fs::read_to_string(&artifact.index_path).expect("read index");
        assert!(index.contains(&artifact.id));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn parses_structured_json_findings() {
        let report = ReviewReport::from_model_output(
            r#"{"findings":[{"severity":"high","file":"src/lib.rs","line":42,"title":"Missing error handling","evidence":"new call unwraps the result","risk":"panic in production","suggestion":"propagate the error","confidence":0.91,"verification_hint":"cargo test"}]}"#,
        );

        assert_eq!(report.parse_status, ReviewParseStatus::Structured);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].severity, ReviewSeverity::High);
        assert_eq!(report.findings[0].line, Some(42));
        assert!(report.findings[0].id.starts_with("finding-"));
        assert_eq!(report.highest_severity(), Some(ReviewSeverity::High));
    }

    #[test]
    fn parses_fenced_json_findings() {
        let report = ReviewReport::from_model_output(
            "Review output:\n```json\n{\"findings\":[{\"severity\":\"low\",\"file\":\"src/main.rs\",\"line\":null,\"title\":\"Add test\",\"evidence\":\"no test changed\",\"risk\":\"regression may be missed\",\"suggestion\":\"add a focused test\",\"confidence\":0.5,\"verification_hint\":null}]}\n```",
        );

        assert_eq!(report.parse_status, ReviewParseStatus::Structured);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].severity, ReviewSeverity::Low);
    }

    #[test]
    fn falls_back_to_raw_text_for_invalid_json() {
        let report = ReviewReport::from_model_output("There may be a bug, but no JSON here.");

        assert_eq!(report.parse_status, ReviewParseStatus::FallbackRawText);
        assert!(report.findings.is_empty());
        assert!(report.raw_text.contains("no JSON"));
    }

    #[test]
    fn persists_structured_findings_in_json_markdown_and_index() {
        let root = temp_path("review-structured-artifact");
        let target = target_with_diff("diff --git a/src/lib.rs b/src/lib.rs\n");
        let report = ReviewReport::from_model_output(
            r#"{"findings":[{"severity":"critical","file":"src/lib.rs","line":7,"title":"Credential leak","evidence":"diff adds a literal secret","risk":"secret exposure","suggestion":"remove the secret and load it from config","confidence":0.98,"verification_hint":"rg secret"}]}"#,
        );

        let artifact =
            persist_review_artifact(&root, &target, &report).expect("artifact should persist");

        let json = std::fs::read_to_string(&artifact.json_path).expect("read json");
        assert!(json.contains("\"parse_status\": \"structured\""));
        assert!(json.contains("\"highest_severity\": \"critical\""));
        assert!(json.contains("\"findings\""));

        let markdown = std::fs::read_to_string(&artifact.markdown_path).expect("read markdown");
        assert!(markdown.contains("## Findings"));
        assert!(markdown.contains("Credential leak"));

        let index = std::fs::read_to_string(&artifact.index_path).expect("read index");
        assert!(index.contains("\"finding_count\":1"));
        assert!(index.contains("\"highest_severity\":\"critical\""));

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
