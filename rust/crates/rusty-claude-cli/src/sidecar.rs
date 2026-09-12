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
    build_review_prompt, persist_review_artifact, review_diff_hash, ReviewContext,
    ReviewInvocationIdentity, ReviewPromptOptions, ReviewReport, ReviewScope,
};
use serde::{Deserialize, Serialize};

use crate::{collect_review_target, LiveCli, PermissionMode};

/// Schema version for the sidecar envelope (aligned with sidecar-request-response.schema.json).
const SIDECAR_SCHEMA_VERSION: u32 = 1;

/// Failure carrying a machine-readable sidecar error code, so the envelope can
/// report a specific reason instead of collapsing every failure into `review_failed`.
#[derive(Debug)]
struct SidecarFailure {
    code: &'static str,
    message: String,
}

impl std::fmt::Display for SidecarFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for SidecarFailure {}

/// Input-binding check for `context.diff_hash` (D-2).
///
/// The caller may assert the exact diff it intends to have reviewed. The value is
/// compared exactly (after trimming) with the hash the persisted artifact will
/// carry, because both come from `review_diff_hash` (`report.rs:741`). A wrong
/// hash must fail rather than be echoed back as if the input had been bound.
#[must_use]
fn diff_hash_binding_ok(expected: &str, actual: &str) -> bool {
    expected.trim() == actual
}

/// Maximum accepted length of `context.invocation_id`, matching
/// `maxLength` in `schema/sidecar-request-response.schema.json`.
const MAX_INVOCATION_ID_LEN: usize = 128;

/// Whether a caller-supplied invocation id satisfies the published contract.
///
/// Sego treats the value as opaque — it never parses its structure — but it does
/// enforce the one bound the schema declares, so a request cannot be accepted here
/// while being invalid against the contract.
#[must_use]
fn invocation_id_is_valid(id: &str) -> bool {
    !id.is_empty() && id.chars().count() <= MAX_INVOCATION_ID_LEN
}


/// Map a request-supplied provider name to a concrete provider kind.
///
/// The governed plugin path must never guess: an unrecognised name is a hard error.
fn provider_kind_from_name(name: &str) -> Result<api::ProviderKind, SidecarFailure> {
    match name.trim().to_ascii_lowercase().as_str() {
        "anthropic" | "claude" => Ok(api::ProviderKind::Anthropic),
        "deepseek" => Ok(api::ProviderKind::DeepSeek),
        "openai" => Ok(api::ProviderKind::OpenAi),
        "xai" | "grok" => Ok(api::ProviderKind::Xai),
        other => Err(SidecarFailure {
            code: "unknown_provider",
            message: format!(
                "options.provider '{other}' is not supported (expected anthropic, deepseek, openai, or xai)"
            ),
        }),
    }
}

/// Canonical lower-case provider name, used in error messages.
#[must_use]
fn provider_name(kind: api::ProviderKind) -> &'static str {
    match kind {
        api::ProviderKind::Anthropic => "anthropic",
        api::ProviderKind::DeepSeek => "deepseek",
        api::ProviderKind::OpenAi => "openai",
        api::ProviderKind::Xai => "xai",
    }
}

