//! End-to-end review pipeline against a mock model endpoint (DEV-QA-05).
//!
//! Covers the whole chain in one run, rather than the stages separately:
//!
//! ```text
//! envelope on stdin -> provider call at the mock endpoint -> parse
//!   -> evidence gate -> persist (artifact + index) -> response on stdout
//! ```
//!
//! The mock returns the findings document when it sees the
//! `PARITY_SCENARIO:review_findings` marker, which this test places inside the
//! staged diff - so the marker travels with the reviewed content into the review
//! prompt, and no product code needs to know the test exists. The scenario's
//! citation (`src/lib.rs` line 2) is the line this test adds.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use mock_anthropic_service::MockAnthropicService;
use serde_json::Value;

const SCENARIO_MARKER: &str = "PARITY_SCENARIO:review_findings";

fn temp_workspace(name: &str) -> PathBuf {
    let unique =
        SystemTime::now().duration_since(UNIX_EPOCH).expect("time should move forward").as_nanos();
    let root = std::env::temp_dir().join(format!("sego-e2e-{name}-{unique}"));
    fs::create_dir_all(&root).expect("create workspace");
    root
}

fn git(args: &[&str], cwd: &Path) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .expect("git should be runnable");
    assert!(status.success(), "git {args:?} failed in {}", cwd.display());
}

