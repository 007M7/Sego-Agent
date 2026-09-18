//! Integration coverage for the REPL loop - `SEG-DEV-001` §1.23 class F.
//!
//! `run_repl` was the last uncovered piece of the CLI's interactive surface, and
//! it is the surface a user actually lives in. It is drivable without a terminal
//! on purpose: `input::LineEditor::read_line` checks `is_terminal()` and falls
//! back to reading a line from stdin, with EOF treated as exit. That fallback is
//! a designed path, not an accident, which is what makes this test possible.
//!
//! Two cases: a read-only session that inspects and leaves, and one that drives a
//! model turn through the REPL (`run_repl` -> `run_turn`), which nothing covered
//! before.
//!
//! Note on acceptance: `SEG-DEV-001` §1.23's "coverage must rise" criterion is
//! **unsatisfiable for this class** - executing the binary as a subprocess
//! contributes 0.00% to the crate's coverage (recorded in §1.30). What this test
//! is worth rests on the assertions below, not on that number.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mock_anthropic_service::{MockAnthropicService, SCENARIO_PREFIX};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Workspace {
    root: PathBuf,
    config_home: PathBuf,
    home: PathBuf,
}

impl Workspace {
    fn create(label: &str) -> Self {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_millis();
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir()
            .join(format!("claw-mock-repl-{label}-{}-{millis}-{counter}", std::process::id()));
        let config_home = root.join("config-home");
        let home = root.join("home");
        fs::create_dir_all(&config_home).expect("config home should be created");
        fs::create_dir_all(&home).expect("home should be created");
        Self { root, config_home, home }
    }

    /// The recovery record the REPL writes, read through the product's own path
    /// helpers rather than a hard-coded one.
    fn exit_state(&self) -> Option<serde_json::Value> {
        let path =
            runtime::recovery::recovery_dir(&self.root).join(runtime::recovery::EXIT_STATE_FILE);
        let text = fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        // Best effort on purpose: a cleanup must not be able to fail a test that
        // already passed (AGENTS.md §5).
        let _ = fs::remove_dir_all(&self.root);
    }
}

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

/// Start the REPL and type `script` at it.
///
/// No positional prompt is passed, because that is what selects the REPL:
/// `parse_args` returns `CliAction::Repl` exactly when no positional argument
/// remains.
fn run_repl_session(
    workspace: &Workspace,
    base_url: &str,
    script: &str,
    extra_args: &[&str],
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sego"));
    command
        .current_dir(&workspace.root)
        .env_clear()
        .env("ANTHROPIC_API_KEY", "test-dummy-key-for-repl")
        .env("ANTHROPIC_BASE_URL", base_url)
        .env("CLAW_CONFIG_HOME", &workspace.config_home)
        .env("HOME", &workspace.home)
        .env("NO_COLOR", "1")
        .args(["--model", "sonnet"])
        .args(extra_args);
    apply_platform_process_env(&mut command);

    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("sego should launch");
    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(script.as_bytes())
        .expect("stdin should write");
    child.wait_with_output().expect("sego should finish")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn repl_reports_the_session_and_records_a_graceful_exit() {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime should build");
    let server =
        runtime.block_on(MockAnthropicService::spawn()).expect("mock service should start");
    let workspace = Workspace::create("status");

    let output = run_repl_session(
        &workspace,
        &server.base_url(),
        "/status\n/exit\n",
        &["--permission-mode", "read-only"],
    );
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("Model"), "the startup banner must be printed:\n{stdout}");
    assert!(stdout.contains("Auto-save"), "the banner must name the auto-save target:\n{stdout}");

    // `/status` has to report the mode the session is actually in. A status line
    // that drifted from the enforced mode would tell the user the agent may do
    // less - or more - than it can.
    assert!(stdout.contains("Status"), "/status must render its report:\n{stdout}");
    assert!(
        stdout.contains("Permission mode  read-only"),
        "the status report must state the mode in force:\n{stdout}"
    );

    // Leaving cleanly means there is nothing to recover. The next launch reads
    // exactly this record to decide whether to offer recovery, so a REPL that
    // exited on /exit must not leave `active` behind and send the next session
    // chasing a crash that never happened.
    let state = workspace.exit_state().expect("the REPL must write its exit state");
    assert_eq!(
        state["state"], "graceful",
        "a clean /exit must be recorded as graceful, got: {state}"
    );
    assert!(
        state["session_id"].as_str().is_some_and(|id| !id.is_empty()),
        "the record must name the session it belongs to: {state}"
    );
}

#[test]
fn repl_runs_a_model_turn_and_renders_the_answer() {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime should build");
    let server =
        runtime.block_on(MockAnthropicService::spawn()).expect("mock service should start");
    let workspace = Workspace::create("turn");

    // A plain prompt typed at the REPL: `run_repl` routes it to `run_turn`, which
    // no test reached through this entry point before.
    let script = format!("{SCENARIO_PREFIX}streaming_text\n/exit\n");
    let output = run_repl_session(
        &workspace,
        &server.base_url(),
        &script,
        &["--permission-mode", "read-only"],
    );
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("Mock streaming says hello from the parity harness."),
        "a prompt typed at the REPL must reach the model and render its answer:\n{stdout}"
    );

    // The turn ran, so the session it belongs to left a clean record.
    let state = workspace.exit_state().expect("the REPL must write its exit state");
    assert_eq!(state["state"], "graceful", "the turn must not disturb the exit record: {state}");
}
