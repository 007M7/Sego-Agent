use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{ReviewScope, ReviewSeverity, ReviewTarget};

/// C20.6-B UX-D: lightweight evidence gate status attached to each finding
/// after deterministic validation against the captured review target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    /// Finding file path was observed in the captured review target.
    Verified,
    /// File path could not be located in the captured target.
    UnverifiedFile,
    /// File located but cited line number is outside captured range.
    UnverifiedLine,
    /// Dependency-related finding without manifest evidence in scope.
    UnverifiedDependency,
    /// Finding refers to content outside captured review scope.
    ScopeNotCaptured,
    /// File was listed in scope but its content was not captured.
    ContentNotCaptured,
    /// File content was captured only partially/truncated.
    ContentTruncated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewContentStatus {
    NotCaptured,
    Full,
    Truncated,
    SkippedLarge,
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFileEvidence {
    /// Repository-relative, sanitized path only.
    pub path: String,
    pub listed: bool,
    pub content_status: ReviewContentStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captured_line_count: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewEvidenceScopeKind {
    Diff,
    FullRepo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewEvidenceCoverage {
    pub scope_kind: ReviewEvidenceScopeKind,
    pub file_tree_count: usize,
    pub observed_files: Vec<ReviewFileEvidence>,
    pub captured_file_count: usize,
    pub truncated_file_count: usize,
    pub not_captured_file_count: usize,
    pub skipped_large_file_count: usize,
    pub unreadable_file_count: usize,
}

impl EvidenceStatus {
    /// Every label, in declaration order. Kept next to the enum so the
    /// contract conformance suite can pin it against the schema enums.
    pub const ALL_LABELS: [&str; 7] = [
        "verified",
        "unverified_file",
        "unverified_line",
        "unverified_dependency",
        "scope_not_captured",
        "content_not_captured",
        "content_truncated",
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::UnverifiedFile => "unverified_file",
            Self::UnverifiedLine => "unverified_line",
            Self::UnverifiedDependency => "unverified_dependency",
            Self::ScopeNotCaptured => "scope_not_captured",
            Self::ContentNotCaptured => "content_not_captured",
            Self::ContentTruncated => "content_truncated",
        }
    }
}

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
    /// C20.6-B UX-D: optional evidence gate status; `None` for legacy artifacts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_status: Option<EvidenceStatus>,
}

/// C20.6-B UX-D: Deterministic evidence gate that annotates findings with
/// `evidence_status` based on the captured review target. Never deletes
/// findings; only annotates. Returns a new Vec with the annotation applied.
#[must_use]
pub fn evaluate_evidence_gate(
    findings: Vec<ReviewFinding>,
    target: &ReviewTarget,
) -> Vec<ReviewFinding> {
    let coverage = build_evidence_coverage(target);
    let scope_files: Vec<String> =
        coverage.observed_files.iter().map(|entry| entry.path.clone()).collect();
    let has_manifest = scope_has_manifest(&scope_files);
    findings
        .into_iter()
        .map(|mut finding| {
            if finding.evidence_status.is_some() {
                return finding;
            }
            finding.evidence_status = Some(classify_finding_evidence(
                &finding,
                target,
                &coverage,
                &scope_files,
                has_manifest,
            ));
            finding
        })
        .collect()
}

fn classify_finding_evidence(
    finding: &ReviewFinding,
    target: &ReviewTarget,
    coverage: &ReviewEvidenceCoverage,
    scope_files: &[String],
    has_manifest: bool,
) -> EvidenceStatus {
    let trimmed_file = finding.file.trim();
    if trimmed_file.is_empty() || trimmed_file == "-" || trimmed_file == "<unknown>" {
        return EvidenceStatus::ScopeNotCaptured;
    }
    if looks_like_dependency_finding(finding) && !has_manifest {
        return EvidenceStatus::UnverifiedDependency;
    }
    let Some(normalized) = normalize_repo_relative_path(trimmed_file) else {
        return EvidenceStatus::ScopeNotCaptured;
    };
    let Some(file_evidence) = find_file_evidence(coverage, &normalized) else {
        if scope_files.is_empty() {
            return EvidenceStatus::ScopeNotCaptured;
        }
        return EvidenceStatus::UnverifiedFile;
    };
    match file_evidence.content_status {
        ReviewContentStatus::NotCaptured
        | ReviewContentStatus::SkippedLarge
        | ReviewContentStatus::Unreadable => return EvidenceStatus::ContentNotCaptured,
        ReviewContentStatus::Truncated => return EvidenceStatus::ContentTruncated,
        ReviewContentStatus::Full => {}
    }
    if let Some(line) = finding.line {
        if line == 0 {
            return EvidenceStatus::UnverifiedLine;
        }
        if let Some(max_line) = file_evidence
            .captured_line_count
            .or_else(|| max_line_for_file_in_target(target, &normalized))
        {
            if (line as usize) > max_line {
                return EvidenceStatus::UnverifiedLine;
            }
        }
    }
    EvidenceStatus::Verified
}

fn scope_has_manifest(scope_files: &[String]) -> bool {
    const MANIFESTS: &[&str] = &[
        "Cargo.toml",
        "Cargo.lock",
        "package.json",
        "package-lock.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "pyproject.toml",
        "requirements.txt",
        "setup.py",
        "go.mod",
        "go.sum",
        "Gemfile",
        "Gemfile.lock",
    ];
    scope_files.iter().any(|f| {
        let name = f.rsplit('/').next().unwrap_or(f);
        MANIFESTS.iter().any(|m| name.eq_ignore_ascii_case(m))
    })
}

fn looks_like_dependency_finding(finding: &ReviewFinding) -> bool {
    let haystack =
        format!("{} {} {} {}", finding.title, finding.evidence, finding.risk, finding.suggestion)
            .to_ascii_lowercase();
    let keywords = [
        "dependency",
        "dependencies",
        "transitive",
        "cve",
        "vulnerable",
        "manifest",
        "lockfile",
        "lock file",
        "supply chain",
        "依赖",
    ];
    keywords.iter().any(|k| haystack.contains(k))
}

#[must_use]
pub fn build_evidence_coverage(target: &ReviewTarget) -> ReviewEvidenceCoverage {
    if matches!(&target.scope, ReviewScope::FullRepo(_)) || !target.full_tree.trim().is_empty() {
        full_repo_evidence_coverage(&target.full_tree)
    } else {
        diff_evidence_coverage(target)
    }
}

fn diff_evidence_coverage(target: &ReviewTarget) -> ReviewEvidenceCoverage {
    let mut files: BTreeMap<String, ReviewFileEvidence> = BTreeMap::new();
    for diff in [&target.staged_diff, &target.unstaged_diff] {
        for path in diff_file_paths(diff) {
            files.entry(path.clone()).or_insert_with(|| ReviewFileEvidence {
                captured_line_count: max_line_for_file_in_target(target, &path),
                path,
                listed: true,
                content_status: ReviewContentStatus::Full,
            });
        }
    }
    finalize_evidence_coverage(ReviewEvidenceScopeKind::Diff, files.len(), files)
}

fn full_repo_evidence_coverage(full_tree: &str) -> ReviewEvidenceCoverage {
    let mut files: BTreeMap<String, ReviewFileEvidence> = BTreeMap::new();
    for path in file_tree_paths(full_tree) {
        files.insert(
            path.clone(),
            ReviewFileEvidence {
                path,
                listed: true,
                content_status: ReviewContentStatus::NotCaptured,
                captured_line_count: None,
            },
        );
    }

    for (path, status, line_count) in key_file_evidence(full_tree) {
        if let Some(entry) = files.get_mut(&path) {
            entry.content_status = status;
            entry.captured_line_count = line_count;
        }
    }

    finalize_evidence_coverage(ReviewEvidenceScopeKind::FullRepo, files.len(), files)
}

fn finalize_evidence_coverage(
    scope_kind: ReviewEvidenceScopeKind,
    file_tree_count: usize,
    files: BTreeMap<String, ReviewFileEvidence>,
) -> ReviewEvidenceCoverage {
    let observed_files: Vec<ReviewFileEvidence> = files.into_values().collect();
    let captured_file_count = observed_files
        .iter()
        .filter(|entry| entry.content_status == ReviewContentStatus::Full)
        .count();
    let truncated_file_count = observed_files
        .iter()
        .filter(|entry| entry.content_status == ReviewContentStatus::Truncated)
        .count();
    let not_captured_file_count = observed_files
        .iter()
        .filter(|entry| entry.content_status == ReviewContentStatus::NotCaptured)
        .count();
    let skipped_large_file_count = observed_files
        .iter()
        .filter(|entry| entry.content_status == ReviewContentStatus::SkippedLarge)
        .count();
    let unreadable_file_count = observed_files
        .iter()
        .filter(|entry| entry.content_status == ReviewContentStatus::Unreadable)
        .count();
    ReviewEvidenceCoverage {
        scope_kind,
        file_tree_count,
        observed_files,
        captured_file_count,
        truncated_file_count,
        not_captured_file_count,
        skipped_large_file_count,
        unreadable_file_count,
    }
}

fn diff_file_paths(diff: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git a/") {
            if let Some(idx) = rest.find(" b/") {
                if let Some(path) = normalize_repo_relative_path(&rest[..idx]) {
                    paths.push(path);
                }
            }
        }
    }
    paths
}

fn file_tree_paths(full_tree: &str) -> Vec<String> {
    let mut files = Vec::new();
    let mut in_tree = false;
    for line in full_tree.lines() {
        if line.starts_with("## File tree") {
            in_tree = true;
            continue;
        }
        if in_tree {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with("##") {
                break;
            }
            if let Some(path) = normalize_repo_relative_path(trimmed) {
                files.push(path);
            }
        }
    }
    files
}

fn key_file_evidence(full_tree: &str) -> Vec<(String, ReviewContentStatus, Option<usize>)> {
    let mut evidence = Vec::new();
    let lines: Vec<&str> = full_tree.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if let Some(header) = trimmed.strip_prefix("### ") {
            if let Some((path, status)) = parse_key_file_header(header) {
                let line_count = captured_line_count_after_header(&lines, i + 1);
                evidence.push((path, status, line_count));
            }
        }
        i += 1;
    }
    evidence
}

fn parse_key_file_header(header: &str) -> Option<(String, ReviewContentStatus)> {
    let header = header.trim();
    let (path_part, marker) = if let Some(start) = header.rfind(" [") {
        let marker = header[start + 2..].strip_suffix(']')?;
        (&header[..start], Some(marker.trim()))
    } else {
        (header, None)
    };
    let path = normalize_repo_relative_path(path_part.trim())?;
    let status = match marker.unwrap_or("full") {
        "full" => ReviewContentStatus::Full,
        "truncated" => ReviewContentStatus::Truncated,
        marker if marker.starts_with("skipped") => ReviewContentStatus::SkippedLarge,
        marker if marker.starts_with("unreadable") => ReviewContentStatus::Unreadable,
        _ => ReviewContentStatus::NotCaptured,
    };
    Some((path, status))
}

