//! Rendering for the code-review reports: the summary, the readiness check,
//! the completion summary printed after a review, and the preflight error.
//!
//! Moved out of `main.rs` (DEV-STRUCT-01) and, unlike the earlier slices, moved
//! **with tests** - this step's acceptance is that the crate's coverage rises,
//! which a pure move cannot achieve on its own.
//!
//! Everything here is formatting: it takes a report struct or a runtime type and
//! returns a `String`. The `print_*` wrappers that write those strings to the
//! terminal stay in `main.rs`, so what moved is the testable half.

use std::path::PathBuf;

use serde_json::json;

use runtime::{
    ReviewFindingStatus, ReviewIndexEntry, ReviewParseStatus, ReviewReport, ReviewSeverity,
    VerificationPlanStatus,
};

// Shared with the callers that print these reports.
use crate::display_path_for_user;
pub(crate) fn format_review_completion_summary(
    report: &ReviewReport,
    artifact: &runtime::PersistedReviewArtifact,
) -> String {
    const TERMINAL_FINDING_LIMIT: usize = 10;

    let highest_severity = report.highest_severity().map_or("none", runtime::ReviewSeverity::label);
    // C20.5-A R2: do not display "Findings 0" when parsing was attempted but failed.
    let findings_display =
        if report.parse_status == runtime::ReviewParseStatus::ParseAttemptedButFailed {
            "unknown (parse failed)".to_string()
        } else {
            report.findings.len().to_string()
        };
    let mut lines = vec![
        "Review Report".to_string(),
        format!("  ID               {}", artifact.id),
        format!("  Diff hash        {}", artifact.diff_hash),
        format!("  Parse status     {}", report.parse_status.label()),
        format!("  Findings         {findings_display}"),
        format!("  Highest severity {highest_severity}"),
        String::new(),
        "Findings".to_string(),
    ];

    if report.parse_status == runtime::ReviewParseStatus::ParseAttemptedButFailed {
        lines.push(
            "  Structured findings could not be parsed, but the raw output appears to contain findings."
                .to_string(),
        );
        lines.push("  Open the Markdown report to inspect the raw output.".to_string());
    } else if report.findings.is_empty() {
        lines.push("  No structured findings.".to_string());
    } else {
        for (index, finding) in report.findings.iter().take(TERMINAL_FINDING_LIMIT).enumerate() {
            let line = finding.line.map_or_else(|| "-".to_string(), |line| line.to_string());
            let evidence_tag =
                finding.evidence_status.map(|s| format!(" [{}]", s.label())).unwrap_or_default();
            lines.push(format!(
                "  {}. [{}] {}:{}{}",
                index + 1,
                finding.severity.label(),
                finding.file,
                line,
                evidence_tag
            ));
            lines.push(format!("     Title          {}", finding.title.trim()));
            push_review_summary_field(&mut lines, "Evidence", &finding.evidence);
            push_review_summary_field(&mut lines, "Risk", &finding.risk);
            push_review_summary_field(&mut lines, "Suggestion", &finding.suggestion);
            if let Some(hint) = &finding.verification_hint {
                push_review_summary_field(&mut lines, "Verify", hint);
            }
            lines.push(String::new());
        }
        if report.findings.len() > TERMINAL_FINDING_LIMIT {
            lines.push(format!(
                "  ... {} more finding(s). Open the Markdown report for the complete list.",
                report.findings.len() - TERMINAL_FINDING_LIMIT
            ));
        }
    }
    // C20.6-A UX-B: surface parse error details.
    if !report.parse_error.is_empty() {
        lines.push(String::new());
        lines.push(format!("  Parse error: {}", report.parse_error));
        if let Some(ref repair) = report.parse_repair {
            lines.push(format!("  Parse repair: {repair}"));
        }
        lines.push("  Open the Markdown report to inspect the raw output.".to_string());
    }
    if report.parse_status == runtime::ReviewParseStatus::FallbackRawText {
        lines.push(
            "  Structured parsing failed; open the Markdown report to inspect raw output."
                .to_string(),
        );
    }

    lines.extend([
        String::new(),
        "Reports".to_string(),
        format!("  Markdown         {}", display_path_for_user(&artifact.markdown_path)),
        format!("  JSON             {}", display_path_for_user(&artifact.json_path)),
        format!("  Index            {}", display_path_for_user(&artifact.index_path)),
        String::new(),
        "Next step".to_string(),
        format!("  Open the Markdown report for details, or run /review show {}.", artifact.id),
    ]);

    lines.join("\n")
}