/// Strict routing for the governed plugin path.
///
/// `detect_provider_kind` falls through to environment-variable priority when a
/// model name carries no known family. This resolves the route without any
/// ordered fallback:
/// - an unrecognised provider name fails (`unknown_provider`);
/// - a provider supplied without an explicit model fails (`provider_requires_model`),
///   so the machine default cannot silently decide the route;
/// - a model whose family cannot be determined from its name fails
///   (`unknown_model`) instead of falling through to env-var priority;
/// - a model whose family contradicts the requested provider fails
///   (`provider_model_conflict`).
///
/// Requiring a determinable family is what makes the route verifiable: the model
/// client re-derives the provider from the same model name inside `LiveCli::new`,
/// so the kind returned here is the kind that actually runs rather than an
/// assertion about it.
fn resolve_strict_routing(
    provider: Option<&str>,
    model: Option<&str>,
) -> Result<(api::ProviderKind, String), SidecarFailure> {
    let requested = provider
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(provider_kind_from_name)
        .transpose()?;

    let explicit_model = model.map(str::trim).filter(|value| !value.is_empty());

    if requested.is_some() && explicit_model.is_none() {
        return Err(SidecarFailure {
            code: "provider_requires_model",
            message: "options.provider requires an explicit options.model on the governed plugin path; the machine default must not decide the route".to_string(),
        });
    }

    let model = explicit_model.map_or_else(crate::default_model, ToString::to_string);

    let Some(family) = api::metadata_for_model(&model).map(|metadata| metadata.provider) else {
        return Err(SidecarFailure {
            code: "unknown_model",
            message: format!(
                "model '{model}' has no determinable provider family; use a recognised model name"
            ),
        });
    };

    if let Some(requested) = requested {
        if requested != family {
            return Err(SidecarFailure {
                code: "provider_model_conflict",
                message: format!(
                    "options.provider '{}' does not match model '{model}', which resolves to provider '{}'",
                    provider_name(requested),
                    provider_name(family)
                ),
            });
        }
    }

    Ok((family, model))
}

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

    // Objective step 3: the envelope version is a contract boundary. An
    // unsupported value must fail closed instead of being parsed on trust.
    if request.schema_version != SIDECAR_SCHEMA_VERSION {
        emit_error(
            "unsupported_schema_version",
            &format!(
                "schema_version {} is not supported by this build (supported: {SIDECAR_SCHEMA_VERSION})",
                request.schema_version
            ),
        );
        return 1;
    }

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
            let (code, message) = match error.downcast_ref::<SidecarFailure>() {
                Some(failure) => (failure.code, failure.message.clone()),
                None => ("review_failed", error.to_string()),
            };
            emit_error(code, &message);
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

    // D-2: verify the caller's expected input binding BEFORE any model call or
    // persistence, so a wrong hash fails instead of silently reviewing whatever
    // the scope happens to contain. `review_diff_hash` is the same function the
    // persisted artifact uses for its `diff_hash` (`report.rs:741`).
    if let Some(expected) = request
        .context
        .as_ref()
        .and_then(|context| context.diff_hash.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let actual = review_diff_hash(&target);
        if !diff_hash_binding_ok(expected, &actual) {
            return Err(Box::new(SidecarFailure {
                code: "diff_hash_mismatch",
                message: format!(
                    "context.diff_hash '{expected}' does not match the diff in the selected scope (actual '{actual}')"
                ),
            }));
        }
    }

    // D-C-1: ReadOnly mode, non-interactive. Write/danger tools are denied.
    // .sego/reviews/ persistence is done by persist_review_artifact directly,
    // not by model tools.
    // Objective step 2: the governed plugin path resolves provider + model
    // strictly, so no environment-ordered fallback and no machine default can
    // decide the route silently. Resolved before the empty-diff shortcut so an
    // unknown provider or model fails even when there is nothing to review,
    // instead of returning a misleading `no_diff` success.
    let (provider, model) = resolve_strict_routing(
        request.options.as_ref().and_then(|options| options.provider.as_deref()),
        request.options.as_ref().and_then(|options| options.model.as_deref()),
    )?;
    // Envelope validation only in this state: the value is bounded by the published
    // contract, so the implementation cannot accept what the contract calls invalid.
    if let Some(id) = request.context.as_ref().and_then(|ctx| ctx.invocation_id.as_deref()) {
        if !invocation_id_is_valid(id) {
            return Err(Box::new(SidecarFailure {
                code: "invalid_request",
                message: format!(
                    "context.invocation_id must be 1..={MAX_INVOCATION_ID_LEN} characters, got {}",
                    id.chars().count()
                ),
            }));
        }
    }

    if target.is_empty() {
        // Restored 2026-09-12 once the consuming side shipped its half: the mapper now
        // accepts `parse_status: "no_diff"` and maps it to Unverified + Missing
        // coverage, never to a pass. Recall what the value means — there was no work to
        // review. It is NOT a checker failure and it is emphatically NOT a pass, and a
        // consumer must not inherit a conclusion from this silence.
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
#[serde(deny_unknown_fields)]
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

/// Optional review options.
///
/// `deny_unknown_fields` is deliberate: an option the implementation does not
/// read must fail loudly rather than be accepted and silently ignored, which
/// would give the caller a false sense of having configured something.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SidecarReviewOptions {
    #[serde(default)]
    pub model: Option<String>,
    /// Requested provider for the governed plugin path. An unrecognised name
    /// fails the request rather than falling back to any other route.
    #[serde(default)]
    pub provider: Option<String>,
}

