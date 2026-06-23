mod context;
mod prompt;
mod report;
mod scope;
mod severity;

pub use context::{ReviewContext, ReviewTarget};
pub use prompt::{build_review_prompt, ReviewPromptOptions};
pub use report::{
    evaluate_evidence_gate, latest_review_finding_statuses, load_review_finding_statuses,
    load_review_index, persist_review_artifact, record_review_finding_status, review_diff_hash,
    EvidenceStatus, PersistedReviewArtifact, ReviewFinding, ReviewFindingStatus,
    ReviewFindingStatusEntry, ReviewIndexEntry, ReviewParseStatus, ReviewReport,
};
pub use scope::{ReviewScope, ReviewScopeParseError};
pub use severity::ReviewSeverity;