pub(crate) fn push_review_summary_field(lines: &mut Vec<String>, label: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }

    let mut value_lines = value.lines();
    if let Some(first) = value_lines.next() {
        lines.push(format!("     {label:<14} {first}"));
    }
    for line in value_lines {
        lines.push(format!("     {:<14} {line}", ""));
    }
}

pub(crate) struct CodeReviewSummaryReport {
    pub(crate) root: String,
    pub(crate) git_status: String,
    pub(crate) staged_diff: String,
    pub(crate) unstaged_diff: String,
    pub(crate) staged_paths: Vec<PathBuf>,
    pub(crate) safety_report: Option<runtime::SafetyLockReport>,
    pub(crate) verification_plan: runtime::VerificationPlan,
    pub(crate) latest_review: Option<ReviewIndexEntry>,
    pub(crate) latest_status_counts: Option<ReviewFindingStatusCounts>,
}

#[derive(Default)]
pub(crate) struct ReviewFindingStatusCounts {
    pub(crate) open: usize,
    pub(crate) acknowledged: usize,
    pub(crate) fixed: usize,
    pub(crate) accepted_risk: usize,
    pub(crate) false_positive: usize,
    pub(crate) ignored: usize,
}

impl ReviewFindingStatusCounts {
    pub(crate) fn from_entries(
        statuses: &std::collections::BTreeMap<String, runtime::ReviewFindingStatusEntry>,
    ) -> Self {
        let mut counts = Self::default();
        for entry in statuses.values() {
            match entry.status {
                ReviewFindingStatus::Open => counts.open += 1,
                ReviewFindingStatus::Acknowledged => counts.acknowledged += 1,
                ReviewFindingStatus::Fixed => counts.fixed += 1,
                ReviewFindingStatus::AcceptedRisk => counts.accepted_risk += 1,
                ReviewFindingStatus::FalsePositive => counts.false_positive += 1,
                ReviewFindingStatus::Ignored => counts.ignored += 1,
            }
        }
        counts
    }

    fn is_empty(&self) -> bool {
        self.open == 0
            && self.acknowledged == 0
            && self.fixed == 0
            && self.accepted_risk == 0
            && self.false_positive == 0
            && self.ignored == 0
    }

    pub(crate) fn render(&self) -> String {
        format!(
            "open {}, acknowledged {}, fixed {}, accepted_risk {}, false_positive {}, ignored {}",
            self.open,
            self.acknowledged,
            self.fixed,
            self.accepted_risk,
            self.false_positive,
            self.ignored
        )
    }
}

impl CodeReviewSummaryReport {
    pub(crate) fn render(&self) -> String {
        let mut lines = Vec::new();
        let branch_status = self.git_status.lines().next().unwrap_or("(not a git repository)");

        lines.push("Review Summary".to_string());
        lines.push(format!("  Root             {}", self.root));
        lines.push("  Mode             read-only".to_string());
        lines.push("  Scope            workspace".to_string());
        lines.push(
            "  Safety           no files were modified; no model calls were made; no tools were executed"
                .to_string(),
        );

        self.push_git_section(&mut lines, branch_status);
        self.push_staged_safety_section(&mut lines);
        self.push_latest_review_section(&mut lines);
        self.push_verify_fast_section(&mut lines);
        self.push_suggested_next_steps(&mut lines);

        lines.join("\n")
    }