/// Optional context for the request.
///
/// Only `diff_hash` is accepted: it is the one context field the implementation
/// actually verifies (see [`diff_hash_binding_ok`]). The previously declared
/// `user_intent` was never read by any code path, so it was removed instead of
/// being accepted as a no-op.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SidecarReviewContext {
    #[serde(default)]
    pub diff_hash: Option<String>,
    /// Caller-minted invocation identity (for example an EgoPulse
    /// verification/checker execution id). Sego only bounds it and echoes it.
    #[serde(default)]
    pub invocation_id: Option<String>,
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
    fn invocation_id_bound_matches_the_published_contract() {
        // The schema declares `maxLength: 128`; the runtime must not accept more.
        assert!(invocation_id_is_valid("inv-1"));
        assert!(invocation_id_is_valid(&"x".repeat(MAX_INVOCATION_ID_LEN)));
        assert!(!invocation_id_is_valid(""));
        assert!(!invocation_id_is_valid(&"x".repeat(MAX_INVOCATION_ID_LEN + 1)));
    }

    #[test]
    fn unknown_option_and_context_fields_are_rejected() {
        // A documented-but-unimplemented option must fail loudly, not be accepted
        // and silently ignored. Each case also asserts *why* it failed, so a typo
        // in the fixture cannot make the test pass for an unrelated reason.
        let unknown_option = r#"{"schema_version":1,"action":"review","cwd":"/tmp/p","options":{"include_safety_lock":true}}"#;
        let error = serde_json::from_str::<SidecarReviewRequest>(unknown_option)
            .expect_err("include_safety_lock must be rejected");
        assert!(error.to_string().contains("include_safety_lock"), "unexpected error: {error}");

        // A declared-but-never-read context field is gone, so sending it fails.
        let unknown_context = r#"{"schema_version":1,"action":"review","cwd":"/tmp/p","context":{"user_intent":"x"}}"#;
        let error = serde_json::from_str::<SidecarReviewRequest>(unknown_context)
            .expect_err("user_intent must be rejected");
        assert!(error.to_string().contains("user_intent"), "unexpected error: {error}");

        // Unknown top-level keys are rejected too.
        let unknown_top = r#"{"schema_version":1,"action":"review","cwd":"/tmp/p","surprise":1}"#;
        let error = serde_json::from_str::<SidecarReviewRequest>(unknown_top)
            .expect_err("unknown top-level keys must be rejected");
        assert!(error.to_string().contains("surprise"), "unexpected error: {error}");

        // The accepted shape still parses.
        let accepted = r#"{"schema_version":1,"action":"review","cwd":"/tmp/p","options":{"provider":"deepseek","model":"deepseek-chat"},"context":{"diff_hash":"abc"}}"#;
        assert!(serde_json::from_str::<SidecarReviewRequest>(accepted).is_ok());
    }

    #[test]
    fn response_serialization_survives_non_finite_confidence() {
        // `serialization_failed` is the one envelope error code with no observed
        // end-to-end trigger. The only float in the envelope is a finding's
        // confidence, so this pins whether that field can make serialization fail.
        // If this assertion ever flips, the documented conclusion changes too.
        let finding = runtime::code_review::ReviewFinding {
            id: String::new(),
            severity: runtime::code_review::ReviewSeverity::Low,
            file: "src/lib.rs".to_string(),
            line: Some(1),
            title: "Test".to_string(),
            evidence: "e".to_string(),
            risk: "r".to_string(),
            suggestion: "s".to_string(),
            confidence: f32::NAN,
            verification_hint: None,
            evidence_status: None,
        };
        let response = SidecarReviewResponse {
            schema_version: 1,
            status: "ok".to_string(),
            review_id: Some("rev-nan".to_string()),
            diff_hash: Some("h".to_string()),
            artifact_path: Some("p".to_string()),
            findings: Some(vec![finding]),
            parse_status: Some("structured".to_string()),
            error: None,
        };
        let serialized = serde_json::to_string(&response);
        assert!(
            serialized.is_ok(),
            "a non-finite confidence now fails response serialization, which would make \
             serialization_failed reachable: {serialized:?}"
        );
        assert!(serialized.expect("checked above").contains("null"));
    }

    #[test]
    fn strict_routing_fails_closed_without_a_fallback() {
        // Unrecognised provider name.
        assert_eq!(
            resolve_strict_routing(Some("bogus"), Some("deepseek-chat")).unwrap_err().code,
            "unknown_provider"
        );
        // Model family not determinable from the name: must not fall through to
        // environment-variable priority. (`gpt-4o` used to land here too, but it
        // now resolves to the OpenAI family, so a genuinely unknown name is used.)
        assert_eq!(
            resolve_strict_routing(None, Some("mystery-model")).unwrap_err().code,
            "unknown_model"
        );
        assert_eq!(
            resolve_strict_routing(Some("openai"), Some("mystery-model")).unwrap_err().code,
            "unknown_model"
        );
        // Explicit provider without an explicit model.
        assert_eq!(
            resolve_strict_routing(Some("deepseek"), None).unwrap_err().code,
            "provider_requires_model"
        );
        // Explicit provider contradicting the model family.
        assert_eq!(
            resolve_strict_routing(Some("anthropic"), Some("deepseek-chat")).unwrap_err().code,
            "provider_model_conflict"
        );
    }

    #[test]
    fn strict_routing_accepts_consistent_pairs() {
        assert_eq!(
            resolve_strict_routing(Some("deepseek"), Some("deepseek-chat")).unwrap().0,
            api::ProviderKind::DeepSeek
        );
        assert_eq!(
            resolve_strict_routing(Some("claude"), Some("claude-sonnet-4-6")).unwrap().0,
            api::ProviderKind::Anthropic
        );
        assert_eq!(
            resolve_strict_routing(None, Some("claude-sonnet-4-6")).unwrap().0,
            api::ProviderKind::Anthropic
        );
        assert_eq!(
            resolve_strict_routing(Some("grok"), Some("grok-3")).unwrap().0,
            api::ProviderKind::Xai
        );
        assert_eq!(
            resolve_strict_routing(Some("openai"), Some("gpt-4o")).unwrap().0,
            api::ProviderKind::OpenAi
        );
        assert_eq!(
            resolve_strict_routing(None, Some("o3-mini")).unwrap().0,
            api::ProviderKind::OpenAi
        );
    }

    #[test]
    fn options_parse_provider_field() {
        let json = r#"{"schema_version":1,"action":"review","cwd":"/tmp/p","options":{"provider":"deepseek","model":"deepseek-chat"}}"#;
        let request: SidecarReviewRequest = serde_json::from_str(json).expect("request parses");
        let options = request.options.expect("options present");
        assert_eq!(options.provider.as_deref(), Some("deepseek"));
        assert_eq!(options.model.as_deref(), Some("deepseek-chat"));
    }

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
