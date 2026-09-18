//! Integration coverage for the **text** model-turn path (DEV-STRUCT-01 step 3,
//! `SEG-DEV-001` §1.23 class D).
//!
//! Why this is not in `mock_parity_harness`: that file records upstream *parity*
//! scenarios, and its `run_case` always passes `--output-format=json`. There,
//! `run_turn_with_output` only ever takes its `Json` arm (which delegates to
//! `run_prompt_json`); the `Text` arm - which delegates to `run_turn`, the plain
//! conversational turn - had no integration coverage at all. Driving it is not a
//! parity claim about the upstream tool, so it lives here, against the same mock
//! endpoint and the same real binary.
//!
//! Both cases run the binary the way a user would: a positional prompt, the text
//! output format, and (in the second case) an answer typed at the permission
//! prompt over stdin.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mock_anthropic_service::{MockAnthropicService, SCENARIO_PREFIX};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A throwaway workspace the binary can be pointed at: it writes its session
/// under `.claw/`, and reads config from `config-home` and `home`.
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
            .join(format!("claw-mock-turns-{label}-{}-{millis}-{counter}", std::process::id()));
        let config_home = root.join("config-home");
        let home = root.join("home");
        fs::create_dir_all(&config_home).expect("config home should be created");
        fs::create_dir_all(&home).expect("home should be created");
        Self { root, config_home, home }
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        // Best effort on purpose: a temp root can still be held open while a
        // child process finishes closing its handles, and a cleanup must not be
        // able to fail a test that already passed (AGENTS.md §5).
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

/// Run the real binary in the **text** output format - no `--output-format`.
fn run_text_turn(
    workspace: &Workspace,
    base_url: &str,
    scenario: &str,
    extra_args: &[&str],
    stdin: Option<&str>,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sego"));
    command
        .current_dir(&workspace.root)
        .env_clear()
        .env("ANTHROPIC_API_KEY", "test-dummy-key-for-text-turn")
        .env("ANTHROPIC_BASE_URL", base_url)
        .env("CLAW_CONFIG_HOME", &workspace.config_home)
        .env("HOME", &workspace.home)
        .env("NO_COLOR", "1")
        .args(["--model", "sonnet"])
        .args(extra_args)
        .arg(format!("{SCENARIO_PREFIX}{scenario}"));
    apply_platform_process_env(&mut command);

    match stdin {
        Some(text) => {
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
                .write_all(text.as_bytes())
                .expect("stdin should write");
            child.wait_with_output().expect("sego should finish")
        }
        None => command.output().expect("sego should launch"),
    }
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
fn text_turn_renders_the_answer_rather_than_the_json_envelope() {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime should build");
    let server =
        runtime.block_on(MockAnthropicService::spawn()).expect("mock service should start");
    let workspace = Workspace::create("plain");

    let output = run_text_turn(
        &workspace,
        &server.base_url(),
        "streaming_text",
        &["--permission-mode", "read-only"],
        None,
    );
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("Mock streaming says hello from the parity harness."),
        "the text format must render the answer itself:\n{stdout}"
    );

    // The JSON envelope exists only on the other arm of `run_turn_with_output`.
    // If these keys appear here, the output-format flag no longer switches the
    // rendering, and every script that parses one format would silently start
    // reading the other.
    for envelope_key in ["\"estimated_cost\"", "\"auto_compaction\"", "\"iterations\""] {
        assert!(
            !stdout.contains(envelope_key),
            "the text format must not print the JSON envelope, but saw {envelope_key}:\n{stdout}"
        );
    }
}

#[test]
fn text_turn_asks_for_permission_on_stdin_and_reports_the_approved_result() {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime should build");
    let server =
        runtime.block_on(MockAnthropicService::spawn()).expect("mock service should start");
    let workspace = Workspace::create("approved");

    let output = run_text_turn(
        &workspace,
        &server.base_url(),
        "bash_permission_prompt_approved",
        &["--permission-mode", "workspace-write", "--allowedTools", "bash"],
        Some("y\n"),
    );
    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // The prompt has to be readable in the text format too - in JSON mode the
    // answer is typed at the same prompt, so a format that hid it would leave the
    // user with a request they cannot see and a turn that never finishes.
    assert!(
        stdout.contains("Approve this tool call? [y/N]:"),
        "the text path must prompt for approval on stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("approved and executed"),
        "the approved turn must report the assistant's answer:\n{stdout}"
    );
}