    fn push_git_section(&self, lines: &mut Vec<String>, branch_status: &str) {
        lines.push(String::new());
        lines.push("  Git".to_string());
        lines.push(format!("    Branch/status  {branch_status}"));
        lines.push(format!("    Staged files   {}", self.staged_paths.len()));
        lines.push(format!(
            "    Staged diff    {}",
            if self.staged_diff.trim().is_empty() { "no" } else { "yes" }
        ));
        lines.push(format!(
            "    Unstaged diff  {}",
            if self.unstaged_diff.trim().is_empty() { "no" } else { "yes" }
        ));
    }

    fn push_staged_safety_section(&self, lines: &mut Vec<String>) {
        lines.push(String::new());
        lines.push("  Staged safety".to_string());
        match &self.safety_report {
            None => {
                lines.push("    Result         no staged files".to_string());
                lines.push("    Findings       0".to_string());
            }
            Some(report) if report.findings.is_empty() => {
                lines.push("    Result         passed".to_string());
                lines.push("    Findings       0".to_string());
            }
            Some(report) => {
                let has_high_risk = report
                    .findings
                    .iter()
                    .any(|finding| matches!(finding.severity, runtime::SafetySeverity::High));
                lines.push(format!(
                    "    Result         {}",
                    if has_high_risk { "blocked" } else { "needs review" }
                ));
                lines.push(format!("    Findings       {}", report.findings.len()));
                for finding in report.findings.iter().take(8) {
                    let location = finding.line.map_or_else(
                        || finding.file.clone(),
                        |line| format!("{}:{line}", finding.file),
                    );
                    lines.push(format!(
                        "    [{}] {} {}",
                        finding.severity.label(),
                        location,
                        finding.title
                    ));
                }
                if report.findings.len() > 8 {
                    lines.push(format!("    ... {} more", report.findings.len() - 8));
                }
            }
        }
    }

    fn push_latest_review_section(&self, lines: &mut Vec<String>) {
        lines.push(String::new());
        lines.push("  Latest review".to_string());
        if let Some(entry) = &self.latest_review {
            let highest_severity =
                entry.highest_severity.map_or("none", runtime::ReviewSeverity::label);
            lines.push(format!("    ID             {}", entry.id));
            lines.push(format!("    Scope          {}", entry.scope));
            lines.push(format!("    Findings       {}", entry.finding_count));
            lines.push(format!("    Highest        {highest_severity}"));
            lines.push(format!("    Parse status   {}", entry.parse_status.label()));
            if let Some(counts) = &self.latest_status_counts {
                if !counts.is_empty() {
                    lines.push(format!("    Statuses       {}", counts.render()));
                }
            }
        } else {
            lines.push("    ID             none".to_string());
            lines.push("    Findings       0".to_string());
        }
    }

    fn push_verify_fast_section(&self, lines: &mut Vec<String>) {
        lines.push(String::new());
        lines.push("  Verify fast".to_string());
        match &self.verification_plan.status {
            VerificationPlanStatus::Ready => {
                lines.push(format!(
                    "    Plan           {} command(s)",
                    self.verification_plan.commands.len()
                ));
                for command in &self.verification_plan.commands {
                    lines.push(format!(
                        "    {} ({})",
                        command.display_command(),
                        command.working_dir
                    ));
                }
            }
            VerificationPlanStatus::NoPlan { reason } => {
                lines.push("    Plan           no plan".to_string());
                lines.push(format!("    Reason         {reason}"));
            }
        }
    }

    fn push_suggested_next_steps(&self, lines: &mut Vec<String>) {
        lines.push(String::new());
        lines.push("  Suggested next steps".to_string());
        lines.push("    sego /review ready".to_string());
        lines.push("    sego /review safety staged".to_string());
        lines.push("    sego /review staged".to_string());
        lines.push("    sego /verify fast".to_string());
    }
}