/// The environment a child needs on this platform. The test clears the
/// environment first, so the essentials have to be put back explicitly.
fn apply_platform_process_env(command: &mut Command) {
    #[cfg(windows)]
    {
        for key in ["ComSpec", "PATH", "Path", "PATHEXT", "SystemRoot", "TEMP", "TMP", "WINDIR"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
    }
    #[cfg(not(windows))]
    {
        command.env("PATH", "/usr/bin:/bin");
    }
}

/// A workspace with one staged change: line 2 of `src/lib.rs` is the line the
/// mock's findings document cites.
fn staged_workspace(root: &Path) {
    let src = root.join("src");
    fs::create_dir_all(&src).expect("create src dir");
    fs::write(src.join("lib.rs"), "fn guard() -> bool { true }\nfn validate() -> bool { true }\n")
        .expect("write base file");

    git(&["init", "--quiet"], root);
    git(&["config", "user.email", "tests@example.com"], root);
    git(&["config", "user.name", "Sego Tests"], root);
    git(&["add", "src/lib.rs"], root);
    git(&["commit", "-m", "base", "--quiet"], root);

    // The reviewed change: the added comment carries the scenario marker, and it
    // lands on the line the mock reports a finding against.
    fs::write(
        src.join("lib.rs"),
        format!(
            "fn guard() -> bool {{ true }}\n// {SCENARIO_MARKER}\nfn validate() -> bool {{ true }}\n"
        ),
    )
    .expect("write staged change");
    git(&["add", "src/lib.rs"], root);
}

#[test]
fn sidecar_review_runs_the_whole_pipeline_against_a_mock_endpoint() {
    let runtime =
        tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("build runtime");
    let mock = runtime.block_on(MockAnthropicService::spawn()).expect("spawn mock service");
    let base_url = mock.base_url();

    let root = temp_workspace("sidecar-review");
    let home = root.join("home");
    let config_home = root.join("config-home");
    fs::create_dir_all(&home).expect("home");
    fs::create_dir_all(&config_home).expect("config home");
    staged_workspace(&root);

    let envelope = serde_json::json!({
        "schema_version": 1,
        "action": "review",
        "cwd": root.to_string_lossy(),
        "scope": "staged",
        "options": { "model": "sonnet" },
        "context": { "invocation_id": "inv-e2e-001" }
    });

    let mut command = Command::new(env!("CARGO_BIN_EXE_sego"));
    command
        .current_dir(&root)
        .env_clear()
        .env("ANTHROPIC_API_KEY", "test-e2e-key")
        .env("ANTHROPIC_BASE_URL", &base_url)
        .env("CLAW_CONFIG_HOME", &config_home)
        .env("HOME", &home)
        .env("NO_COLOR", "1")
        .args(["sidecar", "review"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_platform_process_env(&mut command);

    let mut child = command.spawn().expect("spawn sego sidecar review");
    {
        let stdin = child.stdin.take().expect("stdin pipe");
        serde_json::to_writer(stdin, &envelope).expect("write envelope");
    }
    let output = child.wait_with_output().expect("wait for sego");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "sidecar review failed\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );

    // Stage 1-2: the provider was really called, and the reviewed content
    // travelled into the prompt.
    let requests = runtime.block_on(mock.captured_requests());
    assert!(!requests.is_empty(), "the mock endpoint was never called");
    assert!(
        requests.iter().any(|request| request.path.contains("/v1/messages")),
        "expected a messages call, saw: {:?}",
        requests.iter().map(|request| request.path.clone()).collect::<Vec<_>>()
    );
    assert!(
        requests.iter().any(|request| request.raw_body.contains(SCENARIO_MARKER)),
        "the reviewed diff did not reach the model: {}",
        requests.first().map(|request| request.raw_body.clone()).unwrap_or_default()
    );

    // The response envelope.
    let response: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|error| panic!("response is not JSON ({error}):\n{stdout}"));
    assert_eq!(response["schema_version"], 1, "{response}");
    assert_eq!(response["status"], "ok", "{response}");

    // Stage 3: the model's findings document parsed structurally.
    assert_eq!(response["parse_status"], "structured", "{response}");

    // Stage 5: the artifact was persisted.
    let artifact_path = response["artifact_path"].as_str().expect("artifact_path in response");
    let artifact_path = if Path::new(artifact_path).is_absolute() {
        PathBuf::from(artifact_path)
    } else {
        root.join(artifact_path)
    };
    let artifact: Value =
        serde_json::from_str(&fs::read_to_string(&artifact_path).expect("read artifact"))
            .expect("artifact is JSON");

    assert_eq!(artifact["schema_version"], 1);
    assert_eq!(artifact["parse_status"], "structured");
    assert_eq!(artifact["finding_count"], 1, "{artifact}");
    // Input binding: the artifact and the response describe the same review, and
    // the caller's invocation id was carried through opaquely.
    assert_eq!(artifact["diff_hash"], response["diff_hash"], "artifact and response disagree");
    assert_eq!(artifact["invocation_id"], "inv-e2e-001");
    assert_eq!(artifact["id"], response["review_id"]);

    // Stage 4: the evidence gate ran and recorded what it could and could not
    // capture, and each finding carries a status rather than being dropped.
    assert!(
        artifact.get("evidence_coverage").is_some(),
        "the evidence gate recorded no coverage: {artifact}"
    );
    let findings = artifact["findings"].as_array().expect("findings array");
    assert_eq!(findings.len(), 1);
    let finding = &findings[0];
    assert_eq!(finding["severity"], "high", "{finding}");
    assert_eq!(finding["file"], "src/lib.rs");
    let evidence_status =
        finding["evidence_status"].as_str().expect("each finding carries an evidence status");
    // This citation is inside the captured diff, so the gate can resolve it;
    // asserting the resolved value (not merely "some string") is what makes this
    // an end-to-end claim about the gate.
    assert_eq!(
        evidence_status, "verified",
        "the citation is within the captured diff and should verify: {finding}"
    );

    // The index line was appended, so the artifact is discoverable the way the
    // CLI and consumers look for it.
    let index_path = root.join(".sego").join("reviews").join("index.jsonl");
    let index = fs::read_to_string(&index_path).expect("read review index");
    let review_id = response["review_id"].as_str().expect("review_id");
    assert!(
        index.lines().any(|line| line.contains(review_id)),
        "the index does not reference {review_id}:\n{index}"
    );

    let _ = fs::remove_dir_all(&root);
}
