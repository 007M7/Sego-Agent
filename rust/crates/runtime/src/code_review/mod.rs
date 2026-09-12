mod context;
mod prompt;
mod report;
mod scope;
mod severity;

pub use context::{ReviewContext, ReviewTarget};
pub use prompt::{build_review_prompt, ReviewPromptOptions};
pub use report::{
    evaluate_evidence_gate, latest_review_finding_statuses, load_review_finding_statuses,
    load_review_index, persist_review_artifact, persist_review_artifact_with_identity,
    record_review_finding_status, review_diff_hash, EvidenceStatus, PersistedReviewArtifact,
    ReviewFinding, ReviewFindingStatus, ReviewFindingStatusEntry, ReviewIndexEntry,
    ReviewInvocationIdentity, ReviewParseStatus, ReviewReport, IDENTITY_EVIDENCE_SELF_REPORTED,
    IDENTITY_GAP_NO_ENDPOINT_ACCESSOR,
};
pub use scope::{ReviewScope, ReviewScopeParseError};
pub use severity::ReviewSeverity;