pub(crate) struct CodeReviewReadinessReport {
    pub(crate) root: String,
    pub(crate) staged_paths: Vec<PathBuf>,
    pub(crate) safety_report: runtime::SafetyLockReport,
    pub(crate) verification_plan: runtime::VerificationPlan,
}

impl CodeReviewReadinessReport {
    pub(crate) fn render(&self) -> String {
        let has_high_risk = self
            .safety_report
            .findings
            .iter()
            .any(|finding| finding.severity == runtime::SafetySeverity::High);
        let blocked = self.staged_paths.is_empty() || has_high_risk;
        let mut lines = Vec::new();

        lines.push("Review Readiness".to_string());
        lines.push(format!("  Root             {}", self.root));
        lines.push("  Mode             read-only".to_string());
        lines.push("  Scope            staged".to_string());
        lines.push(
            "  Safety           no files were modified; no model calls were made; no tools were executed"
                .to_string(),
        );
        lines.push(format!(
            "  Result           {}",
            if blocked { "blocked" } else { "ready with manual gates" }
        ));

        lines.push(format!("  Staged files     {}", self.staged_paths.len()));
        for path in self.staged_paths.iter().take(12) {
            lines.push(format!("    {}", path.display()));
        }
        if self.staged_paths.len() > 12 {
            lines.push(format!("    ... {} more", self.staged_paths.len() - 12));
        }

        if self.safety_report.findings.is_empty() {
            lines.push("  Safety lock      passed".to_string());
        } else {
            lines.push(format!(
                "  Safety lock      {} finding(s)",
                self.safety_report.findings.len()
            ));
            for finding in &self.safety_report.findings {
                let location = finding.line.map_or_else(
                    || finding.file.clone(),
                    |line| format!("{}:{line}", finding.file),
                );
                lines.push(format!(
                    "    [{}] {} ({})",
                    finding.severity.label(),
                    location,
                    finding.title
                ));
            }
        }

        match &self.verification_plan.status {
            VerificationPlanStatus::Ready => {
                lines.push(format!(
                    "  Verify fast      planned {} command(s)",
                    self.verification_plan.commands.len()
                ));
                for command in &self.verification_plan.commands {
                    lines.push(format!(
                        "    {} ({})",
                        command.display_command(),
                        command.working_dir
                    ));
                }
            }
            VerificationPlanStatus::NoPlan { reason } => {
                lines.push("  Verify fast      no plan".to_string());
                lines.push(format!("    Reason         {reason}"));
            }
        }

        lines.push("  Review gate      manual".to_string());
        lines.push("    Command        sego /review staged".to_string());
        lines.push("  Verification     manual".to_string());
        lines.push("    Command        sego /verify fast".to_string());

        if self.staged_paths.is_empty() {
            lines.push("  Next step        stage changes before running /review ready".to_string());
        } else if has_high_risk {
            lines.push(
                "  Next step        fix high-risk safety findings before review or commit"
                    .to_string(),
            );
        } else {
            lines.push(
                "  Next step        run /review staged, then /verify fast before commit"
                    .to_string(),
            );
        }

        lines.join("\n")
    }
}

pub(crate) fn build_review_summary_json_value(
    requested_id: &str,
    entry: &ReviewIndexEntry,
    status_counts: ReviewFindingStatusCounts,
) -> serde_json::Value {
    let highest_severity = entry.highest_severity.map(runtime::ReviewSeverity::label);
    let summary_kind =
        if requested_id == "latest" { "sego_latest_review_summary" } else { "sego_review_summary" };
    json!({
        "schema_version": 1,
        "kind": summary_kind,
        "found": true,
        "review": {
            "id": entry.id,
            "created_at_epoch_seconds": entry.created_at_epoch_seconds,
            "scope": entry.scope,
            "diff_hash": entry.diff_hash,
            "finding_count": entry.finding_count,
            "highest_severity": highest_severity,
            "parse_status": entry.parse_status.label(),
            "json_path": entry.json_path,
            "markdown_path": entry.markdown_path,
        },
        "status_counts": {
            "open": status_counts.open,
            "acknowledged": status_counts.acknowledged,
            "fixed": status_counts.fixed,
            "accepted_risk": status_counts.accepted_risk,
            "false_positive": status_counts.false_positive,
            "ignored": status_counts.ignored,
        }
    })
}

