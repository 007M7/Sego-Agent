mod context;
mod prompt;
mod report;
mod scope;
mod severity;

pub use context::{ReviewContext, ReviewTarget};
pub use prompt::{build_review_prompt, ReviewPromptOptions};
pub use report::{
    compute_boundary_for, data_egress_class_for, evaluate_evidence_gate,
    latest_review_finding_statuses, load_review_finding_statuses, load_review_index,
    persist_review_artifact, persist_review_artifact_observed,
    persist_review_artifact_with_identity, record_review_finding_status, review_diff_hash,
    EvidenceStatus, PersistedReviewArtifact, ReviewBudgetDeclaration, ReviewEgressObservation,
    ReviewFinding, ReviewFindingStatus, ReviewFindingStatusEntry, ReviewIndexEntry,
    ReviewInvocationIdentity, ReviewParseStatus, ReviewReport, COMPUTE_BOUNDARY_LOCAL,
    COMPUTE_BOUNDARY_REMOTE, COMPUTE_BOUNDARY_UNKNOWN, DATA_EGRESS_NONE, DATA_EGRESS_PROVIDER,
    DATA_EGRESS_PROVIDER_AND_FETCH, DATA_EGRESS_UNKNOWN, IDENTITY_EVIDENCE_SELF_REPORTED,
    IDENTITY_GAP_NO_ENDPOINT_ACCESSOR, WEB_FETCH_TOOL_NAME,
};
pub use scope::{ReviewScope, ReviewScopeParseError};
pub use severity::ReviewSeverity;
