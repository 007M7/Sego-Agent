//! Sidecar JSON interface for external skill / tool consumers (c9/c PoC).
//!
//! Implements `sego sidecar review`: reads a JSON request from stdin, runs a
//! code review, and writes a JSON response to stdout. stderr is reserved for
//! diagnostic logs.
//!
//! Design decisions (Codex c9/c):
//! - D-C-1: PermissionMode::ReadOnly (non-interactive, fail-safe on write/danger)
//! - D-C-2: Reuses existing review components, does NOT modify run_review_target
//! - D-C-3: stdout always emits machine-readable JSON, even on error
//! - Minimal PoC: only `review` action, no plugin marketplace / skill runtime

use std::io::{Read, Write};

use runtime::code_review::{
    build_review_prompt, persist_review_artifact, ReviewContext, ReviewPromptOptions, ReviewReport,
    ReviewScope,
};
use serde::{Deserialize, Serialize};

use crate::{collect_review_target, LiveCli, PermissionMode};

/// Schema version for the sidecar envelope (aligned with sidecar-request-response.schema.json).
const SIDECAR_SCHEMA_VERSION: u32 = 1;

/// Run the sidecar review pipeline: read stdin JSON → review → write stdout JSON.
///
/// Returns the process exit code (0 = success, 1 = error). The caller should
/// pass this to `std::process::exit`.
pub fn run_sidecar_review_pipeline() -> i32 {
    // Read entire stdin.
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        emit_error("read_stdin_failed", "failed to read stdin");
        return 1;
    }

    // Parse request JSON. Even on parse failure, emit a structured error envelope.
    let request: SidecarReviewRequest = match serde_json::from_str(&input) {
        Ok(req) => req,
        Err(error) => {
            emit_error("invalid_request", &format!("failed to parse request JSON: {error}"));
            return 1;
        }
    };

    if request.action != "review" {
        emit_error(
            "unsupported_action",
            &format!("action '{}' is not supported (currently: review)", request.action),
        );
        return 1;
    }

    match execute_review(&request) {
        Ok(response) => {
            // Serialize and write to stdout. stdout must be pure JSON.
            match serde_json::to_string(&response) {
                Ok(json) => {
                    let _ = writeln!(std::io::stdout(), "{json}");
                    0
                }
                Err(error) => {
                    emit_error(
                        "serialization_failed",
                        &format!("failed to serialize response: {error}"),
                    );
                    1
                }
            }
        }
        Err(error) => {
            emit_error("review_failed", &error.to_string());
            1
        }
    }
}

/// Execute a single review and return a structured response.
///
/// Reuses: collect_review_target, build_review_prompt, run_turn_capture_text,
/// ReviewReport::from_model_output, persist_review_artifact.
/// Does NOT modify existing run_review_target (D-C-2).
fn execute_review(
    request: &SidecarReviewRequest,
) -> Result<SidecarReviewResponse, Box<dyn std::error::Error>> {
    let cwd = std::path::PathBuf::from(&request.cwd);
    let review_scope = ReviewScope::parse(request.scope.as_deref())?;
    let target = collect_review_target(&cwd, review_scope)?;

    if target.is_empty() {
        // No diff to review — return success with no findings.
        return Ok(SidecarReviewResponse {
            schema_version: SIDECAR_SCHEMA_VERSION,
            status: "ok".to_string(),
            review_id: None,
            diff_hash: None,
            artifact_path: None,
            findings: Some(vec![]),
            parse_status: Some("no_diff".to_string()),
            error: None,
        });
    }

    // D-C-1: ReadOnly mode, non-interactive. Write/danger tools are denied.
    // .sego/reviews/ persistence is done by persist_review_artifact directly,
    // not by model tools.
    let model = request
        .options
        .as_ref()
        .and_then(|opts| opts.model.clone())
        .unwrap_or_else(crate::default_model);
    let mut cli = LiveCli::new(model, true, None, PermissionMode::ReadOnly)?.with_machine_output();

    let context = ReviewContext::new(target);
    let prompt = build_review_prompt(&context, ReviewPromptOptions::default());
    let review_text = cli.run_turn_capture_text(&prompt, false)?;
    let report = ReviewReport::from_model_output(review_text);
    // C20.6-B R2 UX-D: apply evidence gate before persistence so sidecar
    // artifacts get the same evidence_status annotations as the CLI path.
    let findings = runtime::code_review::evaluate_evidence_gate(report.findings, &context.target);
    let report = ReviewReport { findings, ..report };
    let artifact = persist_review_artifact(&cwd, &context.target, &report)?;

    Ok(SidecarReviewResponse {
        schema_version: SIDECAR_SCHEMA_VERSION,
        status: "ok".to_string(),
        review_id: Some(artifact.id),
        diff_hash: Some(artifact.diff_hash),
        artifact_path: Some(artifact.json_path.to_string_lossy().to_string()),
        findings: Some(report.findings.clone()),
        parse_status: Some(report.parse_status.label().to_string()),
        error: None,
    })
}