/// Phase 2-C: Format a preflight block as a structured error message.
///
/// Block guarantees snapshot collection and model runtime are not reached.
/// This message is shown to the user; it is not a review artifact.
pub(crate) fn format_preflight_block_error(
    preflight: &crate::full_scope_preflight::PreflightResult,
) -> String {
    let reason = preflight.block_reason.as_ref().expect("block result must have a block reason");
    let mut lines = vec![
        "Review".to_string(),
        "  Result           blocked by preflight".to_string(),
        format!("  Block code        {}", reason.code()),
        format!("  Block reason      {}", reason.message()),
    ];
    if let Some(target) = &preflight.resolved_target {
        lines.push(format!("  Resolved target   {}", target.display()));
    }
    if let Some(root) = &preflight.git_root {
        lines.push(format!("  Git root          {}", root.display()));
    }
    lines.push(
        "  Boundary          preflight block; snapshot and model runtime not reached".to_string(),
    );
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime::{
        EvidenceStatus, PersistedReviewArtifact, ReviewFinding, ReviewFindingStatus,
        ReviewFindingStatusEntry, ReviewParseStatus, ReviewSeverity, SafetyCategory, SafetyFinding,
        SafetyLockReport, SafetyScanMode, SafetySeverity, VerificationCommand, VerificationPlan,
        VerificationScope,
    };

    fn plan_with(commands: Vec<VerificationCommand>) -> VerificationPlan {
        VerificationPlan::ready(VerificationScope::Auto, commands)
    }

    fn safety_report(findings: Vec<SafetyFinding>) -> SafetyLockReport {
        SafetyLockReport {
            root: "/w".to_string(),
            mode: SafetyScanMode::Staged,
            findings,
            warnings: Vec::new(),
        }
    }

    fn high_risk_finding() -> SafetyFinding {
        SafetyFinding {
            severity: SafetySeverity::High,
            category: SafetyCategory::Secret,
            file: "src/lib.rs".to_string(),
            line: Some(3),
            title: "possible secret".to_string(),
            evidence: "a key-shaped literal".to_string(),
            risk: "a credential may be committed".to_string(),
            suggestion: "remove it".to_string(),
        }
    }

    fn finding(id: &str, severity: ReviewSeverity) -> ReviewFinding {
        ReviewFinding {
            id: id.to_string(),
            severity,
            file: "src/lib.rs".to_string(),
            line: Some(2),
            title: "guard bypass".to_string(),
            evidence: "a branch runs before the guard".to_string(),
            risk: "unvalidated input reaches the parser".to_string(),
            suggestion: "move the guard up".to_string(),
            confidence: 0.8,
            verification_hint: Some("rg guard src/lib.rs".to_string()),
            evidence_status: Some(EvidenceStatus::Verified),
        }
    }

    fn index_entry() -> ReviewIndexEntry {
        ReviewIndexEntry {
            id: "review-1-abcdef".to_string(),
            created_at_epoch_seconds: 1_700_000_000,
            scope: "staged".to_string(),
            diff_hash: "abc123".to_string(),
            finding_count: 2,
            highest_severity: Some(ReviewSeverity::High),
            parse_status: ReviewParseStatus::Structured,
            json_path: "/w/.sego/reviews/x.json".to_string(),
            markdown_path: "/w/.sego/reviews/x.md".to_string(),
        }
    }

    #[test]
    fn readiness_blocks_when_nothing_is_staged() {
        let report = CodeReviewReadinessReport {
            root: "/w".to_string(),
            staged_paths: Vec::new(),
            safety_report: safety_report(Vec::new()),
            verification_plan: plan_with(Vec::new()),
        };
        let rendered = report.render();
        assert!(rendered.contains("Review Readiness"), "{rendered}");
        // The point of the readiness report: say whether a review can run at all.
        assert!(!rendered.contains("Ready"), "nothing staged must not read as ready:\n{rendered}");
    }

    #[test]
    fn readiness_accepts_staged_paths_without_high_risk_findings() {
        let report = CodeReviewReadinessReport {
            root: "/w".to_string(),
            staged_paths: vec![PathBuf::from("src/lib.rs")],
            safety_report: safety_report(Vec::new()),
            verification_plan: plan_with(Vec::new()),
        };
        let rendered = report.render();
        assert!(rendered.contains("src/lib.rs"), "{rendered}");
        assert!(
            !rendered.contains("blocked"),
            "a clean staged change must not be reported as blocked:\n{rendered}"
        );
    }

    #[test]
    fn readiness_blocks_on_a_high_severity_safety_finding() {
        let report = CodeReviewReadinessReport {
            root: "/w".to_string(),
            staged_paths: vec![PathBuf::from("src/lib.rs")],
            safety_report: safety_report(vec![high_risk_finding()]),
            verification_plan: plan_with(Vec::new()),
        };
        let rendered = report.render();
        assert!(
            rendered.contains("possible secret"),
            "the blocking finding must be visible in the report:\n{rendered}"
        );
    }

    #[test]
    fn summary_report_renders_its_sections() {
        let report = CodeReviewSummaryReport {
            root: "/w".to_string(),
            git_status: "## main".to_string(),
            staged_diff: String::new(),
            unstaged_diff: String::new(),
            staged_paths: vec![PathBuf::from("src/lib.rs")],
            safety_report: None,
            verification_plan: plan_with(Vec::new()),
            latest_review: None,
            latest_status_counts: None,
        };
        let rendered = report.render();
        assert!(rendered.contains("Review Summary"), "{rendered}");
        assert!(rendered.contains("read-only"), "the mode is part of the claim:\n{rendered}");
        assert!(
            rendered.contains("no model calls were made"),
            "the summary states what it did not do; that is the trust claim:\n{rendered}"
        );
    }

    #[test]
    fn summary_report_includes_the_latest_review_when_present() {
        let report = CodeReviewSummaryReport {
            root: "/w".to_string(),
            git_status: "## main".to_string(),
            staged_diff: String::new(),
            unstaged_diff: String::new(),
            staged_paths: vec![PathBuf::from("src/lib.rs")],
            safety_report: None,
            verification_plan: plan_with(Vec::new()),
            latest_review: Some(index_entry()),
            latest_status_counts: Some(ReviewFindingStatusCounts::default()),
        };
        let rendered = report.render();
        assert!(rendered.contains("review-1-abcdef"), "{rendered}");
    }

    #[test]
    fn completion_summary_says_unknown_rather_than_zero_when_parsing_failed() {
        // C20.5-A R2, the rule this function carries: a failed parse must not be
        // displayed as "Findings 0", because that reads as a clean review.
        let report = ReviewReport {
            findings: Vec::new(),
            raw_text: String::new(),
            parse_status: ReviewParseStatus::ParseAttemptedButFailed,
            parse_error: "unexpected token".to_string(),
            parse_repair: None,
        };
        let artifact = PersistedReviewArtifact {
            id: "review-1-abcdef".to_string(),
            diff_hash: "abc123".to_string(),
            json_path: PathBuf::from("/w/x.json"),
            markdown_path: PathBuf::from("/w/x.md"),
            index_path: PathBuf::from("/w/index.jsonl"),
        };
        let rendered = format_review_completion_summary(&report, &artifact);
        assert!(rendered.contains("unknown (parse failed)"), "{rendered}");
        assert!(
            !rendered.contains("Findings         0"),
            "a failed parse must never read as zero findings:\n{rendered}"
        );
    }

    #[test]
    fn completion_summary_lists_findings_and_caps_the_terminal_list() {
        let findings =
            (0..12).map(|n| finding(&format!("f{n}"), ReviewSeverity::Medium)).collect::<Vec<_>>();
        let report = ReviewReport {
            findings,
            raw_text: String::new(),
            parse_status: ReviewParseStatus::Structured,
            parse_error: String::new(),
            parse_repair: None,
        };
        let artifact = PersistedReviewArtifact {
            id: "review-2-abcdef".to_string(),
            diff_hash: "def456".to_string(),
            json_path: PathBuf::from("/w/y.json"),
            markdown_path: PathBuf::from("/w/y.md"),
            index_path: PathBuf::from("/w/index.jsonl"),
        };
        let rendered = format_review_completion_summary(&report, &artifact);
        // The summary prints titles and a numbered list, not ids.
        assert!(rendered.contains("  Findings         12"), "{rendered}");
        assert!(rendered.contains("  10. [medium]"), "the tenth entry is listed:\n{rendered}");
        assert!(!rendered.contains("  11. [medium]"), "the eleventh is not:\n{rendered}");
        assert!(
            rendered.contains("... 2 more finding(s)"),
            "the cap must be stated, so a reader knows the list is partial:\n{rendered}"
        );
    }

    #[test]
    fn status_counts_tally_each_status() {
        let entry = |id: &str, status| ReviewFindingStatusEntry {
            report_id: "review-1".to_string(),
            finding_id: id.to_string(),
            status,
            note: None,
            updated_at_epoch_seconds: 0,
        };
        let statuses = [
            ("a", ReviewFindingStatus::Open),
            ("b", ReviewFindingStatus::Open),
            ("c", ReviewFindingStatus::Fixed),
            ("d", ReviewFindingStatus::AcceptedRisk),
            ("e", ReviewFindingStatus::FalsePositive),
            ("f", ReviewFindingStatus::Acknowledged),
        ]
        .into_iter()
        .map(|(id, status)| (id.to_string(), entry(id, status)))
        .collect();
        let counts = ReviewFindingStatusCounts::from_entries(&statuses);
        assert_eq!(counts.open, 2);
        assert_eq!(counts.fixed, 1);
        assert_eq!(counts.accepted_risk, 1);
        assert_eq!(counts.false_positive, 1);
        assert_eq!(counts.acknowledged, 1);
        assert_eq!(counts.ignored, 0);
    }

    #[test]
    fn summary_json_carries_the_entry_and_the_counts() {
        let mut statuses = std::collections::BTreeMap::new();
        statuses.insert(
            "f".to_string(),
            ReviewFindingStatusEntry {
                report_id: "review-1".to_string(),
                finding_id: "f".to_string(),
                status: ReviewFindingStatus::Open,
                note: None,
                updated_at_epoch_seconds: 0,
            },
        );
        let counts = ReviewFindingStatusCounts::from_entries(&statuses);
        let value = build_review_summary_json_value("latest", &index_entry(), counts);
        assert_eq!(value["kind"], "sego_latest_review_summary");
        assert_eq!(value["review"]["id"], "review-1-abcdef");
        // The counts sit beside the review object, not inside it.
        assert_eq!(value["status_counts"]["open"], 1);
        assert_eq!(value["review"]["highest_severity"], "high");
    }

    #[test]
    fn a_multi_line_field_is_indented_under_its_label() {
        let mut lines = Vec::new();
        push_review_summary_field(&mut lines, "Evidence", "first\nsecond");
        assert_eq!(lines.first().map(String::as_str), Some("     Evidence       first"));
        assert_eq!(
            lines.get(1).map(String::as_str),
            Some("                    second"),
            "the continuation must line up under the value, not the label"
        );
    }
}