fn captured_line_count_after_header(lines: &[&str], mut idx: usize) -> Option<usize> {
    while idx < lines.len() && lines[idx].trim().is_empty() {
        idx += 1;
    }
    if idx >= lines.len() || lines[idx].trim() != "```" {
        return None;
    }
    idx += 1;
    let mut count = 0;
    while idx < lines.len() {
        if lines[idx].trim() == "```" {
            return Some(count);
        }
        count += 1;
        idx += 1;
    }
    None
}

fn find_file_evidence<'a>(
    coverage: &'a ReviewEvidenceCoverage,
    normalized: &str,
) -> Option<&'a ReviewFileEvidence> {
    let has_dir = normalized.contains('/');
    coverage.observed_files.iter().find(|entry| {
        entry.path == normalized || (has_dir && entry.path.ends_with(&format!("/{normalized}")))
    })
}

fn normalize_repo_relative_path(path: &str) -> Option<String> {
    let mut normalized = path.trim().replace('\\', "/");
    while let Some(rest) = normalized.strip_prefix("./") {
        normalized = rest.to_string();
    }
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.starts_with("//")
        || normalized.contains("://")
        || normalized.as_bytes().get(1) == Some(&b':')
    {
        return None;
    }
    let mut parts = Vec::new();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return None;
        }
        parts.push(part);
    }
    Some(parts.join("/"))
}

/// Helper: find the maximum new-file line number observed for a given file
/// across staged + unstaged diff hunks. Returns `None` if the file is not
/// present in any diff (so callers should not treat that as a failure).
fn max_line_for_file_in_target(target: &ReviewTarget, normalized_path: &str) -> Option<usize> {
    for diff in [&target.staged_diff, &target.unstaged_diff] {
        if let Some(max) = max_line_in_diff(diff, normalized_path) {
            return Some(max);
        }
    }
    None
}

fn max_line_in_diff(diff: &str, normalized_path: &str) -> Option<usize> {
    let mut in_target_file = false;
    let mut max_line: Option<usize> = None;
    let mut current_new_line: Option<usize> = None;
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git a/") {
            in_target_file = rest.starts_with(normalized_path)
                || rest.contains(&format!(" b/{normalized_path}"));
            current_new_line = None;
            continue;
        }
        if !in_target_file {
            continue;
        }
        if let Some(rest) = line.strip_prefix("@@ ") {
            // R4: reset before parsing each hunk header to avoid stale carry-over.
            current_new_line = None;
            if let Some(plus_idx) = rest.find('+') {
                let after = &rest[plus_idx + 1..];
                let end = after.find(' ').unwrap_or(after.len());
                let spec = &after[..end];
                let mut parts = spec.split(',');
                if let Some(s) = parts.next() {
                    if let Ok(start) = s.parse::<usize>() {
                        let count: usize = parts.next().and_then(|s| s.parse().ok()).unwrap_or(1);
                        let last = start.saturating_add(count.saturating_sub(1));
                        max_line = Some(max_line.map_or(last, |m| m.max(last)));
                        current_new_line = Some(start);
                    }
                }
            }
        } else if line.starts_with('+') && !line.starts_with("+++") {
            if let Some(n) = current_new_line.as_mut() {
                max_line = Some(max_line.map_or(*n, |m| m.max(*n)));
                *n += 1;
            }
        } else if line.starts_with(' ') {
            if let Some(n) = current_new_line.as_mut() {
                *n += 1;
            }
        }
    }
    max_line
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
    /// Every label, in declaration order. Kept next to the enum so the
    /// contract conformance suite can pin it against the schema enums.
    pub const ALL_LABELS: [&str; 3] =
        ["structured", "fallback_raw_text", "parse_attempted_but_failed"];

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
    /// C20.6-A UX-B: parse error detail for auditability.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub parse_error: String,
    /// C20.6-A UX-B: whether a JSON repair was attempted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parse_repair: Option<String>,
}

impl ReviewReport {
    #[must_use]
    pub fn no_findings(raw_text: impl Into<String>) -> Self {
        Self {
            findings: Vec::new(),
            raw_text: raw_text.into(),
            parse_status: ReviewParseStatus::FallbackRawText,
            parse_error: String::new(),
            parse_repair: None,
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
                parse_error: String::new(),
                parse_repair: None,
            };
        }

        let Some(json_candidate) = extract_json_candidate(trimmed) else {
            // C20.5-A: if text looks like it contains findings, don't silently report 0.
            if trimmed.contains("\"findings\"") || trimmed.contains("\"findings\":") {
                return Self {
                    findings: Vec::new(),
                    raw_text,
                    parse_status: ReviewParseStatus::ParseAttemptedButFailed,
                    parse_error: String::from("no valid JSON candidate found in model output"),
                    parse_repair: None,
                };
            }
            return Self::no_findings(raw_text);
        };

        // C20.6-A R3: source-based repair tracking.
        let extraction_source = json_candidate.source;
        let json_candidate = json_candidate.text;
        let parsed = serde_json::from_str::<ReviewReportWire>(json_candidate)
            .map(|wire| wire.findings)
            .or_else(|_| serde_json::from_str::<Vec<ReviewFinding>>(json_candidate));

