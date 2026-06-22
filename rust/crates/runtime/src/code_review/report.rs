use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{ReviewScope, ReviewSeverity, ReviewTarget};

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
    /// C20.5-A: raw text contains findings-like content but JSON could not be parsed.
    /// Prevents misleading "0 findings" display when the model clearly produced findings.
    ParseAttemptedButFailed,
}

impl ReviewParseStatus {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Structured => "structured",
            Self::FallbackRawText => "fallback_raw_text",
            Self::ParseAttemptedButFailed => "parse_attempted_but_failed",
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
            // C20.5-A: if text looks like it contains findings, don't silently report 0.
            if trimmed.contains("\"findings\"") || trimmed.contains("\"findings\":") {
                return Self {
                    findings: Vec::new(),
                    raw_text,
                    parse_status: ReviewParseStatus::ParseAttemptedButFailed,
                };
            }
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
            Err(_) => {
                // C20.5-A: parsing failed but candidate text was found.
                if trimmed.contains("\"findings\"") || trimmed.contains("\"findings\":") {
                    return Self {
                        findings: Vec::new(),
                        raw_text,
                        parse_status: ReviewParseStatus::ParseAttemptedButFailed,
                    };
                }
                Self::no_findings(raw_text)
            }
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
pub struct ReviewIndexEntry {
    pub id: String,
    pub created_at_epoch_seconds: u64,
    pub scope: String,
    pub diff_hash: String,
    pub finding_count: usize,
    pub highest_severity: Option<ReviewSeverity>,
    pub parse_status: ReviewParseStatus,
    pub json_path: String,
    pub markdown_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewFindingStatus {
    Open,
    Acknowledged,
    Fixed,
    Ignored,
}

impl ReviewFindingStatus {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Acknowledged => "acknowledged",
            Self::Fixed => "fixed",
            Self::Ignored => "ignored",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "open" => Ok(Self::Open),
            "acknowledged" | "ack" => Ok(Self::Acknowledged),
            "fixed" | "resolved" => Ok(Self::Fixed),
            "ignored" | "ignore" => Ok(Self::Ignored),
            other => Err(format!(
                "unsupported review finding status `{other}` (expected open, acknowledged, fixed, or ignored)"
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFindingStatusEntry {
    pub report_id: String,
    pub finding_id: String,
    pub status: ReviewFindingStatus,
    pub note: Option<String>,
    pub updated_at_epoch_seconds: u64,
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

pub fn load_review_index(workspace_root: &Path) -> io::Result<Vec<ReviewIndexEntry>> {
    let index_path = workspace_root.join(".sego").join("reviews").join("index.jsonl");
    if !index_path.exists() {
        return Ok(Vec::new());
    }

    let contents = fs::read_to_string(index_path)?;
    contents
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(line_index, line)| {
            let line = line.trim_start_matches('\u{feff}');
            serde_json::from_str::<ReviewIndexEntry>(line).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid review index entry at line {}: {error}", line_index + 1),
                )
            })
        })
        .collect()
}

pub fn record_review_finding_status(
    workspace_root: &Path,
    report_id: &str,
    finding_id: &str,
    status: ReviewFindingStatus,
    note: Option<String>,
) -> io::Result<ReviewFindingStatusEntry> {
    let reviews_dir = workspace_root.join(".sego").join("reviews");
    fs::create_dir_all(&reviews_dir)?;

    let entry = ReviewFindingStatusEntry {
        report_id: report_id.to_string(),
        finding_id: finding_id.to_string(),
        status,
        note: note.filter(|value| !value.trim().is_empty()),
        updated_at_epoch_seconds: current_epoch_seconds(),
    };

    let status_path = reviews_dir.join("status.jsonl");
    let mut line = serde_json::to_string(&entry)?;
    line.push('\n');
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(status_path)?
        .write_all(line.as_bytes())?;

    Ok(entry)
}

pub fn load_review_finding_statuses(
    workspace_root: &Path,
) -> io::Result<Vec<ReviewFindingStatusEntry>> {
    let status_path = workspace_root.join(".sego").join("reviews").join("status.jsonl");
    if !status_path.exists() {
        return Ok(Vec::new());
    }

    let contents = fs::read_to_string(status_path)?;
    contents
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(line_index, line)| {
            let line = line.trim_start_matches('\u{feff}');
            serde_json::from_str::<ReviewFindingStatusEntry>(line).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid review status entry at line {}: {error}", line_index + 1),
                )
            })
        })
        .collect()
}

