use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{EvidenceStatus, ReviewParseStatus, ReviewSeverity};

pub const ACCEPTANCE_RECORD_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceState {
    NeedsAttention,
    NeedsReview,
    ReadyForNormalVerification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AcceptanceReasonCode {
    BlockedReview,
    UnreliableArtifact,
    CriticalFinding,
    UnresolvedHighRisk,
    UnresolvedMediumRisk,
    FullReviewCoverageGap,
    HumanDecisionRequired,
    RemediationRerunFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NextActionCode {
    ResolveBlockerBeforeContinuing,
    ConfirmOrFixRiskThenRerun,
    ReviewCoverageGapBeforeRelease,
    ContinueNormalVerification,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewKind {
    Node,
    TaskEnd,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewTrigger {
    Automatic,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewOutcome {
    Clean,
    Warning,
    Blocked,
    CoverageGap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemediationStatus {
    NotAttempted,
    Proposed,
    AppliedAndRerunPassed,
    AppliedRerunFailed,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemediationRecord {
    pub status: RemediationStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
}

impl RemediationRecord {
    #[must_use]
    pub fn not_attempted() -> Self {
        Self {
            status: RemediationStatus::NotAttempted,
            changed_files: Vec::new(),
            reason_code: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnresolvedFinding {
    pub finding_id: String,
    pub severity: ReviewSeverity,
    pub title_original: String,
    pub evidence_original: String,
    pub file: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_status: Option<EvidenceStatus>,
    pub human_decision_required: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewEvent {
    pub event_id: String,
    pub review_kind: ReviewKind,
    pub trigger: ReviewTrigger,
    pub review_id: String,
    pub outcome: ReviewOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub highest_severity: Option<ReviewSeverity>,
    pub parse_status: ReviewParseStatus,
    pub remediation: RemediationRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_json_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_markdown_path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_findings: Vec<UnresolvedFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AcceptanceEvidenceLinks {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_json_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_markdown_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub test_evidence_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diff_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcceptanceRecord {
    pub schema_version: u32,
    pub task_id: String,
    pub acceptance_state: AcceptanceState,
    pub reason_codes: Vec<AcceptanceReasonCode>,
    pub next_action_code: NextActionCode,
    pub review_events: Vec<ReviewEvent>,
    pub evidence_links: AcceptanceEvidenceLinks,
}

impl AcceptanceRecord {
    #[must_use]
    pub fn from_review_events(task_id: impl Into<String>, review_events: Vec<ReviewEvent>) -> Self {
        let (acceptance_state, reason_codes) = aggregate_state(&review_events);
        let next_action_code = next_action_for(acceptance_state, &reason_codes);
        let evidence_links = evidence_links_for(&review_events);
        Self {
            schema_version: ACCEPTANCE_RECORD_SCHEMA_VERSION,
            task_id: task_id.into(),
            acceptance_state,
            reason_codes,
            next_action_code,
            review_events,
            evidence_links,
        }
    }

    #[must_use]
    pub fn unresolved_findings(&self) -> Vec<&UnresolvedFinding> {
        self.review_events.iter().flat_map(|event| event.unresolved_findings.iter()).collect()
    }
}

fn aggregate_state(events: &[ReviewEvent]) -> (AcceptanceState, Vec<AcceptanceReasonCode>) {
    let mut reasons = Vec::new();
    let has = |predicate: fn(&ReviewEvent) -> bool| events.iter().any(predicate);
    let has_finding = |predicate: fn(&UnresolvedFinding) -> bool| {
        events.iter().flat_map(|event| event.unresolved_findings.iter()).any(predicate)
    };

    if has(|event| event.outcome == ReviewOutcome::Blocked) {
        reasons.push(AcceptanceReasonCode::BlockedReview);
    }
    if has(|event| event.parse_status != ReviewParseStatus::Structured) {
        reasons.push(AcceptanceReasonCode::UnreliableArtifact);
    }
    if has(|event| event.highest_severity == Some(ReviewSeverity::Critical))
        || has_finding(|finding| finding.severity == ReviewSeverity::Critical)
    {
        reasons.push(AcceptanceReasonCode::CriticalFinding);
    }
    if has(|event| event.remediation.status == RemediationStatus::AppliedRerunFailed) {
        reasons.push(AcceptanceReasonCode::RemediationRerunFailed);
    }

    if !reasons.is_empty() {
        return (AcceptanceState::NeedsAttention, reasons);
    }

    if has_finding(|finding| finding.severity == ReviewSeverity::High)
        || has(|event| {
            event.highest_severity == Some(ReviewSeverity::High)
                && event.remediation.status != RemediationStatus::AppliedAndRerunPassed
        })
    {
        reasons.push(AcceptanceReasonCode::UnresolvedHighRisk);
    }
    if has_finding(|finding| finding.severity == ReviewSeverity::Medium)
        || has(|event| {
            event.highest_severity == Some(ReviewSeverity::Medium)
                && event.remediation.status != RemediationStatus::AppliedAndRerunPassed
        })
    {
        reasons.push(AcceptanceReasonCode::UnresolvedMediumRisk);
    }
    if has(|event| {
        event.review_kind == ReviewKind::Full && event.outcome == ReviewOutcome::CoverageGap
    }) {
        reasons.push(AcceptanceReasonCode::FullReviewCoverageGap);
    }
    if has_finding(|finding| finding.human_decision_required) {
        reasons.push(AcceptanceReasonCode::HumanDecisionRequired);
    }
    if has(|event| {
        event.outcome == ReviewOutcome::Warning
            && event.remediation.status != RemediationStatus::AppliedAndRerunPassed
            && matches!(
                event.highest_severity,
                None | Some(ReviewSeverity::Low) | Some(ReviewSeverity::Info)
            )
    }) {
        reasons.push(AcceptanceReasonCode::UnresolvedMediumRisk);
    }

    reasons.sort_by_key(reason_rank);
    reasons.dedup();
    if reasons.is_empty() {
        (AcceptanceState::ReadyForNormalVerification, reasons)
    } else {
        (AcceptanceState::NeedsReview, reasons)
    }
}

fn next_action_for(state: AcceptanceState, reasons: &[AcceptanceReasonCode]) -> NextActionCode {
    match state {
        AcceptanceState::NeedsAttention => NextActionCode::ResolveBlockerBeforeContinuing,
        AcceptanceState::ReadyForNormalVerification => NextActionCode::ContinueNormalVerification,
        AcceptanceState::NeedsReview
            if reasons == [AcceptanceReasonCode::FullReviewCoverageGap] =>
        {
            NextActionCode::ReviewCoverageGapBeforeRelease
        }
        AcceptanceState::NeedsReview => NextActionCode::ConfirmOrFixRiskThenRerun,
    }
}

fn evidence_links_for(events: &[ReviewEvent]) -> AcceptanceEvidenceLinks {
    let mut json_paths = BTreeSet::new();
    let mut markdown_paths = BTreeSet::new();
    for event in events {
        if let Some(path) = &event.artifact_json_path {
            json_paths.insert(path.clone());
        }
        if let Some(path) = &event.artifact_markdown_path {
            markdown_paths.insert(path.clone());
        }
    }
    AcceptanceEvidenceLinks {
        artifact_json_paths: json_paths.into_iter().collect(),
        artifact_markdown_paths: markdown_paths.into_iter().collect(),
        ..AcceptanceEvidenceLinks::default()
    }
}

const fn reason_rank(reason: &AcceptanceReasonCode) -> u8 {
    match reason {
        AcceptanceReasonCode::UnresolvedHighRisk => 0,
        AcceptanceReasonCode::FullReviewCoverageGap => 1,
        AcceptanceReasonCode::HumanDecisionRequired => 2,
        AcceptanceReasonCode::UnresolvedMediumRisk => 3,
        AcceptanceReasonCode::BlockedReview => 4,
        AcceptanceReasonCode::UnreliableArtifact => 5,
        AcceptanceReasonCode::CriticalFinding => 6,
        AcceptanceReasonCode::RemediationRerunFailed => 7,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AcceptanceReasonCode, AcceptanceRecord, AcceptanceState, NextActionCode, RemediationRecord,
        RemediationStatus, ReviewEvent, ReviewKind, ReviewOutcome, ReviewTrigger,
        UnresolvedFinding,
    };
    use crate::{ReviewParseStatus, ReviewSeverity};

    fn event(kind: ReviewKind, outcome: ReviewOutcome) -> ReviewEvent {
        ReviewEvent {
            event_id: format!("event-{kind:?}"),
            review_kind: kind,
            trigger: ReviewTrigger::Automatic,
            review_id: "review-123".to_string(),
            outcome,
            highest_severity: None,
            parse_status: ReviewParseStatus::Structured,
            remediation: RemediationRecord::not_attempted(),
            artifact_json_path: Some(".sego/reviews/review-123.json".to_string()),
            artifact_markdown_path: Some(".sego/reviews/review-123.md".to_string()),
            unresolved_findings: Vec::new(),
        }
    }

    fn unresolved(severity: ReviewSeverity, human_decision_required: bool) -> UnresolvedFinding {
        UnresolvedFinding {
            finding_id: "finding-123".to_string(),
            severity,
            title_original: "Original finding title".to_string(),
            evidence_original: "Original evidence text".to_string(),
            file: "src/auth.rs".to_string(),
            line: Some(42),
            evidence_status: None,
            human_decision_required,
        }
    }

    #[test]
    fn aggregates_node_and_full_review_into_one_task_record() {
        let mut node = event(ReviewKind::Node, ReviewOutcome::Warning);
        node.remediation = RemediationRecord {
            status: RemediationStatus::AppliedAndRerunPassed,
            changed_files: vec!["src/auth.rs".to_string()],
            reason_code: None,
        };
        let full = event(ReviewKind::Full, ReviewOutcome::CoverageGap);
        let record = AcceptanceRecord::from_review_events("task-42", vec![node, full]);

        assert_eq!(record.acceptance_state, AcceptanceState::NeedsReview);
        assert_eq!(record.next_action_code, NextActionCode::ReviewCoverageGapBeforeRelease);
        assert_eq!(record.reason_codes, vec![AcceptanceReasonCode::FullReviewCoverageGap]);
        assert_eq!(record.review_events.len(), 2);
        assert_eq!(record.evidence_links.artifact_json_paths.len(), 1);
    }

    #[test]
    fn critical_or_unreliable_event_requires_attention() {
        let mut event = event(ReviewKind::TaskEnd, ReviewOutcome::Warning);
        event.parse_status = ReviewParseStatus::ParseAttemptedButFailed;
        event.unresolved_findings.push(unresolved(ReviewSeverity::Critical, true));
        let record = AcceptanceRecord::from_review_events("task-42", vec![event]);

        assert_eq!(record.acceptance_state, AcceptanceState::NeedsAttention);
        assert_eq!(
            record.reason_codes,
            vec![AcceptanceReasonCode::UnreliableArtifact, AcceptanceReasonCode::CriticalFinding,]
        );
        assert_eq!(record.next_action_code, NextActionCode::ResolveBlockerBeforeContinuing);
    }

    #[test]
    fn unremediated_high_event_without_finding_detail_still_requires_review() {
        let mut event = event(ReviewKind::Node, ReviewOutcome::Warning);
        event.highest_severity = Some(ReviewSeverity::High);
        let record = AcceptanceRecord::from_review_events("task-42", vec![event]);

        assert_eq!(record.acceptance_state, AcceptanceState::NeedsReview);
        assert_eq!(record.reason_codes, vec![AcceptanceReasonCode::UnresolvedHighRisk]);
        assert_eq!(record.next_action_code, NextActionCode::ConfirmOrFixRiskThenRerun);
    }

    #[test]
    fn serialized_record_is_language_neutral_and_preserves_original_evidence() {
        let mut event = event(ReviewKind::Node, ReviewOutcome::Warning);
        event.unresolved_findings.push(unresolved(ReviewSeverity::High, true));
        let record = AcceptanceRecord::from_review_events("task-42", vec![event]);
        let json = serde_json::to_string(&record).expect("serialize record");

        assert!(json.contains("\"acceptance_state\":\"needs_review\""));
        assert!(json.contains("Original finding title"));
        assert!(!json.contains("请复核"));
        assert!(!json.contains("Needs review"));
    }
}