        match parsed {
            Ok(findings) => Self {
                findings: assign_finding_ids(findings),
                raw_text,
                parse_status: ReviewParseStatus::Structured,
                parse_error: String::new(),
                parse_repair: if extraction_source == ExtractionSource::Direct {
                    None
                } else {
                    Some(format!("{extraction_source:?}"))
                },
            },
            Err(e) => {
                // C20.5-A: parsing failed but candidate text was found.
                let parse_err = format!("serde parse error: {e}");
                if trimmed.contains("\"findings\"") || trimmed.contains("\"findings\":") {
                    return Self {
                        findings: Vec::new(),
                        raw_text,
                        parse_status: ReviewParseStatus::ParseAttemptedButFailed,
                        parse_error: parse_err,
                        parse_repair: if extraction_source == ExtractionSource::Direct {
                            None
                        } else {
                            Some(format!("{extraction_source:?}"))
                        },
                    };
                }
                let mut r = Self::no_findings(raw_text);
                r.parse_error = parse_err;
                r
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
struct SegoReviewArtifact {
    schema_version: u32,
    id: String,
    created_at_epoch_seconds: u64,
    /// C21: local trust metadata identifying the engine that produced this artifact.
    /// This is not a cryptographic signature or provenance attestation.
    #[serde(default)]
    reviewer: String,
    /// C21: Sego engine version that wrote the artifact. Legacy artifacts default to empty.
    #[serde(default)]
    engine_version: String,
    /// C21: review mode used to produce the artifact (for example, `model_code_review`).
    #[serde(default)]
    review_mode: String,
    /// Governed plugin path only: the provider Sego resolved for the invocation
    /// that produced this artifact. Absent when the writing path does not record
    /// a route. Sego's routing decision, not a provider-side attestation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolved_provider: Option<String>,
    /// Governed plugin path only: the model Sego sent the request for. Same caveat
    /// as `resolved_provider`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolved_model: Option<String>,
    /// Governed plugin path only: the endpoint Sego resolved for this invocation.
    /// Absent when the provider has no base-URL accessor; `identity_evidence_gap`
    /// then names that unobservable dimension explicitly instead of leaving the
    /// reader to infer it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolved_endpoint: Option<String>,
    /// Governed plugin path only: how strong this identity evidence is. Currently
    /// always `IDENTITY_EVIDENCE_SELF_REPORTED` — Sego's own routing decision,
    /// confirmed against the wire, but NOT a provider-side attestation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    identity_evidence: Option<String>,
    /// Governed plugin path only: a named dimension Sego cannot observe. Present so
    /// a consumer never has to infer an evidence gap from a missing field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    identity_evidence_gap: Option<String>,
    /// Caller-minted invocation identity, echoed back by the governed path so an
    /// execution can be tied to the artifact it produced. Opaque to Sego: it is
    /// never parsed, and never used as an execution idempotency key. Absent when
    /// the caller does not supply one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    invocation_id: Option<String>,
    /// Contract revision 3 (`SEG-ADR-004`): the class of destination the reviewed
    /// content actually reached. Written only by a path that observed the run; a
    /// missing field means the writing path does not observe at all, and reads as
    /// `unknown` - never as `none`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data_egress_class: Option<String>,
    /// Contract revision 3 (`SEG-ADR-004`): where the inference actually ran.
    /// Orthogonal to `data_egress_class`, and observed the same way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compute_boundary: Option<String>,
    /// Contract revision 3 (`SEG-ADR-004`): the declared ceiling and the observed
    /// usage, both or neither. See [`build_budget`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    budget: Option<ReviewBudget>,
    scope: String,
    diff_hash: String,
    finding_count: usize,
    highest_severity: Option<ReviewSeverity>,
    parse_status: ReviewParseStatus,
    /// C20.6-A R2: parse diagnostics persisted for auditability.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    parse_error: String,
    /// C20.6-A R2: whether JSON repair/extraction was used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parse_repair: Option<String>,
    git_status: String,
    findings: Vec<ReviewFinding>,
    raw_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    evidence_coverage: Option<ReviewEvidenceCoverage>,
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
    AcceptedRisk,
    FalsePositive,
    /// Legacy-compatible terminal status. Prefer `accepted_risk` or
    /// `false_positive` for new records so disposition remains auditable.
    Ignored,
}

impl ReviewFindingStatus {
    /// Every label, in declaration order. Kept next to the enum so the
    /// contract conformance suite can pin it against the schema enums.
    pub const ALL_LABELS: [&str; 6] =
        ["open", "acknowledged", "fixed", "accepted_risk", "false_positive", "ignored"];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Acknowledged => "acknowledged",
            Self::Fixed => "fixed",
            Self::AcceptedRisk => "accepted_risk",
            Self::FalsePositive => "false_positive",
            Self::Ignored => "ignored",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "open" => Ok(Self::Open),
            "acknowledged" | "ack" => Ok(Self::Acknowledged),
            "fixed" | "resolved" => Ok(Self::Fixed),
            "accepted_risk" | "accepted-risk" | "risk_accepted" | "accept-risk" => {
                Ok(Self::AcceptedRisk)
            }
            "false_positive" | "false-positive" | "fp" => Ok(Self::FalsePositive),
            "ignored" | "ignore" => Ok(Self::Ignored),
            other => Err(format!(
                "unsupported review finding status `{other}` (expected open, acknowledged, fixed, accepted_risk, false_positive, or ignored)"
            )),
        }
    }

    #[must_use]
    pub fn requires_note(self) -> bool {
        matches!(self, Self::AcceptedRisk | Self::FalsePositive | Self::Ignored)
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

/// Evidence strength for an invocation identity: Sego resolved the route itself
/// (deterministically, from the model name) and confirmed it against the wire.
/// It is **not** a provider-side attestation, and the only value Sego can produce
/// today; a future `provider_attested` value would be a contract extension, not a
/// silent addition.
pub const IDENTITY_EVIDENCE_SELF_REPORTED: &str = "self_reported_resolution";

/// Named evidence gap: the provider has no base-URL accessor yet, so the endpoint
/// Sego used cannot be reported without guessing (see PROV-M03). Recorded
/// explicitly rather than omitted.
pub const IDENTITY_GAP_NO_ENDPOINT_ACCESSOR: &str = "no_endpoint_accessor_for_provider";

// ---------------------------------------------------------------------------
// Contract revision 3 (`SEG-ADR-004`): where the data went, where the work ran,
// and under what budget.
//
// The two axes are orthogonal on purpose. Local inference that fetched a page is
// `local` compute with `provider_and_fetch` egress, and no single enum expresses
// that combination - which is why they are two fields and not one.
// ---------------------------------------------------------------------------

/// Nothing left this machine: either nothing was sent at all, or the provider that
/// received the content runs on this machine.
pub const DATA_EGRESS_NONE: &str = "none";
/// The reviewed content reached a model provider over a network.
pub const DATA_EGRESS_PROVIDER: &str = "provider";
/// A model provider was reached, and a web fetch left this machine as well.
pub const DATA_EGRESS_PROVIDER_AND_FETCH: &str = "provider_and_fetch";
/// The invocation was observed, but the destination could not be determined.
///
/// **Not** a synonym for [`DATA_EGRESS_NONE`]. The distinction is the hard
/// requirement of `SEG-ADR-004` section 3.4: reading `unknown` as `none` is how
/// "local-first" gets claimed for a review that never established it.
pub const DATA_EGRESS_UNKNOWN: &str = "unknown";

/// Inference ran on this machine.
pub const COMPUTE_BOUNDARY_LOCAL: &str = "local";
/// Inference ran at the provider.
pub const COMPUTE_BOUNDARY_REMOTE: &str = "remote";
/// The invocation was observed, but where the inference ran could not be
/// determined.
///
/// The contract deliberately has no `local_and_remote`. An earlier draft of
/// revision 3 declared one for a shape where part of the work runs on each side,
/// and it was removed before publication: nothing produced it, and the consuming
/// side refuses any value outside `local` / `remote`. A declared value with no
/// producer is the same over-claim the field pair exists to prevent, and adding
/// one later is a contract change rather than a quiet widening.
pub const COMPUTE_BOUNDARY_UNKNOWN: &str = "unknown";

/// Every `data_egress_class` label. Kept next to the constants so the contract
/// conformance suite can pin them against the schema enum **in both directions**,
/// the way the older enums already are: one side drifting alone is the failure
/// that check exists to catch.
pub const DATA_EGRESS_ALL_LABELS: [&str; 4] =
    [DATA_EGRESS_NONE, DATA_EGRESS_PROVIDER, DATA_EGRESS_PROVIDER_AND_FETCH, DATA_EGRESS_UNKNOWN];

/// Every `compute_boundary` label. Same pinning as [`DATA_EGRESS_ALL_LABELS`].
pub const COMPUTE_BOUNDARY_ALL_LABELS: [&str; 3] =
    [COMPUTE_BOUNDARY_LOCAL, COMPUTE_BOUNDARY_REMOTE, COMPUTE_BOUNDARY_UNKNOWN];

/// Tool name whose use means data left this machine for a third-party host.
///
/// The name lives in the `tools` crate, which this one does not depend on, so the
/// two are pinned from the outside by
/// `the_tool_name_the_artifact_classifies_by_is_a_registered_tool`. A rename on
/// either side would otherwise make the fetch count silently zero, which is the
/// direction that under-reports egress.
pub const WEB_FETCH_TOOL_NAME: &str = "WebFetch";

/// What one review invocation **observed about its own behaviour**.
///
/// `SEG-ADR-004` section 3.2 is the reason this is an input rather than a read of
/// the provider configuration: the values have to come from what the invocation
/// actually did. A config that names a local model while the request went to a
/// remote provider must not be recorded as `none`, and taking the values from the
/// run is the only way to guarantee that.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewEgressObservation {
    /// Model-provider requests this invocation actually opened.
    ///
    /// The conversation loop increments its iteration counter once per request,
    /// so this counts requests made, not requests intended.
    pub provider_calls: u64,
    /// Input tokens the provider reported, or `None` when it reported none. A
    /// zero here would be a claim; the contract has a value for the gap.
    pub input_tokens: Option<u64>,
    /// Names of the tools that returned during the invocation, in call order.
    /// Only the fetch classification reads this, and it is never persisted.
    pub tool_names: Vec<String>,
}

impl ReviewEgressObservation {
    /// Web fetches this invocation performed.
    ///
    /// A call that *failed* counts too. A failed fetch may still have reached the
    /// third-party host, and under-reporting egress is the direction that lets
    /// "local-first" be over-claimed, so the classification errs towards the wider
    /// class rather than towards the comfortable one.
    #[must_use]
    pub fn fetch_count(&self) -> usize {
        self.tool_names.iter().filter(|name| name.as_str() == WEB_FETCH_TOOL_NAME).count()
    }
}

/// Whether an endpoint a request was sent to is on this machine.
///
/// Deliberately strict: only a loopback IP literal or `localhost` counts. A name
/// such as `127.0.0.1.example.com` resolves wherever its owner says, and reading
/// it as loopback would under-report egress - the exact failure this
/// classification exists to prevent.
#[must_use]
fn endpoint_is_loopback(endpoint: &str) -> bool {
    let after_scheme = endpoint.split_once("://").map_or(endpoint, |(_, rest)| rest);
    let authority = after_scheme.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    let host = if let Some(rest) = host_port.strip_prefix('[') {
        match rest.split_once(']') {
            // A bracketed address has to be the whole host. `[::1].example.com` is a
            // name whose first label merely looks like the literal, and reading it as
            // loopback is the under-reporting this function exists to avoid.
            Some((literal, tail)) if tail.is_empty() || tail.starts_with(':') => literal,
            _ => "",
        }
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    match host.parse::<std::net::IpAddr>() {
        Ok(address) => address.is_loopback(),
        Err(_) => host.eq_ignore_ascii_case("localhost"),
    }
}

/// Classify where the reviewed content actually went (`SEG-ADR-004` section 3.3).
///
/// Orthogonal to [`compute_boundary_for`]: this answers "where did the data go",
/// that one answers "where did the work run".
#[must_use]
pub fn data_egress_class_for(
    observation: &ReviewEgressObservation,
    endpoint: Option<&str>,
) -> &'static str {
    if observation.fetch_count() > 0 {
        // A fetch leaves this machine by definition, and the model that decided to
        // make it was reached first.
        return DATA_EGRESS_PROVIDER_AND_FETCH;
    }
    if observation.provider_calls == 0 {
        // Nothing was sent anywhere. This is the one case where `none` is known
        // rather than assumed.
        return DATA_EGRESS_NONE;
    }
    match endpoint {
        Some(endpoint) if endpoint_is_loopback(endpoint) => DATA_EGRESS_NONE,
        Some(_) => DATA_EGRESS_PROVIDER,
        // The destination is not observable. `unknown` is the value the ADR sets
        // aside for this, and it is defined to be read as anything but `none`.
        None => DATA_EGRESS_UNKNOWN,
    }
}

/// Classify where the inference actually ran (`SEG-ADR-004` section 3.3).
///
/// Orthogonal to [`data_egress_class_for`]. Both return `unknown` rather than a
/// definite value when the endpoint is unobservable. The destination is not
/// widened to `provider` here: `unknown` already carries the "not none" reading
/// the ADR requires, and asserting `provider` would claim something unobserved.
#[must_use]
pub fn compute_boundary_for(
    observation: &ReviewEgressObservation,
    endpoint: Option<&str>,
) -> &'static str {
    if observation.provider_calls == 0 {
        return COMPUTE_BOUNDARY_LOCAL;
    }
    match endpoint {
        Some(endpoint) if endpoint_is_loopback(endpoint) => COMPUTE_BOUNDARY_LOCAL,
        Some(_) => COMPUTE_BOUNDARY_REMOTE,
        None => COMPUTE_BOUNDARY_UNKNOWN,
    }
}

/// The ceiling a caller declared for one review.
///
/// A caller may declare an intent ceiling. It may not declare an actual: the
/// engine is the only writer of what happened (`SEG-ADR-004` section 3.5).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewBudgetDeclaration {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_provider_calls: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fetches: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u64>,
}

impl ReviewBudgetDeclaration {
    /// Whether the caller declared a ceiling on any dimension.
    ///
    /// A declaration with no dimension is not a budget, and is treated as no
    /// declaration at all rather than as a ceiling of zero.
    #[must_use]
    pub fn declares_nothing(&self) -> bool {
        self.max_provider_calls.is_none()
            && self.max_fetches.is_none()
            && self.max_input_tokens.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ReviewBudgetActual {
    provider_calls: u64,
    fetches: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input_tokens: Option<u64>,
    limit_reached: bool,
    aborted: bool,
}

/// The `budget` object written into an artifact: a declared ceiling and the
/// observed usage, **both or neither**.
///
/// `SEG-ADR-004` section 3.3 requires both halves. A ceiling alone reads as "did
/// not exceed it", and an actual alone invites the question "was it ever limited?"
/// while giving the reader nothing to answer it with. A review under no declared
/// ceiling therefore carries no budget object and reads as unknown - which is the
/// truthful answer to that question: nobody said.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ReviewBudget {
    declared: ReviewBudgetDeclaration,
    actual: ReviewBudgetActual,
}

/// Whether the observed usage reached any dimension the caller declared.
#[must_use]
fn budget_limit_reached(
    declared: &ReviewBudgetDeclaration,
    observation: &ReviewEgressObservation,
) -> bool {
    let reached = |limit: Option<u64>, actual: u64| limit.is_some_and(|limit| actual >= limit);
    reached(declared.max_provider_calls, observation.provider_calls)
        || reached(declared.max_fetches, observation.fetch_count() as u64)
        || declared
            .max_input_tokens
            .zip(observation.input_tokens)
            .is_some_and(|(limit, actual)| actual >= limit)
}

/// Build the budget object, or nothing when the caller declared no ceiling.
///
/// `aborted` is false because nothing enforces a ceiling yet: a run cannot have
/// been stopped by a limit that no code applies. It is not a default standing in
/// for an unknown - it is what happened.
fn build_budget(
    declared: &ReviewBudgetDeclaration,
    observation: &ReviewEgressObservation,
) -> Option<ReviewBudget> {
    if declared.declares_nothing() {
        return None;
    }
    Some(ReviewBudget {
        declared: declared.clone(),
        actual: ReviewBudgetActual {
            provider_calls: observation.provider_calls,
            fetches: observation.fetch_count() as u64,
            input_tokens: observation.input_tokens,
            limit_reached: budget_limit_reached(declared, observation),
            aborted: false,
        },
    })
}

/// Runtime identity of one invocation, recorded by the governed plugin path.
///
/// Distinct from `reviewer` / `engine_version` / `review_mode`, which identify the
/// engine that wrote the artifact. These identify the route this invocation took:
/// Sego's routing decision derived from the model name — not a provider-side
/// attestation, and not proof of which model the provider actually executed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReviewInvocationIdentity {
    pub provider: Option<String>,
    pub model: Option<String>,
    /// Endpoint Sego resolved for this invocation, or `None` when the provider has
    /// no base-URL accessor and the value therefore cannot be reported without
    /// guessing. A missing value is an explicitly recorded evidence gap
    /// (see `IDENTITY_GAP_NO_ENDPOINT_ACCESSOR`), never a silently omitted field.
    pub resolved_endpoint: Option<String>,
    /// Caller-minted persistent identity for the invocation that produced this
    /// artifact (for example an `EgoPulse` verification/checker execution id).
    /// Opaque to Sego: it is never parsed or interpreted, and never used to derive
    /// execution idempotency.
    pub invocation_id: Option<String>,
}