/// Emit a structured error envelope to stdout (D-C-3: stdout always JSON).
fn emit_error(code: &str, message: &str) {
    let response = SidecarReviewResponse {
        schema_version: SIDECAR_SCHEMA_VERSION,
        status: "error".to_string(),
        review_id: None,
        diff_hash: None,
        artifact_path: None,
        findings: None,
        parse_status: None,
        error: Some(SidecarError { code: code.to_string(), message: message.to_string() }),
    };
    if let Ok(json) = serde_json::to_string(&response) {
        let _ = writeln!(std::io::stdout(), "{json}");
    }
}

// ---------------------------------------------------------------------------
// Request / Response types (aligned with sidecar-request-response.schema.json)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SidecarReviewRequest {
    pub schema_version: u32,
    pub action: String,
    pub cwd: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub options: Option<SidecarReviewOptions>,
    #[serde(default)]
    pub context: Option<SidecarReviewContext>,
}

#[derive(Debug, Deserialize)]
pub struct SidecarReviewOptions {
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SidecarReviewContext {
    #[serde(default)]
    pub user_intent: Option<String>,
    #[serde(default)]
    pub diff_hash: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SidecarReviewResponse {
    pub schema_version: u32,
    pub status: String,
    pub review_id: Option<String>,
    pub diff_hash: Option<String>,
    pub artifact_path: Option<String>,
    pub findings: Option<Vec<runtime::code_review::ReviewFinding>>,
    pub parse_status: Option<String>,
    pub error: Option<SidecarError>,
}

#[derive(Debug, Serialize)]
pub struct SidecarError {
    pub code: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_parses_minimal_json() {
        let json = r#"{"schema_version":1,"action":"review","cwd":"/tmp/project"}"#;
        let req: SidecarReviewRequest = serde_json::from_str(json).expect("parse");
        assert_eq!(req.action, "review");
        assert_eq!(req.cwd, "/tmp/project");
        assert!(req.scope.is_none());
    }

    #[test]
    fn request_parses_with_scope_and_options() {
        let json = r#"{"schema_version":1,"action":"review","cwd":"/tmp","scope":"staged","options":{"model":"deepseek-v4-pro"}}"#;
        let req: SidecarReviewRequest = serde_json::from_str(json).expect("parse");
        assert_eq!(req.scope.as_deref(), Some("staged"));
        assert_eq!(req.options.as_ref().and_then(|o| o.model.as_deref()), Some("deepseek-v4-pro"));
    }

    #[test]
    fn response_serializes_success() {
        let response = SidecarReviewResponse {
            schema_version: 1,
            status: "ok".to_string(),
            review_id: Some("rev-001".to_string()),
            diff_hash: Some("sha256:abc".to_string()),
            artifact_path: Some(".sego/reviews/rev-001.json".to_string()),
            findings: Some(vec![]),
            parse_status: Some("structured".to_string()),
            error: None,
        };
        let json = serde_json::to_string(&response).expect("serialize");
        assert!(json.contains(r#""status":"ok""#));
        assert!(json.contains(r#""review_id":"rev-001""#));
        assert!(json.contains(r#""schema_version":1"#));
    }

    #[test]
    fn response_serializes_evidence_status_in_findings() {
        // R5: SidecarReviewResponse with findings carrying evidence_status
        // must serialize to JSON containing "evidence_status":"verified".
        let finding = runtime::code_review::ReviewFinding {
            id: String::new(),
            severity: runtime::code_review::ReviewSeverity::Low,
            file: "src/lib.rs".to_string(),
            line: Some(1),
            title: "Test".to_string(),
            evidence: "e".to_string(),
            risk: "r".to_string(),
            suggestion: "s".to_string(),
            confidence: 0.5,
            verification_hint: None,
            evidence_status: Some(runtime::code_review::EvidenceStatus::Verified),
        };
        let response = SidecarReviewResponse {
            schema_version: 1,
            status: "ok".to_string(),
            review_id: Some("rev-evidence".to_string()),
            diff_hash: Some("sha256:abc".to_string()),
            artifact_path: Some(".sego/reviews/rev-evidence.json".to_string()),
            findings: Some(vec![finding]),
            parse_status: Some("structured".to_string()),
            error: None,
        };
        let json = serde_json::to_string(&response).expect("serialize");
        assert!(json.contains(r#""status":"ok""#));
        assert!(json.contains(r#""evidence_status":"verified""#));
    }

    #[test]
    fn response_serializes_error() {
        let response = SidecarReviewResponse {
            schema_version: 1,
            status: "error".to_string(),
            review_id: None,
            diff_hash: None,
            artifact_path: None,
            findings: None,
            parse_status: None,
            error: Some(SidecarError {
                code: "invalid_request".to_string(),
                message: "bad JSON".to_string(),
            }),
        };
        let json = serde_json::to_string(&response).expect("serialize");
        assert!(json.contains(r#""status":"error""#));
        assert!(json.contains(r#""code":"invalid_request""#));
    }

    #[test]
    fn emit_error_outputs_structured_envelope() {
        // emit_error writes to stdout; we just verify it doesn't panic.
        emit_error("test_code", "test message");
    }
}