pub fn latest_review_finding_statuses(
    workspace_root: &Path,
    report_id: &str,
) -> io::Result<BTreeMap<String, ReviewFindingStatusEntry>> {
    let mut statuses = BTreeMap::new();
    for entry in load_review_finding_statuses(workspace_root)? {
        if entry.report_id == report_id {
            statuses.insert(entry.finding_id.clone(), entry);
        }
    }
    Ok(statuses)
}

#[must_use]
pub fn review_diff_hash(target: &ReviewTarget) -> String {
    let mut hasher = Sha256::new();
    hasher.update(target.scope.label().as_bytes());
    if matches!(&target.scope, ReviewScope::FullRepo(_)) {
        hasher.update(b"\n---full_tree---\n");
        hasher.update(target.full_tree.as_bytes());
    } else {
        hasher.update(b"\n---staged---\n");
        hasher.update(target.staged_diff.as_bytes());
        hasher.update(b"\n---unstaged---\n");
        hasher.update(target.unstaged_diff.as_bytes());
    }
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

    output.push_str("## Verification Self-check\n\n");
    output.push_str("- Sego only treats tests, lint checks, syntax checks, dry-runs, generated files, and runtime outputs as verified when the evidence is present in the captured review context.\n");
    output.push_str("- Claims without captured evidence should be treated as unverified recommendations, not completed validation.\n");
    output.push_str("- Reviewers should double-check behavior changes such as output schema, column names, retry semantics, request headers, filesystem paths, and data-flow changes before accepting fixes.\n");
    if report.findings.is_empty() {
        output.push_str("- No structured verification hints were captured.\n");
    } else {
        let hint_count =
            report.findings.iter().filter(|finding| finding.verification_hint.is_some()).count();
        let _ = writeln!(
            output,
            "- Structured findings with verification hints: `{hint_count}/{}`.",
            report.findings.len()
        );
    }
    output.push('\n');

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

    // Check for fenced JSON block: ```json ... ```
    let mut offset = 0;
    for line in trimmed.split_inclusive('\n') {
        let line_without_eol = line.trim_end_matches(&['\r', '\n'][..]);
        if line_without_eol.trim().eq_ignore_ascii_case("```json") {
            let content_start = offset + line.len();
            let content = &trimmed[content_start..];
            let mut content_offset = 0;
            for content_line in content.split_inclusive('\n') {
                let candidate = content_line.trim_end_matches(&['\r', '\n'][..]).trim();
                if candidate == "```" {
                    return Some(content[..content_offset].trim());
                }
                content_offset += content_line.len();
            }
            return None;
        }
        offset += line.len();
    }

    // C20.5-A: fallback — search for a JSON object containing "findings" in prose text.
    extract_json_from_prose(trimmed)
}

/// C20.5-A: Search for a `{...}` JSON object in arbitrary prose text.
/// Scans every `{` in the text, extracts the balanced brace object,
/// and returns the first one that contains `"findings"`.
fn extract_json_from_prose(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut search_start = 0;
    while let Some(rel_pos) = bytes[search_start..].iter().position(|&b| b == b'{') {
        let abs_pos = search_start + rel_pos;
        let slice = &text[abs_pos..];
        if let Some(end_offset) = find_balanced_json_end(slice) {
            let candidate = &slice[..end_offset];
            if candidate.contains("\"findings\"") {
                return Some(candidate);
            }
            // Skip past this object to continue searching.
            search_start = abs_pos + end_offset;
        } else {
            search_start = abs_pos + 1;
        }
    }
    None
}