/// Persist a review artifact without recording an invocation route.
///
/// Kept for existing callers (the CLI review paths and tests). The governed
/// plugin path uses [`persist_review_artifact_with_identity`] instead.
pub fn persist_review_artifact(
    workspace_root: &Path,
    target: &ReviewTarget,
    report: &ReviewReport,
) -> std::io::Result<PersistedReviewArtifact> {
    persist_review_artifact_with_identity(
        workspace_root,
        target,
        report,
        &ReviewInvocationIdentity::default(),
    )
}

/// Persist a review artifact, recording the provider and model this invocation
/// resolved.
pub fn persist_review_artifact_with_identity(
    workspace_root: &Path,
    target: &ReviewTarget,
    report: &ReviewReport,
    identity: &ReviewInvocationIdentity,
) -> std::io::Result<PersistedReviewArtifact> {
    persist_review_artifact_inner(workspace_root, target, report, identity, None)
}

/// Persist a review artifact from a run whose behaviour was observed.
///
/// The egress class and the compute boundary are derived from `observation` - the
/// run itself - and never from configuration (`SEG-ADR-004` section 3.2), which is
/// the whole point of the field pair. A caller that has no observation to pass
/// should use [`persist_review_artifact_with_identity`] instead: the fields are
/// then absent, which a consumer reads as `unknown` rather than as `none`.
pub fn persist_review_artifact_observed(
    workspace_root: &Path,
    target: &ReviewTarget,
    report: &ReviewReport,
    identity: &ReviewInvocationIdentity,
    observation: &ReviewEgressObservation,
    declared_budget: &ReviewBudgetDeclaration,
) -> std::io::Result<PersistedReviewArtifact> {
    persist_review_artifact_inner(
        workspace_root,
        target,
        report,
        identity,
        Some((observation, declared_budget)),
    )
}

fn persist_review_artifact_inner(
    workspace_root: &Path,
    target: &ReviewTarget,
    report: &ReviewReport,
    identity: &ReviewInvocationIdentity,
    observed: Option<(&ReviewEgressObservation, &ReviewBudgetDeclaration)>,
) -> std::io::Result<PersistedReviewArtifact> {
    let reviews_dir = workspace_root.join(".sego").join("reviews");
    fs::create_dir_all(&reviews_dir)?;

    let diff_hash = review_diff_hash(target);
    let created_at_epoch_seconds = current_epoch_seconds();
    let id = format!("review-{created_at_epoch_seconds}-{}", short_hash(&diff_hash));

    let artifact = SegoReviewArtifact {
        schema_version: 1,
        id: id.clone(),
        created_at_epoch_seconds,
        reviewer: "sego".to_string(),
        engine_version: env!("CARGO_PKG_VERSION").to_string(),
        review_mode: "model_code_review".to_string(),
        resolved_provider: identity.provider.clone(),
        resolved_model: identity.model.clone(),
        resolved_endpoint: identity.resolved_endpoint.clone(),
        identity_evidence: if identity.provider.is_some() || identity.model.is_some() {
            Some(IDENTITY_EVIDENCE_SELF_REPORTED.to_string())
        } else {
            None
        },
        identity_evidence_gap: if identity.resolved_endpoint.is_none()
            && (identity.provider.is_some() || identity.model.is_some())
        {
            Some(IDENTITY_GAP_NO_ENDPOINT_ACCESSOR.to_string())
        } else {
            None
        },
        invocation_id: identity.invocation_id.clone(),
        scope: target.scope.label().clone(),
        diff_hash: diff_hash.clone(),
        finding_count: report.findings.len(),
        highest_severity: report.highest_severity(),
        parse_status: report.parse_status,
        // C20.6-A R2: persist parse diagnostics in artifacts.
        parse_error: report.parse_error.clone(),
        parse_repair: report.parse_repair.clone(),
        git_status: target.git_status.clone(),
        findings: report.findings.clone(),
        raw_text: report.raw_text.clone(),
        evidence_coverage: Some(build_evidence_coverage(target)),
        // The endpoint is the one this invocation resolved, which is the address
        // the client dialled - not a preference read back from configuration. An
        // `unknown` here is the observation gap, written rather than omitted so a
        // consumer never has to infer it from a missing field.
        data_egress_class: observed.map(|(observation, _)| {
            data_egress_class_for(observation, identity.resolved_endpoint.as_deref()).to_string()
        }),
        compute_boundary: observed.map(|(observation, _)| {
            compute_boundary_for(observation, identity.resolved_endpoint.as_deref()).to_string()
        }),
        budget: observed.and_then(|(observation, declared)| build_budget(declared, observation)),
    };

    let json_path = reviews_dir.join(format!("{id}.json"));
    let markdown_path = reviews_dir.join(format!("{id}.md"));
    let index_path = reviews_dir.join("index.jsonl");

    // Artifact-identity conflict (EgoPulse ruling 2026-09-12, option (c)): the id is
    // `review-<epoch-second>-<diff-prefix>`, so two reviews of the same diff finishing
    // in the same second compute the same id and therefore the same path. Writing with
    // `create_new` refuses to replace an existing artifact instead of silently
    // destroying earlier evidence, and does so atomically — two processes racing for
    // the same id cannot both believe they won. There is deliberately NO retry with a
    // fresh id: that would turn a detected conflict into a silent identity change.
    write_new_file(
        &json_path,
        &(serde_json::to_string_pretty(&artifact)? + "\n"),
        &artifact,
        workspace_root,
    )?;
    write_new_file(
        &markdown_path,
        &render_review_markdown(&artifact, report),
        &artifact,
        workspace_root,
    )?;
    append_review_index(&index_path, &artifact, &json_path, &markdown_path)?;

    Ok(PersistedReviewArtifact { id, diff_hash, json_path, markdown_path, index_path })
}

/// Write a brand-new artifact file, refusing to replace an existing one.
///
/// The refusal is atomic (`create_new`), not a check followed by a write, so two
/// concurrent reviews computing the same artifact id cannot both proceed. The error
/// names the conflicting id together with the `(workspace, diff, scope)` identity so
/// a caller can locate the existing artifact instead of guessing.
fn write_new_file(
    path: &Path,
    contents: &str,
    artifact: &SegoReviewArtifact,
    workspace_root: &Path,
) -> std::io::Result<()> {
    use std::io::Write as _;

    let mut file =
        fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!(
                        "artifact id conflict: id '{}' already exists at {} for workspace {} \
                         (scope '{}', diff_hash '{}'). Refusing to overwrite an existing artifact; \
                         Sego does not retry with a different id. Serialize reviews for the same \
                         (workspace, diff) or move the existing artifact aside.",
                        artifact.id,
                        path.display(),
                        workspace_root.display(),
                        artifact.scope,
                        artifact.diff_hash,
                    ),
                )
            } else {
                error
            }
        })?;
    file.write_all(contents.as_bytes())
}

pub fn load_review_index(workspace_root: &Path) -> io::Result<Vec<ReviewIndexEntry>> {
    let index_path = workspace_root.join(".sego").join("reviews").join("index.jsonl");
    if !index_path.exists() {
        return Ok(Vec::new());
    }

    let contents = std::fs::read_to_string(index_path)?;
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

    let note = note.map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
    if status.requires_note() && note.is_none() {
        return Err(io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "review finding status `{}` requires a non-empty disposition note",
                status.label()
            ),
        ));
    }

    let entry = ReviewFindingStatusEntry {
        report_id: report_id.to_string(),
        finding_id: finding_id.to_string(),
        status,
        note,
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

    let contents = std::fs::read_to_string(status_path)?;
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

// A single markdown document assembled section by section; the length is the
// report format, and splitting it would put the shape of the output in two
// places.
#[allow(clippy::too_many_lines)]
fn render_review_markdown(artifact: &SegoReviewArtifact, report: &ReviewReport) -> String {
    let mut output = String::new();
    output.push_str("# Sego Review Report\n\n");
    let _ = writeln!(output, "- ID: `{}`", artifact.id);
    let _ = writeln!(output, "- Scope: `{}`", artifact.scope);
    if !artifact.reviewer.is_empty() {
        let _ = writeln!(output, "- Reviewer: `{}`", artifact.reviewer);
    }
    if !artifact.engine_version.is_empty() {
        let _ = writeln!(output, "- Engine version: `{}`", artifact.engine_version);
    }
    if !artifact.review_mode.is_empty() {
        let _ = writeln!(output, "- Review mode: `{}`", artifact.review_mode);
    }
    let _ = writeln!(output, "- Diff hash: `{}`", artifact.diff_hash);
    let _ = writeln!(output, "- Findings: `{}`", artifact.finding_count);
    let _ = writeln!(output, "- Parse status: `{}`", artifact.parse_status.label());
    // C20.6-A R2: render parse diagnostics in Markdown.
    if !artifact.parse_error.is_empty() {
        let _ = writeln!(output, "- Parse error: `{}`", artifact.parse_error);
        if let Some(ref repair) = artifact.parse_repair {
            let _ = writeln!(output, "- Parse repair: `{repair}`");
        }
    }
    if let Some(severity) = artifact.highest_severity {
        let _ = writeln!(output, "- Highest severity: `{}`", severity.label());
    }
    let _ =
        writeln!(output, "- Created at epoch seconds: `{}`\n", artifact.created_at_epoch_seconds);

    if let Some(coverage) = &artifact.evidence_coverage {
        output.push_str("## Evidence Coverage\n\n");
        let _ = writeln!(output, "- Scope kind: `{:?}`", coverage.scope_kind);
        let _ = writeln!(output, "- File tree count: `{}`", coverage.file_tree_count);
        let _ = writeln!(output, "- Captured files: `{}`", coverage.captured_file_count);
        let _ = writeln!(output, "- Truncated files: `{}`", coverage.truncated_file_count);
        let _ = writeln!(output, "- Not captured files: `{}`", coverage.not_captured_file_count);
        let _ = writeln!(output, "- Skipped large files: `{}`", coverage.skipped_large_file_count);
        let _ = writeln!(output, "- Unreadable files: `{}`\n", coverage.unreadable_file_count);
    }

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
            if let Some(status) = finding.evidence_status {
                let _ = writeln!(output, "- Evidence status: `{}`", status.label());
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
    artifact: &SegoReviewArtifact,
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

/// C20.6-A R3: tracks where a JSON candidate was extracted from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExtractionSource {
    Direct,
    Fenced,
    Prose,
}

struct ExtractedJson<'a> {
    text: &'a str,
    source: ExtractionSource,
}

fn extract_json_candidate(text: &str) -> Option<ExtractedJson<'_>> {
    let trimmed = text.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        return Some(ExtractedJson { text: trimmed, source: ExtractionSource::Direct });
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
                    return Some(ExtractedJson {
                        text: content[..content_offset].trim(),
                        source: ExtractionSource::Fenced,
                    });
                }
                content_offset += content_line.len();
            }
            return None;
        }
        offset += line.len();
    }

    // C20.5-A: fallback — search for a JSON object containing "findings" in prose text.
    extract_json_from_prose(trimmed)
        .map(|text| ExtractedJson { text, source: ExtractionSource::Prose })
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
        build_budget, build_evidence_coverage, compute_boundary_for, data_egress_class_for,
        evaluate_evidence_gate, latest_review_finding_statuses, load_review_finding_statuses,
        load_review_index, persist_review_artifact, persist_review_artifact_observed,
        persist_review_artifact_with_identity, record_review_finding_status, review_diff_hash,
        EvidenceStatus, ReviewBudgetDeclaration, ReviewContentStatus, ReviewEgressObservation,
        ReviewEvidenceScopeKind, ReviewFinding, ReviewFindingStatus, ReviewFindingStatusEntry,
        ReviewIndexEntry, ReviewInvocationIdentity, ReviewParseStatus, ReviewReport,
        SegoReviewArtifact, COMPUTE_BOUNDARY_LOCAL, COMPUTE_BOUNDARY_REMOTE,
        COMPUTE_BOUNDARY_UNKNOWN, DATA_EGRESS_NONE, DATA_EGRESS_PROVIDER,
        DATA_EGRESS_PROVIDER_AND_FETCH, DATA_EGRESS_UNKNOWN, IDENTITY_EVIDENCE_SELF_REPORTED,
        IDENTITY_GAP_NO_ENDPOINT_ACCESSOR,
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
    // The float comparison in here is exact on purpose: the value is written and
    // read back through the artifact, so any drift is the bug this test exists to
    // catch, and an epsilon would hide exactly that.
    #[allow(clippy::float_cmp)]
    fn review_artifact_round_trips_through_json() {
        // Golden fixture: construct a SegoReviewArtifact with a finding, serialize,
        // deserialize, and verify every field survives the round-trip.
        let artifact = SegoReviewArtifact {
            schema_version: 1,
            id: "rev-test-001".to_string(),
            created_at_epoch_seconds: 1_719_849_600,
            reviewer: "sego".to_string(),
            engine_version: "0.1.9-test".to_string(),
            review_mode: "model_code_review".to_string(),
            resolved_provider: Some("anthropic".to_string()),
            resolved_model: Some("claude-sonnet-4-6".to_string()),
            resolved_endpoint: Some("https://api.anthropic.com".to_string()),
            identity_evidence: Some(IDENTITY_EVIDENCE_SELF_REPORTED.to_string()),
            identity_evidence_gap: None,
            invocation_id: Some("inv-test-001".to_string()),
            // Revision 3 fields carried through the golden round-trip too, so a
            // later rename of a JSON key is caught here rather than by a consumer.
            data_egress_class: Some(DATA_EGRESS_PROVIDER.to_string()),
            compute_boundary: Some(COMPUTE_BOUNDARY_REMOTE.to_string()),
            budget: None,
            scope: "staged".to_string(),
            diff_hash: "sha256:abc123".to_string(),
            finding_count: 1,
            highest_severity: Some(ReviewSeverity::High),
            parse_status: ReviewParseStatus::Structured,
            parse_error: String::new(),
            parse_repair: None,
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
                evidence_status: None,
            }],
            raw_text: "raw model output".to_string(),
            evidence_coverage: None,
        };

        let json = serde_json::to_string(&artifact).expect("serialize artifact");
        let parsed: SegoReviewArtifact = serde_json::from_str(&json).expect("deserialize artifact");

        // Field-by-field round-trip verification.
        assert_eq!(parsed.schema_version, 1);
        assert_eq!(parsed.id, "rev-test-001");
        assert_eq!(parsed.reviewer, "sego");
        assert_eq!(parsed.engine_version, "0.1.9-test");
        assert_eq!(parsed.review_mode, "model_code_review");
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

        assert!(json.contains(r#""reviewer":"sego""#), "reviewer must be serialized: {json}");
        assert!(
            json.contains(r#""review_mode":"model_code_review""#),
            "review_mode must be serialized: {json}"
        );

        // schema_version must be present in JSON (schema required field).
        assert!(json.contains(r#""schema_version":1"#), "schema_version must be in JSON: {json}");

        // finding_count must equal findings length (schema consistency).
        assert_eq!(parsed.finding_count, parsed.findings.len());
    }

    #[test]
    fn review_artifact_with_no_findings_round_trips() {
        let artifact = SegoReviewArtifact {
            schema_version: 1,
            id: "rev-test-002".to_string(),
            created_at_epoch_seconds: 1_719_849_600,
            reviewer: "sego".to_string(),
            engine_version: "0.1.9-test".to_string(),
            review_mode: "model_code_review".to_string(),
            resolved_provider: None,
            resolved_model: None,
            resolved_endpoint: None,
            identity_evidence: None,
            identity_evidence_gap: None,
            invocation_id: None,
            // The writing path that does not observe at all: all three are absent,
            // which is the case a consumer must read as `unknown`, not as `none`.
            data_egress_class: None,
            compute_boundary: None,
            budget: None,
            scope: "workspace".to_string(),
            diff_hash: "sha256:none".to_string(),
            finding_count: 0,
            highest_severity: None,
            parse_status: ReviewParseStatus::FallbackRawText,
            parse_error: String::new(),
            parse_repair: None,
            git_status: String::new(),
            findings: vec![],
            raw_text: "No findings.".to_string(),
            evidence_coverage: None,
        };

        let json = serde_json::to_string(&artifact).expect("serialize");
        let parsed: SegoReviewArtifact = serde_json::from_str(&json).expect("deserialize");

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
    fn nonexistent_finding_file_marked_unverified_file() {
        // UX-D: if a finding references a file not in scope, mark unverified_file.
        let report = ReviewReport::from_model_output(
            r#"{"findings":[{"severity":"low","file":"nonexistent.rs","line":1,"title":"Missing","evidence":"none","risk":"none","suggestion":"none","confidence":0.1}]}"#,
        );
        assert_eq!(report.parse_status, ReviewParseStatus::Structured);
        let findings = evaluate_evidence_gate(
            report.findings,
            &target_with_diff(
                "diff --git a/src/main.rs b/src/main.rs\n@@ -1,3 +1,4 @@\n old\n+new\n",
            ),
        );
        assert_eq!(findings[0].evidence_status, Some(EvidenceStatus::UnverifiedFile));
    }

    #[test]
    fn verified_finding_remains_verified() {
        // UX-D: a finding referencing an in-scope file with plausible line stays verified.
        let report = ReviewReport::from_model_output(
            r#"{"findings":[{"severity":"low","file":"src/main.rs","line":4,"title":"OK","evidence":"seen","risk":"none","suggestion":"none","confidence":0.5}]}"#,
        );
        let findings = evaluate_evidence_gate(
            report.findings,
            &target_with_diff(
                "diff --git a/src/main.rs b/src/main.rs\n@@ -1,3 +1,4 @@\n old\n+new\n",
            ),
        );
        assert_eq!(findings[0].evidence_status, Some(EvidenceStatus::Verified));
    }

    #[test]
    fn line_beyond_file_length_marked_unverified_line() {
        let report = ReviewReport::from_model_output(
            r#"{"findings":[{"severity":"low","file":"src/main.rs","line":999,"title":"Bad line","evidence":"none","risk":"none","suggestion":"none","confidence":0.1}]}"#,
        );
        let target = target_with_diff(
            "diff --git a/src/main.rs b/src/main.rs\n@@ -1,5 +1,6 @@\n old\n+new\n",
        );
        let findings = evaluate_evidence_gate(report.findings, &target);
        assert_eq!(findings[0].evidence_status, Some(EvidenceStatus::UnverifiedLine));
    }

    #[test]
    fn a_second_persist_with_the_same_id_must_not_overwrite() {
        // The id is `review-<epoch-second>-<diff-prefix>`, so within one second two
        // persists of the same diff compute the same id. Whatever the timing, the
        // invariant is: an existing artifact id can never be written twice.
        let report = ReviewReport::from_model_output("{\"findings\": []}");
        let root = temp_path("review-id-conflict");
        let _ = std::fs::create_dir_all(&root);
        let target = target_with_diff(
            "diff --git a/src/lib.rs b/src/lib.rs
",
        );

        let first = persist_review_artifact(&root, &target, &report).expect("first persist");
        let first_json = std::fs::read_to_string(&first.json_path).expect("first json");

        match persist_review_artifact(&root, &target, &report) {
            Ok(second) => assert_ne!(
                second.id, first.id,
                "a second persist must never reuse an existing artifact id"
            ),
            Err(error) => {
                assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
                assert!(
                    error.to_string().contains(&first.id),
                    "the conflict error must name the conflicting id: {error}"
                );
                assert_eq!(
                    std::fs::read_to_string(&first.json_path).expect("first json"),
                    first_json,
                    "the earlier artifact must survive a detected conflict intact"
                );
            }
        }
    }

    #[test]
    fn invocation_identity_is_recorded_only_when_supplied() {
        // Contract guarantee downstream consumers rely on: the persisted artifact
        // shape is unchanged unless the governed path supplies an invocation
        // identity. Parsed structurally rather than by substring, so the result
        // cannot depend on the serializer's whitespace.
        let report = ReviewReport::from_model_output("{\"findings\": []}");
        // One root per case on purpose: the artifact id is derived from the epoch
        // second plus the diff prefix, so three persists of the same diff inside one
        // second would collide — which is now a hard error by design.
        let root_plain = temp_path("review-identity-plain");
        let root_governed = temp_path("review-identity-governed");
        let root_gapped = temp_path("review-identity-gapped");
        for root in [&root_plain, &root_governed, &root_gapped] {
            let _ = std::fs::create_dir_all(root);
        }
        let target = target_with_diff(
            "diff --git a/src/lib.rs b/src/lib.rs
",
        );

        let plain = persist_review_artifact(&root_plain, &target, &report).expect("persist plain");
        let plain_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&plain.json_path).expect("plain json"))
                .expect("plain artifact is JSON");
        assert!(
            plain_json.get("resolved_provider").is_none(),
            "a path that does not record a route must omit resolved_provider, got {plain_json}"
        );
        assert!(plain_json.get("resolved_model").is_none());
        assert!(plain_json.get("invocation_id").is_none());
        // The new identity-evidence fields must not leak into non-governed artifacts
        // either: an absent field is fine, a claimed one would be a lie.
        assert!(plain_json.get("resolved_endpoint").is_none());
        assert!(plain_json.get("identity_evidence").is_none());
        assert!(plain_json.get("identity_evidence_gap").is_none());

        let identity = ReviewInvocationIdentity {
            provider: Some("anthropic".to_string()),
            model: Some("claude-sonnet-4-6".to_string()),
            resolved_endpoint: Some("https://api.anthropic.com".to_string()),
            invocation_id: Some("inv-abc-123".to_string()),
        };
        let governed =
            persist_review_artifact_with_identity(&root_governed, &target, &report, &identity)
                .expect("persist governed");
        let governed_json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&governed.json_path).expect("governed json"),
        )
        .expect("governed artifact is JSON");
        assert_eq!(
            governed_json.get("resolved_provider").and_then(serde_json::Value::as_str),
            Some("anthropic")
        );
        assert_eq!(
            governed_json.get("resolved_model").and_then(serde_json::Value::as_str),
            Some("claude-sonnet-4-6")
        );
        assert_eq!(
            governed_json.get("invocation_id").and_then(serde_json::Value::as_str),
            Some("inv-abc-123"),
            "a caller-minted invocation id must be echoed into the artifact verbatim"
        );

        // A provider without a base-URL accessor: the endpoint is absent AND the gap
        // is named, so a consumer never has to infer a missing observation from
        // silence.
        let gap_identity = ReviewInvocationIdentity {
            provider: Some("openai".to_string()),
            model: Some("gpt-4o".to_string()),
            resolved_endpoint: None,
            invocation_id: None,
        };
        let gapped =
            persist_review_artifact_with_identity(&root_gapped, &target, &report, &gap_identity)
                .expect("persist gapped");
        let gapped_json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&gapped.json_path).expect("gapped json"))
                .expect("gapped artifact is JSON");
        assert!(gapped_json.get("resolved_endpoint").is_none());
        assert_eq!(
            gapped_json.get("identity_evidence_gap").and_then(serde_json::Value::as_str),
            Some(IDENTITY_GAP_NO_ENDPOINT_ACCESSOR),
            "an unobservable endpoint must be named as a gap, not silently omitted"
        );
    }

    #[test]
    fn parse_failed_artifact_persists_diagnostics() {
        // R3: parse-failed review must persist parse_error in JSON + Markdown artifacts.
        let report = ReviewReport::from_model_output(
            "{\"findings\": [{\"severity\": \"bad\", \"title\": \"unclosed string}",
        );
        assert_eq!(report.parse_status, ReviewParseStatus::ParseAttemptedButFailed);
        assert!(!report.parse_error.is_empty());

        let root = temp_path("review-parse-failed");
        let _ = std::fs::create_dir_all(&root);
        let target = target_with_diff(
            "diff --git a/src/lib.rs b/src/lib.rs
",
        );
        let artifact = persist_review_artifact(&root, &target, &report).expect("persist");

        // JSON artifact must contain diagnostics.
        let json = std::fs::read_to_string(&artifact.json_path).expect("json");
        assert!(json.contains("parse_attempted_but_failed"));
        assert!(json.contains("parse_error"));

        // Markdown artifact must contain diagnostics.
        let md = std::fs::read_to_string(&artifact.markdown_path).expect("md");
        assert!(md.contains("Parse status"));
        assert!(md.contains("Parse error"));

        // Index still writes.
        let index = std::fs::read_to_string(&artifact.index_path).expect("index");
        assert!(index.contains("parse_attempted_but_failed"));

        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn parse_success_repair_marker_absent_for_direct_json() {
        let report = ReviewReport::from_model_output(
            r#"{"findings":[{"severity":"low","file":"a.rs","line":1,"title":"ok","evidence":"e","risk":"r","suggestion":"s","confidence":0.5}]}"#,
        );
        assert_eq!(report.parse_status, ReviewParseStatus::Structured);
        assert!(report.parse_repair.is_none(), "Direct JSON must have no repair marker");
    }

    #[test]
    fn parse_success_repair_marker_present_for_fenced_json() {
        let report = ReviewReport::from_model_output(
            "```json\n{\"findings\":[{\"severity\":\"low\",\"file\":\"a.rs\",\"line\":1,\"title\":\"fenced\",\"evidence\":\"e\",\"risk\":\"r\",\"suggestion\":\"s\",\"confidence\":0.5}]}\n```",
        );
        assert_eq!(report.parse_status, ReviewParseStatus::Structured);
        assert!(report.parse_repair.is_some(), "Fenced JSON must have repair marker");
    }

    #[test]
    fn parse_success_repair_marker_present_for_prose_json() {
        let report = ReviewReport::from_model_output(
            "Here is output:\n{\"findings\":[{\"severity\":\"low\",\"file\":\"a.rs\",\"line\":1,\"title\":\"prose\",\"evidence\":\"e\",\"risk\":\"r\",\"suggestion\":\"s\",\"confidence\":0.5}]}\nEnd.",
        );
        assert_eq!(report.parse_status, ReviewParseStatus::Structured);
        assert!(report.parse_repair.is_some(), "Prose JSON must have repair marker");
    }

    #[test]
    fn evidence_status_persisted_in_artifact_json_and_markdown() {
        // R2: artifact JSON persists evidence_status; Markdown renders it.
        let root = temp_path("evidence-artifact");
        let _ = std::fs::create_dir_all(&root);
        let target = target_with_diff(
            "diff --git a/src/main.rs b/src/main.rs\n@@ -1,3 +1,4 @@\n old\n+new\n",
        );
        let report = ReviewReport::from_model_output(
            r#"{"findings":[{"severity":"low","file":"src/main.rs","line":2,"title":"OK","evidence":"e","risk":"r","suggestion":"s","confidence":0.5}]}"#,
        );
        let findings = evaluate_evidence_gate(report.findings, &target);
        let report = ReviewReport { findings, ..report };
        let artifact = persist_review_artifact(&root, &target, &report).expect("persist");

        let json = std::fs::read_to_string(&artifact.json_path).expect("read json");
        assert!(json.contains("\"evidence_status\""));
        assert!(json.contains("\"verified\""));

        let md = std::fs::read_to_string(&artifact.markdown_path).expect("read md");
        assert!(md.contains("Evidence status:"));
        assert!(md.contains("`verified`"));

        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn old_artifact_without_evidence_status_still_loads() {
        // R2: backward compatibility — JSON missing evidence_status must parse via serde(default).
        let json = r#"{
            "schema_version": 1,
            "id": "review-old",
            "created_at_epoch_seconds": 1700000000,
            "scope": "staged",
            "diff_hash": "abc",
            "finding_count": 1,
            "highest_severity": "low",
            "parse_status": "structured",
            "git_status": "",
            "findings": [{
                "id": "f-1",
                "severity": "low",
                "file": "src/lib.rs",
                "line": 1,
                "title": "Old",
                "evidence": "e",
                "risk": "r",
                "suggestion": "s",
                "confidence": 0.5,
                "verification_hint": null
            }],
            "raw_text": ""
        }"#;
        let parsed: SegoReviewArtifact =
            serde_json::from_str(json).expect("deserialize old artifact");
        assert_eq!(parsed.findings.len(), 1);
        assert!(parsed.findings[0].evidence_status.is_none());
        assert!(parsed.evidence_coverage.is_none());
    }

    #[test]
    fn bare_filename_does_not_verify_nested_scope_file() {
        // R4: scope has src/foo/bar.rs, finding cites bar.rs — must NOT be Verified.
        let target =
            target_with_diff("diff --git a/foo/bar.rs b/foo/bar.rs\n@@ -1,3 +1,4 @@\n old\n+new\n");
        let report = ReviewReport::from_model_output(
            r#"{"findings":[{"severity":"low","file":"bar.rs","line":2,"title":"Bare match","evidence":"e","risk":"r","suggestion":"s","confidence":0.1}]}"#,
        );
        let findings = evaluate_evidence_gate(report.findings, &target);
        assert_eq!(
            findings[0].evidence_status,
            Some(EvidenceStatus::UnverifiedFile),
            "bare bar.rs must not verify against src/foo/bar.rs"
        );
    }

    #[test]
    fn relative_path_suffix_still_verifies_when_path_has_directory() {
        // R4: scope has src/foo/bar.rs, finding cites foo/bar.rs — must be Verified.
        let target =
            target_with_diff("diff --git a/foo/bar.rs b/foo/bar.rs\n@@ -1,3 +1,4 @@\n old\n+new\n");
        let report = ReviewReport::from_model_output(
            r#"{"findings":[{"severity":"low","file":"foo/bar.rs","line":1,"title":"Good match","evidence":"e","risk":"r","suggestion":"s","confidence":0.1}]}"#,
        );
        let findings = evaluate_evidence_gate(report.findings, &target);
        assert_eq!(findings[0].evidence_status, Some(EvidenceStatus::Verified));
    }

    #[test]
    fn malformed_middle_hunk_does_not_carry_previous_line_state() {
        // R7: the malformed middle hunk contains enough +leak lines to push
        // stale current_new_line above 13 if it were NOT reset. With proper
        // reset before each hunk header parse, the leak lines are ignored
        // and only the valid third hunk (max=12) counts -> line 13 stays
        // UnverifiedLine.
        let target = target_with_diff(
            "diff --git a/src/main.rs b/src/main.rs\n@@ -1,2 +1,2 @@\n old\n+new\n@@ -5,2 +abc,2 @@\n+leak03\n+leak04\n+leak05\n+leak06\n+leak07\n+leak08\n+leak09\n+leak10\n+leak11\n+leak12\n+leak13\n@@ -10,3 +10,3 @@\n old\n+line11\n+line12\n",
        );
        let report = ReviewReport::from_model_output(
            r#"{"findings":[{"severity":"low","file":"src/main.rs","line":13,"title":"Line 13","evidence":"e","risk":"r","suggestion":"s","confidence":0.1}]}"#,
        );
        let findings = evaluate_evidence_gate(report.findings, &target);
        // Without reset, stale current_new_line counting the leak lines
        // could improperly verify line 13. With reset, only the third hunk
        // contributes: max=12 -> line 13 is UnverifiedLine.
        assert_eq!(
            findings[0].evidence_status,
            Some(EvidenceStatus::UnverifiedLine),
            "stale current_new_line carry-over must be prevented by hunk-header reset"
        );
    }

    #[test]
    fn reverse_path_suffix_does_not_verify_unrelated_files() {
        // R3: scope has bar.rs, finding cites foo/bar.rs — must NOT be Verified.
        let target =
            target_with_diff("diff --git a/src/bar.rs b/src/bar.rs\n@@ -1,3 +1,4 @@\n old\n+new\n");
        let report = ReviewReport::from_model_output(
            r#"{"findings":[{"severity":"low","file":"foo/bar.rs","line":2,"title":"Bad match","evidence":"e","risk":"r","suggestion":"s","confidence":0.1}]}"#,
        );
        let findings = evaluate_evidence_gate(report.findings, &target);
        assert_eq!(
            findings[0].evidence_status,
            Some(EvidenceStatus::UnverifiedFile),
            "reverse suffix must not verify unrelated paths"
        );
    }

    #[test]
    fn full_tree_file_with_line_but_no_content_marks_content_not_captured() {
        // EC-03/EC-08: file in full_tree only, no key-file content => content_not_captured.
        let mut target = target_with_diff("");
        target.full_tree = "## File tree\nsrc/lib.rs\nsrc/main.rs\n\n## Key files\n### Cargo.toml\n```\n...\n```\n"
            .to_string();
        let report = ReviewReport::from_model_output(
            r#"{"findings":[{"severity":"low","file":"src/lib.rs","line":999999,"title":"Unknown line","evidence":"e","risk":"r","suggestion":"s","confidence":0.1}]}"#,
        );
        let findings = evaluate_evidence_gate(report.findings, &target);
        assert_eq!(
            findings[0].evidence_status,
            Some(EvidenceStatus::ContentNotCaptured),
            "full_tree listed-only files must not be treated as verified content"
        );
    }

    #[test]
    fn dependency_finding_without_manifest_marked_unverified_dependency() {
        // R2: dependency-themed finding gets UnverifiedDependency when no manifest in scope.
        let target = target_with_diff(
            "diff --git a/src/main.rs b/src/main.rs\n@@ -1,3 +1,4 @@\n old\n+new\n",
        );
        let report = ReviewReport::from_model_output(
            r#"{"findings":[{"severity":"high","file":"src/main.rs","line":1,"title":"Outdated dependency","evidence":"transitive CVE","risk":"vulnerable lockfile","suggestion":"bump dependency","confidence":0.7}]}"#,
        );
        let findings = evaluate_evidence_gate(report.findings, &target);
        assert_eq!(findings[0].evidence_status, Some(EvidenceStatus::UnverifiedDependency));
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
        assert_eq!(
            ReviewFindingStatus::parse("accepted-risk"),
            Ok(ReviewFindingStatus::AcceptedRisk)
        );
        assert_eq!(
            ReviewFindingStatus::parse("false_positive"),
            Ok(ReviewFindingStatus::FalsePositive)
        );
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
            Some("legacy ignored with explicit reason".to_string()),
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

    #[test]
    fn terminal_disposition_statuses_require_notes_when_recorded() {
        let root = temp_path("review-finding-status-note-required");

        let accepted_risk = record_review_finding_status(
            &root,
            "review-1",
            "finding-1",
            ReviewFindingStatus::AcceptedRisk,
            None,
        )
        .expect_err("accepted risk without note should fail");
        assert_eq!(accepted_risk.kind(), std::io::ErrorKind::InvalidInput);

        let false_positive = record_review_finding_status(
            &root,
            "review-1",
            "finding-1",
            ReviewFindingStatus::FalsePositive,
            Some("   ".to_string()),
        )
        .expect_err("blank false-positive note should fail");
        assert_eq!(false_positive.kind(), std::io::ErrorKind::InvalidInput);

        let entry = record_review_finding_status(
            &root,
            "review-1",
            "finding-1",
            ReviewFindingStatus::AcceptedRisk,
            Some("Founder accepts risk for local-only prototype".to_string()),
        )
        .expect("accepted risk with note should record");
        assert_eq!(entry.status, ReviewFindingStatus::AcceptedRisk);
        assert_eq!(entry.note.as_deref(), Some("Founder accepts risk for local-only prototype"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn ec_01_diff_scope_produces_full_evidence_for_diff_paths() {
        let target =
            target_with_diff("diff --git a/src/lib.rs b/src/lib.rs\n@@ -1,1 +1,2 @@\n old\n+new\n");
        let coverage = build_evidence_coverage(&target);

        assert_eq!(coverage.scope_kind, ReviewEvidenceScopeKind::Diff);
        assert_eq!(coverage.file_tree_count, 1);
        assert_eq!(coverage.captured_file_count, 1);
        assert_eq!(coverage.observed_files[0].path, "src/lib.rs");
        assert_eq!(coverage.observed_files[0].content_status, ReviewContentStatus::Full);
    }

    #[test]
    fn ec_02_full_repo_listed_and_captured_file_is_full() {
        let target = target_full_repo(
            "## File tree\nsrc/lib.rs\nREADME.md\n\n## Key files\n\n### src/lib.rs [full]\n```\nfn a() {}\nfn b() {}\n```\n",
        );
        let coverage = build_evidence_coverage(&target);
        let lib = coverage
            .observed_files
            .iter()
            .find(|entry| entry.path == "src/lib.rs")
            .expect("lib evidence");

        assert_eq!(coverage.scope_kind, ReviewEvidenceScopeKind::FullRepo);
        assert_eq!(coverage.file_tree_count, 2);
        assert_eq!(coverage.captured_file_count, 1);
        assert_eq!(lib.content_status, ReviewContentStatus::Full);
        assert_eq!(lib.captured_line_count, Some(2));
    }

    #[test]
    fn ec_03_full_repo_listed_without_key_content_is_not_captured() {
        let target = target_full_repo(
            "## File tree\nsrc/lib.rs\nsrc/not_sampled.rs\n\n## Key files\n\n### src/lib.rs [full]\n```\nfn a() {}\n```\n",
        );
        let coverage = build_evidence_coverage(&target);
        let skipped = coverage
            .observed_files
            .iter()
            .find(|entry| entry.path == "src/not_sampled.rs")
            .expect("not sampled evidence");

        assert_eq!(skipped.content_status, ReviewContentStatus::NotCaptured);
        assert_eq!(coverage.not_captured_file_count, 1);
    }

    #[test]
    fn ec_04_full_repo_marked_truncation_is_counted() {
        let target = target_full_repo(
            "## File tree\nsrc/lib.rs\n\n## Key files\n\n### src/lib.rs [truncated]\n```\nfn a() {}\n```\n",
        );
        let coverage = build_evidence_coverage(&target);

        assert_eq!(coverage.truncated_file_count, 1);
        assert_eq!(coverage.observed_files[0].content_status, ReviewContentStatus::Truncated);
    }

    #[test]
    fn ec_05_skipped_large_and_unreadable_are_counted() {
        let target = target_full_repo(
            "## File tree\nsrc/large.rs\nsrc/unreadable.rs\n\n## Key files\n\n### src/large.rs [skipped: 1000001 bytes]\n\n### src/unreadable.rs [unreadable: PermissionDenied]\n",
        );
        let coverage = build_evidence_coverage(&target);

        assert_eq!(coverage.skipped_large_file_count, 1);
        assert_eq!(coverage.unreadable_file_count, 1);
        assert_eq!(coverage.not_captured_file_count, 0);
    }

    #[test]
    fn ec_06_absolute_looking_paths_are_rejected_from_coverage() {
        let target = target_full_repo(
            "## File tree\nE:/secret/token.txt\n/src/abs.rs\nsrc/lib.rs\n../escape.rs\n\n## Key files\n\n### E:/secret/token.txt [full]\n```\nsecret\n```\n\n### src/lib.rs [full]\n```\nfn ok() {}\n```\n",
        );
        let coverage = build_evidence_coverage(&target);

        assert_eq!(coverage.file_tree_count, 1);
        assert_eq!(coverage.observed_files.len(), 1);
        assert_eq!(coverage.observed_files[0].path, "src/lib.rs");
        assert!(serde_json::to_string(&coverage).expect("json").contains("src/lib.rs"));
        assert!(!serde_json::to_string(&coverage).expect("json").contains("E:/secret"));
    }

    #[test]
    fn ec_07_persisted_artifact_has_optional_evidence_coverage_and_legacy_missing_loads() {
        let root = temp_path("review-evidence-coverage-artifact");
        let target = target_full_repo(
            "## File tree\nsrc/lib.rs\n\n## Key files\n\n### src/lib.rs [full]\n```\nfn ok() {}\n```\n",
        );
        let report = ReviewReport::from_model_output("No findings.");
        let artifact = persist_review_artifact(&root, &target, &report).expect("persist");
        let json = std::fs::read_to_string(&artifact.json_path).expect("read json");
        let parsed: SegoReviewArtifact = serde_json::from_str(&json).expect("deserialize artifact");

        assert!(parsed.evidence_coverage.is_some());
        assert!(json.contains("evidence_coverage"));
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn ec_08_not_captured_and_truncated_findings_get_specific_evidence_status() {
        let target = target_full_repo(
            "## File tree\nsrc/not_sampled.rs\nsrc/truncated.rs\n\n## Key files\n\n### src/truncated.rs [truncated]\n```\nfn partial() {}\n```\n",
        );
        let report = ReviewReport::from_model_output(
            r#"{"findings":[{"severity":"low","file":"src/not_sampled.rs","line":1,"title":"No content","evidence":"e","risk":"r","suggestion":"s","confidence":0.1},{"severity":"low","file":"src/truncated.rs","line":1,"title":"Partial","evidence":"e","risk":"r","suggestion":"s","confidence":0.1}]}"#,
        );
        let findings = evaluate_evidence_gate(report.findings, &target);

        assert_eq!(findings[0].evidence_status, Some(EvidenceStatus::ContentNotCaptured));
        assert_eq!(findings[1].evidence_status, Some(EvidenceStatus::ContentTruncated));
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

    // -----------------------------------------------------------------
    // Contract revision 3 (`SEG-ADR-004`): the egress, compute and budget
    // fields. The five fail-closed directions of that ADR section 3.4 each have
    // a case here, plus the misclassification cases that would make a wrong
    // answer look right.
    // -----------------------------------------------------------------

    fn observation(provider_calls: u64, tools: &[&str]) -> ReviewEgressObservation {
        ReviewEgressObservation {
            provider_calls,
            input_tokens: Some(1234),
            tool_names: tools.iter().map(|name| (*name).to_string()).collect(),
        }
    }

    #[test]
    fn a_review_that_sent_nothing_is_the_only_case_that_reports_none() {
        // Fail-closed direction 2 and 5: `none` is a claim, and it is made only
        // where the run supports it. A run that opened no request and fetched
        // nothing sent nothing - whatever the configuration says the provider is.
        let idle = observation(0, &[]);
        assert_eq!(
            data_egress_class_for(&idle, Some("https://api.deepseek.com")),
            DATA_EGRESS_NONE
        );
        assert_eq!(
            compute_boundary_for(&idle, Some("https://api.deepseek.com")),
            COMPUTE_BOUNDARY_LOCAL
        );
    }

    #[test]
    fn a_remote_provider_is_provider_egress_and_remote_compute() {
        let call = observation(1, &[]);
        assert_eq!(
            data_egress_class_for(&call, Some("https://api.deepseek.com")),
            DATA_EGRESS_PROVIDER
        );
        assert_eq!(
            compute_boundary_for(&call, Some("https://api.deepseek.com")),
            COMPUTE_BOUNDARY_REMOTE
        );
    }

    #[test]
    fn a_locally_served_provider_is_none_egress_and_local_compute() {
        // `SEG-ADR-004` section 3.1: a model served on this machine is local compute
        // with no egress. The two axes move independently, which is why a review can
        // be `none` here and `provider_and_fetch` two cases down.
        let call = observation(1, &[]);
        assert_eq!(
            data_egress_class_for(&call, Some("http://127.0.0.1:11434/v1")),
            DATA_EGRESS_NONE
        );
        assert_eq!(
            compute_boundary_for(&call, Some("http://127.0.0.1:11434/v1")),
            COMPUTE_BOUNDARY_LOCAL
        );
        assert_eq!(data_egress_class_for(&call, Some("http://[::1]:8080")), DATA_EGRESS_NONE);
        assert_eq!(data_egress_class_for(&call, Some("http://localhost:11434")), DATA_EGRESS_NONE);
    }

    #[test]
    fn an_unobservable_endpoint_is_unknown_and_never_none() {
        // Fail-closed directions 1 and 5, and the hard requirement of the ADR: a
        // review that reached a provider over an endpoint Sego cannot name is
        // `unknown`. Reading it as `none` is the exact move that lets "local-first"
        // be claimed for a review that never established it.
        let call = observation(1, &[]);
        let egress = data_egress_class_for(&call, None);
        assert_eq!(egress, DATA_EGRESS_UNKNOWN);
        assert_ne!(egress, DATA_EGRESS_NONE, "unknown must never collapse into none");
        assert_eq!(compute_boundary_for(&call, None), COMPUTE_BOUNDARY_UNKNOWN);
    }

    #[test]
    fn a_hostname_that_only_looks_like_loopback_is_not_read_as_one() {
        // The dangerous misclassification: a dotted name that begins with a loopback
        // address resolves wherever its owner points it. Treating it as loopback
        // would under-report egress, which is the direction that matters.
        let call = observation(1, &[]);
        assert_eq!(
            data_egress_class_for(&call, Some("https://127.0.0.1.example.com/v1")),
            DATA_EGRESS_PROVIDER
        );
        assert_eq!(
            compute_boundary_for(&call, Some("https://127.0.0.1.example.com/v1")),
            COMPUTE_BOUNDARY_REMOTE
        );
        // And an IPv6-embedded name is not an IPv6 literal.
        assert_eq!(
            data_egress_class_for(&call, Some("https://[::1].example.com/v1")),
            DATA_EGRESS_PROVIDER
        );
    }

    #[test]
    fn a_fetch_widens_the_egress_class_even_when_the_model_ran_locally() {
        // The combination the single-enum design could not express: local inference
        // that went and fetched a page. The fetch left this machine regardless of
        // where the model ran, so the egress class widens and the compute boundary
        // does not.
        let fetched = observation(1, &["WebFetch"]);
        assert_eq!(
            data_egress_class_for(&fetched, Some("http://127.0.0.1:11434")),
            DATA_EGRESS_PROVIDER_AND_FETCH
        );
        assert_eq!(
            compute_boundary_for(&fetched, Some("http://127.0.0.1:11434")),
            COMPUTE_BOUNDARY_LOCAL
        );
        // The other web tool is not a fetch and must not widen anything.
        let searched = observation(1, &["WebSearch"]);
        assert_eq!(
            data_egress_class_for(&searched, Some("https://api.deepseek.com")),
            DATA_EGRESS_PROVIDER
        );
    }

    #[test]
    fn half_a_budget_is_never_written() {
        // `SEG-ADR-004` section 3.3: both halves or neither. A review whose usage was
        // observed but whose caller declared no ceiling carries no budget object at
        // all, so the artifact reads as unknown rather than inviting "was it ever
        // limited?" with an answer it cannot support.
        let call = observation(3, &[]);
        assert!(
            build_budget(&ReviewBudgetDeclaration::default(), &call).is_none(),
            "an observed actual with no declared ceiling must not be written"
        );

        let declared = ReviewBudgetDeclaration {
            max_provider_calls: Some(5),
            max_fetches: None,
            max_input_tokens: None,
        };
        let budget = build_budget(&declared, &call).expect("both halves are present here");
        assert_eq!(budget.declared.max_provider_calls, Some(5));
        assert_eq!(budget.actual.provider_calls, 3);
        assert_eq!(budget.actual.fetches, 0);
        assert!(!budget.actual.limit_reached);
        assert!(!budget.actual.aborted);
    }

    #[test]
    fn a_reached_ceiling_is_recorded_in_the_budget_object() {
        // Fail-closed direction 4: touching the ceiling is visible rather than
        // silently degraded into "finished".
        let call = observation(3, &["WebFetch"]);
        let declared = ReviewBudgetDeclaration {
            max_provider_calls: Some(3),
            max_fetches: None,
            max_input_tokens: None,
        };
        let budget = build_budget(&declared, &call).expect("both halves are present here");
        assert!(budget.actual.limit_reached, "3 calls against a ceiling of 3 reached it");

        // A ceiling on a dimension the run never touched is not reached.
        let untouched = ReviewBudgetDeclaration {
            max_provider_calls: Some(9),
            max_fetches: None,
            max_input_tokens: Some(10_000),
        };
        let budget = build_budget(&untouched, &call).expect("both halves are present here");
        assert!(!budget.actual.limit_reached);
    }

    #[test]
    fn a_missing_usage_reading_is_a_gap_not_a_zero() {
        // The actual half reports what was observed. When the provider reported no
        // usage, the field is absent - `0` would be a claim about a review that
        // certainly consumed the prompt at least.
        let call =
            ReviewEgressObservation { provider_calls: 1, input_tokens: None, tool_names: vec![] };
        let declared = ReviewBudgetDeclaration {
            max_provider_calls: None,
            max_fetches: None,
            max_input_tokens: Some(10),
        };
        let budget = build_budget(&declared, &call).expect("both halves are present here");
        assert_eq!(budget.actual.input_tokens, None);
        assert!(
            !budget.actual.limit_reached,
            "an unmeasured dimension cannot be reported as having reached its ceiling"
        );
    }

    #[test]
    fn the_observed_path_writes_the_fields_and_the_plain_path_omits_them() {
        // The distinction the contract draws: an absent field means the writing path
        // does not observe, an explicit `unknown` means it observed but could not
        // determine. Both read as unknown to a consumer, and neither is `none`.
        let report = ReviewReport::from_model_output("{\"findings\": []}");
        let root_plain = temp_path("sec12-plain");
        let root_observed = temp_path("sec12-observed");
        let root_blind = temp_path("sec12-blind");
        for root in [&root_plain, &root_observed, &root_blind] {
            let _ = std::fs::create_dir_all(root);
        }
        let target = target_with_diff("diff --git a/src/lib.rs b/src/lib.rs\n");

        let read = |path: &std::path::Path| -> serde_json::Value {
            serde_json::from_str(&std::fs::read_to_string(path).expect("artifact json"))
                .expect("artifact is JSON")
        };

        let plain = persist_review_artifact(&root_plain, &target, &report).expect("persist plain");
        let plain_json = read(&plain.json_path);
        assert!(plain_json.get("data_egress_class").is_none());
        assert!(plain_json.get("compute_boundary").is_none());
        assert!(plain_json.get("budget").is_none());

        let identity = ReviewInvocationIdentity {
            provider: Some("deepseek".to_string()),
            model: Some("deepseek-chat".to_string()),
            resolved_endpoint: Some("https://api.deepseek.com".to_string()),
            invocation_id: None,
        };
        let observed = persist_review_artifact_observed(
            &root_observed,
            &target,
            &report,
            &identity,
            &observation(2, &["WebFetch"]),
            &ReviewBudgetDeclaration::default(),
        )
        .expect("persist observed");
        let observed_json = read(&observed.json_path);
        assert_eq!(
            observed_json.get("data_egress_class").and_then(serde_json::Value::as_str),
            Some(DATA_EGRESS_PROVIDER_AND_FETCH)
        );
        assert_eq!(
            observed_json.get("compute_boundary").and_then(serde_json::Value::as_str),
            Some(COMPUTE_BOUNDARY_REMOTE)
        );
        assert!(
            observed_json.get("budget").is_none(),
            "the caller declared no ceiling, so no budget object is written"
        );

        // Observed, but the endpoint could not be named: the value is written
        // explicitly as `unknown` rather than left for the reader to infer.
        let blind_identity = ReviewInvocationIdentity {
            provider: Some("openai".to_string()),
            model: Some("gpt-4o".to_string()),
            resolved_endpoint: None,
            invocation_id: None,
        };
        let blind = persist_review_artifact_observed(
            &root_blind,
            &target,
            &report,
            &blind_identity,
            &observation(1, &[]),
            &ReviewBudgetDeclaration::default(),
        )
        .expect("persist blind");
        let blind_json = read(&blind.json_path);
        assert_eq!(
            blind_json.get("data_egress_class").and_then(serde_json::Value::as_str),
            Some(DATA_EGRESS_UNKNOWN)
        );
        assert_ne!(
            blind_json.get("data_egress_class").and_then(serde_json::Value::as_str),
            Some(DATA_EGRESS_NONE)
        );
    }

    #[test]
    fn an_unobserved_artifact_reads_as_unknown_not_as_local() {
        // The consumer-side consequence written into `SEG-ADR-004` section 3.6: an
        // artifact with no egress fields must not be usable as evidence that the
        // review was local. The schema makes all three optional for exactly this
        // reason, so the field must be absent rather than defaulted to `none`.
        let draft = serde_json::json!({
            "schema_version": 1,
            "id": "review-legacy",
            "created_at_epoch_seconds": 1,
            "scope": "staged",
            "diff_hash": "sha256:legacy",
            "finding_count": 0,
            "highest_severity": null,
            "parse_status": "structured",
            "git_status": "",
            "findings": [],
            "raw_text": "",
        });
        let parsed: SegoReviewArtifact =
            serde_json::from_value(draft).expect("a pre-revision-3 artifact must still parse");
        assert_eq!(parsed.data_egress_class, None);
        assert_eq!(parsed.compute_boundary, None);
        assert!(parsed.budget.is_none());
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("sego-{name}-{unique}"))
    }
}