/// Given a string slice that starts with `{`, find the byte offset of the
/// matching closing `}` (respecting string escapes). Returns `None` if the
/// braces are not balanced. `char_indices()` yields byte offsets, so the
/// returned offset is safe for slicing the original UTF-8 string.
fn find_balanced_json_end(slice: &str) -> Option<usize> {
    let mut brace_depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, ch) in slice.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' && in_string {
            escaped = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            continue;
        }
        if !in_string {
            if ch == '{' {
                brace_depth += 1;
            } else if ch == '}' {
                brace_depth -= 1;
                if brace_depth == 0 {
                    return Some(i + ch.len_utf8());
                }
            }
        }
    }
    None
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
    use super::{
        latest_review_finding_statuses, load_review_finding_statuses, load_review_index,
        persist_review_artifact, record_review_finding_status, review_diff_hash, ReviewArtifact,
        ReviewFinding, ReviewFindingStatus, ReviewFindingStatusEntry, ReviewIndexEntry,
        ReviewParseStatus, ReviewReport,
    };
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
        assert!(markdown.contains("## Verification Self-check"));
        let index = std::fs::read_to_string(&artifact.index_path).expect("read index");
        assert!(index.contains(&artifact.id));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn review_artifact_round_trips_through_json() {
        // Golden fixture: construct a ReviewArtifact with a finding, serialize,
        // deserialize, and verify every field survives the round-trip.
        let artifact = ReviewArtifact {
            schema_version: 1,
            id: "rev-test-001".to_string(),
            created_at_epoch_seconds: 1_719_849_600,
            scope: "staged".to_string(),
            diff_hash: "sha256:abc123".to_string(),
            finding_count: 1,
            highest_severity: Some(ReviewSeverity::High),
            parse_status: ReviewParseStatus::Structured,
            git_status: "## main...origin/main".to_string(),
            findings: vec![ReviewFinding {
                id: "f-001".to_string(),
                severity: ReviewSeverity::High,
                file: "src/lib.rs".to_string(),
                line: Some(42),
                title: "Missing error handling".to_string(),
                evidence: "unwrap on result".to_string(),
                risk: "panic in production".to_string(),
                suggestion: "propagate the error".to_string(),
                confidence: 0.91,
                verification_hint: Some("cargo test".to_string()),
            }],
            raw_text: "raw model output".to_string(),
        };

        let json = serde_json::to_string(&artifact).expect("serialize artifact");
        let parsed: ReviewArtifact = serde_json::from_str(&json).expect("deserialize artifact");

        // Field-by-field round-trip verification.
        assert_eq!(parsed.schema_version, 1);
        assert_eq!(parsed.id, "rev-test-001");
        assert_eq!(parsed.scope, "staged");
        assert_eq!(parsed.diff_hash, "sha256:abc123");
        assert_eq!(parsed.finding_count, 1);
        assert_eq!(parsed.highest_severity, Some(ReviewSeverity::High));
        assert_eq!(parsed.parse_status, ReviewParseStatus::Structured);
        assert_eq!(parsed.findings.len(), 1);
        assert_eq!(parsed.findings[0].severity, ReviewSeverity::High);
        assert_eq!(parsed.findings[0].line, Some(42));
        assert_eq!(parsed.findings[0].confidence, 0.91);
        assert_eq!(parsed.raw_text, "raw model output");

        // Enum serialization must be snake_case (aligned with schema).
        assert!(json.contains(r#""severity":"high""#), "severity must be snake_case: {json}");
        assert!(
            json.contains(r#""parse_status":"structured""#),
            "parse_status must be snake_case: {json}"
        );

        // schema_version must be present in JSON (schema required field).
        assert!(json.contains(r#""schema_version":1"#), "schema_version must be in JSON: {json}");

        // finding_count must equal findings length (schema consistency).
        assert_eq!(parsed.finding_count, parsed.findings.len());
    }

    #[test]
    fn review_artifact_with_no_findings_round_trips() {
        let artifact = ReviewArtifact {
            schema_version: 1,
            id: "rev-test-002".to_string(),
            created_at_epoch_seconds: 1_719_849_600,
            scope: "workspace".to_string(),
            diff_hash: "sha256:none".to_string(),
            finding_count: 0,
            highest_severity: None,
            parse_status: ReviewParseStatus::FallbackRawText,
            git_status: String::new(),
            findings: vec![],
            raw_text: "No findings.".to_string(),
        };

        let json = serde_json::to_string(&artifact).expect("serialize");
        let parsed: ReviewArtifact = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.finding_count, 0);
        assert!(parsed.findings.is_empty());
        assert_eq!(parsed.highest_severity, None);
        // null severity must serialize correctly.
        assert!(json.contains(r#""highest_severity":null"#), "null severity: {json}");
    }

    #[test]
    fn review_index_entry_round_trips_through_json() {
        let entry = ReviewIndexEntry {
            id: "rev-test-003".to_string(),
            created_at_epoch_seconds: 1_719_849_600,
            scope: "staged".to_string(),
            diff_hash: "sha256:def456".to_string(),
            finding_count: 2,
            highest_severity: Some(ReviewSeverity::Critical),
            parse_status: ReviewParseStatus::Structured,
            json_path: ".sego/reviews/rev-test-003.json".to_string(),
            markdown_path: ".sego/reviews/rev-test-003.md".to_string(),
        };

        let json = serde_json::to_string(&entry).expect("serialize index entry");
        let parsed: ReviewIndexEntry =
            serde_json::from_str(&json).expect("deserialize index entry");

        assert_eq!(parsed.id, entry.id);
        assert_eq!(parsed.highest_severity, Some(ReviewSeverity::Critical));
        assert_eq!(parsed.finding_count, 2);
        // Index entries do NOT have schema_version (Codex D-B-2).
        assert!(!json.contains("schema_version"), "index must not have schema_version: {json}");
    }

    #[test]
    fn review_finding_status_entry_round_trips_through_json() {
        let entry = ReviewFindingStatusEntry {
            report_id: "rev-test-001".to_string(),
            finding_id: "f-001".to_string(),
            status: ReviewFindingStatus::Fixed,
            note: Some("fixed in commit abc".to_string()),
            updated_at_epoch_seconds: 1_719_849_700,
        };

        let json = serde_json::to_string(&entry).expect("serialize status entry");
        let parsed: ReviewFindingStatusEntry =
            serde_json::from_str(&json).expect("deserialize status entry");

        assert_eq!(parsed.status, ReviewFindingStatus::Fixed);
        assert_eq!(parsed.finding_id, "f-001");
        // status must be snake_case.
        assert!(json.contains(r#""status":"fixed""#), "status must be snake_case: {json}");
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
    fn parses_fenced_json_with_markdown_fence_inside_string_field() {
        let report = ReviewReport::from_model_output(
            "```json\n{\"findings\":[{\"severity\":\"medium\",\"file\":\"src/main.rs\",\"line\":1970,\"title\":\"Nested fence\",\"evidence\":\"```rust\\nprintln!(\\\"hello\\\");\\n```\",\"risk\":\"parser may truncate the JSON at the nested fence\",\"suggestion\":\"only treat a line-level fence as the outer closing fence\",\"confidence\":0.8,\"verification_hint\":\"cargo test\"}]}\n```",
        );

        assert_eq!(report.parse_status, ReviewParseStatus::Structured);
        assert_eq!(report.findings.len(), 1);
        assert!(report.findings[0].evidence.contains("```rust"));
    }

    #[test]
    fn falls_back_to_raw_text_for_invalid_json() {
        let report = ReviewReport::from_model_output("There may be a bug, but no JSON here.");

        assert_eq!(report.parse_status, ReviewParseStatus::FallbackRawText);
        assert!(report.findings.is_empty());
        assert!(report.raw_text.contains("no JSON"));
    }

    // C20.5-A: parser robustness tests.

    #[test]
    fn parses_raw_json_findings_in_prose_text() {
        let report = ReviewReport::from_model_output(
            "Here is my review analysis.\n\n{\"findings\":[{\"severity\":\"high\",\"file\":\"src/lib.rs\",\"line\":42,\"title\":\"Missing error handling\",\"evidence\":\"unwrap on result\",\"risk\":\"panic\",\"suggestion\":\"propagate error\",\"confidence\":0.91}]}\n\nLet me know if you need more detail.",
        );

        assert_eq!(report.parse_status, ReviewParseStatus::Structured);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].severity, ReviewSeverity::High);
    }

    #[test]
    fn parses_fenced_json_embedded_in_prose() {
        let report = ReviewReport::from_model_output(
            "Review complete. Here is the output:\n\n```json\n{\"findings\":[{\"severity\":\"low\",\"file\":\"src/main.rs\",\"line\":null,\"title\":\"Missing test\",\"evidence\":\"no test changed\",\"risk\":\"regression\",\"suggestion\":\"add test\",\"confidence\":0.5}]}\n```\n\nDone.",
        );

        assert_eq!(report.parse_status, ReviewParseStatus::Structured);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].severity, ReviewSeverity::Low);
    }

    #[test]
    fn reports_parse_attempted_when_findings_like_content_present_but_not_parsable() {
        let report = ReviewReport::from_model_output(
            "I found some issues: {\"findings\": [malformed json here}",
        );

        assert_eq!(report.parse_status, ReviewParseStatus::ParseAttemptedButFailed);
        assert!(report.findings.is_empty());
        // Must not silently show 0 findings as if the review was clean.
    }

    #[test]
    fn parses_prose_with_json_containing_nested_braces_in_strings() {
        let report = ReviewReport::from_model_output(
            "Result:\n{\"findings\":[{\"severity\":\"medium\",\"file\":\"src/main.rs\",\"line\":10,\"title\":\"Nested braces\",\"evidence\":\"found { and } in string\",\"risk\":\"parser bug\",\"suggestion\":\"escape properly\",\"confidence\":0.8}]}\nEnd.",
        );

        assert_eq!(report.parse_status, ReviewParseStatus::Structured);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].evidence, "found { and } in string");
    }

    #[test]
    fn parses_raw_json_no_findings_as_structured() {
        // C20.5-A R2: {"findings":[]} must parse as Structured, not fallback.
        let report = ReviewReport::from_model_output(r#"{"findings":[]}"#);
        assert_eq!(report.parse_status, ReviewParseStatus::Structured);
        assert!(report.findings.is_empty());
    }

    #[test]
    fn parses_pretty_json_object_embedded_in_prose() {
        // C20.5-A R3: pretty-printed JSON with newlines after `{` must parse.
        let report = ReviewReport::from_model_output(
            "Here is output:\n\n{\n  \"findings\": [\n    {\n      \"severity\": \"low\",\n      \"file\": \"src/lib.rs\",\n      \"line\": 1,\n      \"title\": \"Pretty JSON\",\n      \"evidence\": \"pretty printed\",\n      \"risk\": \"none\",\n      \"suggestion\": \"none\",\n      \"confidence\": 0.5\n    }\n  ]\n}\n\nDone.",
        );
        assert_eq!(report.parse_status, ReviewParseStatus::Structured);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].title, "Pretty JSON");
    }

    #[test]
    fn parses_pretty_json_with_cjk_and_emoji_in_string_fields() {
        let report = ReviewReport::from_model_output(
            "Review:\n{\n  \"findings\": [\n    {\n      \"severity\": \"low\",\n      \"file\": \"src/中文.rs\",\n      \"line\": 8,\n      \"title\": \"中文路径 ✅\",\n      \"evidence\": \"含有 emoji 🚀 和中文字符\",\n      \"risk\": \"低风险\",\n      \"suggestion\": \"保持 UTF-8 字节边界安全\",\n      \"confidence\": 0.7\n    }\n  ]\n}\nEnd.",
        );

        assert_eq!(report.parse_status, ReviewParseStatus::Structured);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].title, "中文路径 ✅");
        assert!(report.findings[0].evidence.contains("🚀"));
    }

    #[test]
    fn parses_json_after_unbalanced_brace_noise() {
        let report = ReviewReport::from_model_output(
            "Model note with an unmatched brace { before the real JSON.\n{\"findings\":[{\"severity\":\"low\",\"file\":\"src/lib.rs\",\"line\":2,\"title\":\"After noise\",\"evidence\":\"valid object after noise\",\"risk\":\"low\",\"suggestion\":\"keep scanning\",\"confidence\":0.6}]}",
        );

        assert_eq!(report.parse_status, ReviewParseStatus::Structured);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].title, "After noise");
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
        assert!(markdown.contains("Structured findings with verification hints: `1/1`"));

        let index = std::fs::read_to_string(&artifact.index_path).expect("read index");
        assert!(index.contains("\"finding_count\":1"));
        assert!(index.contains("\"highest_severity\":\"critical\""));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn loads_review_index_entries() {
        let root = temp_path("review-index-load");
        let target = target_with_diff("diff --git a/src/lib.rs b/src/lib.rs\n");
        let report = ReviewReport::from_model_output(
            r#"{"findings":[{"severity":"medium","file":"src/lib.rs","line":9,"title":"Missing branch coverage","evidence":"new branch has no test","risk":"regression can slip","suggestion":"add a focused test","confidence":0.72,"verification_hint":"cargo test"}]}"#,
        );
        let artifact =
            persist_review_artifact(&root, &target, &report).expect("artifact should persist");

        let entries = load_review_index(&root).expect("index should load");

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, artifact.id);
        assert_eq!(entries[0].finding_count, 1);
        assert_eq!(entries[0].highest_severity, Some(ReviewSeverity::Medium));
        assert_eq!(entries[0].parse_status, ReviewParseStatus::Structured);

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn missing_review_index_loads_as_empty_history() {
        let root = temp_path("review-index-missing");

        let entries = load_review_index(&root).expect("missing index should be empty");

        assert!(entries.is_empty());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn parses_review_finding_status_aliases() {
        assert_eq!(ReviewFindingStatus::parse("open"), Ok(ReviewFindingStatus::Open));
        assert_eq!(ReviewFindingStatus::parse("ack"), Ok(ReviewFindingStatus::Acknowledged));
        assert_eq!(ReviewFindingStatus::parse("resolved"), Ok(ReviewFindingStatus::Fixed));
        assert_eq!(ReviewFindingStatus::parse("ignore"), Ok(ReviewFindingStatus::Ignored));
        assert!(ReviewFindingStatus::parse("done").is_err());
    }

    #[test]
    fn records_and_loads_latest_review_finding_statuses() {
        let root = temp_path("review-finding-status");

        record_review_finding_status(
            &root,
            "review-1",
            "finding-1",
            ReviewFindingStatus::Acknowledged,
            Some("triaged".to_string()),
        )
        .expect("status should record");
        record_review_finding_status(
            &root,
            "review-1",
            "finding-1",
            ReviewFindingStatus::Fixed,
            Some("covered by test".to_string()),
        )
        .expect("status should record");
        record_review_finding_status(
            &root,
            "review-1",
            "finding-2",
            ReviewFindingStatus::Ignored,
            None,
        )
        .expect("status should record");

        let all_statuses = load_review_finding_statuses(&root).expect("statuses should load");
        let latest =
            latest_review_finding_statuses(&root, "review-1").expect("latest statuses should load");

        assert_eq!(all_statuses.len(), 3);
        assert_eq!(latest.len(), 2);
        assert_eq!(latest["finding-1"].status, ReviewFindingStatus::Fixed);
        assert_eq!(latest["finding-1"].note.as_deref(), Some("covered by test"));
        assert_eq!(latest["finding-2"].status, ReviewFindingStatus::Ignored);

        let _ = std::fs::remove_dir_all(root);
    }

    fn target_with_diff(diff: &str) -> ReviewTarget {
        ReviewTarget {
            scope: ReviewScope::Workspace,
            git_status: "## main\n M src/lib.rs".to_string(),
            staged_diff: String::new(),
            unstaged_diff: diff.to_string(),
            full_tree: String::new(),
            workspace_root: None,
        }
    }

    fn target_full_repo(full_tree: &str) -> ReviewTarget {
        ReviewTarget {
            scope: ReviewScope::FullRepo("/test/repo".into()),
            git_status: String::new(),
            staged_diff: String::new(),
            unstaged_diff: String::new(),
            full_tree: full_tree.to_string(),
            workspace_root: Some("/test/repo".into()),
        }
    }

    #[test]
    fn full_repo_diff_hash_uses_full_tree_not_diffs() {
        let t1 = target_full_repo("tree content A");
        let t2 = target_full_repo("tree content B");
        let t1_dup = target_full_repo("tree content A");

        let h1 = review_diff_hash(&t1);
        let h2 = review_diff_hash(&t2);
        let h1_dup = review_diff_hash(&t1_dup);

        assert_eq!(h1, h1_dup, "same full_tree should produce same hash");
        assert_ne!(h1, h2, "different full_tree should produce different hash");
        assert!(!h1.is_empty());
        assert!(!h2.is_empty());
    }

    #[test]
    fn full_repo_hash_differs_from_diff_based_hash() {
        let full = target_full_repo("content");
        let diff = target_with_diff("diff --git a/a b/a\n");

        let h_full = review_diff_hash(&full);
        let h_diff = review_diff_hash(&diff);

        assert_ne!(h_full, h_diff, "FullRepo hash should differ from diff-based hash");
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("sego-{name}-{unique}"))
    }
}
