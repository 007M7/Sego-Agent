//! Tests for the CLI binary (moved out of `main.rs`).
//!
//! A move, not a rewrite: same module name, same imports, `super` still
//! the crate root. The `#[cfg(test)]` helpers these tests use stay in
//! `main.rs`, so nothing in the import list had to change.

use super::{
    build_review_summary_json_value, build_runtime_plugin_state_with_loader,
    build_runtime_with_plugin_state, create_managed_session_handle, default_model,
    describe_tool_progress, display_path_for_user, filter_tool_specs, format_bughunter_report,
    format_commit_preflight_report, format_commit_skipped_report, format_compact_report,
    format_cost_report, format_internal_prompt_progress_line, format_issue_report,
    format_model_report, format_model_switch_report, format_permissions_report,
    format_permissions_switch_report, format_pr_report, format_resume_report,
    format_review_completion_summary, format_status_report, format_tool_call_start,
    format_tool_result, format_ultraplan_report, format_unknown_slash_command,
    format_unknown_slash_command_message, generate_review_card_for, is_git_worktree,
    is_no_assistant_response_export_error, is_turn_cancelled_message, latest_review_index_entry,
    non_git_review_error, normalize_permission_mode, parse_args, parse_git_status_branch,
    parse_git_status_metadata_for, parse_git_workspace_summary, permission_policy, print_help_to,
    push_output_block, render_code_review_readiness_for, render_code_review_summary_for,
    render_config_report, render_diff_report, render_diff_report_for, render_memory_report,
    render_natural_language_directory, render_repl_help, render_resume_usage, resolve_model_alias,
    resolve_review_entry, resolve_session_reference, response_to_events,
    resume_supported_slash_commands, run_resume_command,
    slash_command_completion_candidates_with_sessions, status_context, validate_no_args,
    workspace_context, write_mcp_server_fixture, CliAction, CliOutputFormat, CliToolExecutor,
    GitWorkspaceSummary, InternalPromptProgressEvent, InternalPromptProgressState, LiveCli,
    NoAssistantResponseExportError, ReviewFindingStatusCounts, SafetyReviewScope, SlashCommand,
    StatusUsage, TURN_CANCELLED_MESSAGE,
};
use api::{MessageResponse, OutputContentBlock, Usage};
use plugins::{
    PluginManager, PluginManagerConfig, PluginTool, PluginToolDefinition, PluginToolPermission,
};
use runtime::ReviewFindingStatus;
use runtime::ReviewIndexEntry;
use runtime::{
    AssistantEvent, ConfigLoader, ContentBlock, ConversationMessage, MessageRole, PermissionMode,
    Session, ToolExecutor,
};
use serde_json::json;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tools::GlobalToolRegistry;

#[test]
fn detects_no_assistant_response_export_error_through_source_chain() {
    #[derive(Debug)]
    struct WrappedExportError(NoAssistantResponseExportError);

    impl std::fmt::Display for WrappedExportError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("wrapped export error")
        }
    }

    impl std::error::Error for WrappedExportError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    let direct = NoAssistantResponseExportError;
    assert!(is_no_assistant_response_export_error(&direct));

    let wrapped = WrappedExportError(NoAssistantResponseExportError);
    assert!(is_no_assistant_response_export_error(&wrapped));

    let unrelated = std::io::Error::other("other");
    assert!(!is_no_assistant_response_export_error(&unrelated));
}

fn registry_with_plugin_tool() -> GlobalToolRegistry {
    GlobalToolRegistry::with_plugin_tools(vec![PluginTool::new(
        "plugin-demo@external",
        "plugin-demo",
        PluginToolDefinition {
            name: "plugin_echo".to_string(),
            description: Some("Echo plugin payload".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" }
                },
                "required": ["message"],
                "additionalProperties": false
            }),
        },
        "echo".to_string(),
        Vec::new(),
        PluginToolPermission::WorkspaceWrite,
        None,
    )])
    .expect("plugin tool registry should build")
}

fn temp_dir() -> PathBuf {
    // A clock reading is not a value: two tests can read the same nanosecond and
    // then share one temp root, which is how a cleanup in one test deleted a
    // directory another test was still using. The counter is what makes the name
    // unique per call, which is what this function promises.
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be after epoch")
        .as_nanos();
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("rusty-claude-cli-{nanos}-{sequence}"))
}

struct EnvRestore {
    values: Vec<(&'static str, Option<String>)>,
}

impl EnvRestore {
    fn isolate(root: &Path) -> Self {
        let vars = [
            "HOME",
            "USERPROFILE",
            "CLAW_CONFIG_HOME",
            "RUSTY_CLAUDE_PERMISSION_MODE",
            "DEEPSEEK_MODEL",
            "ANTHROPIC_MODEL",
        ];
        let values = vars.into_iter().map(|var| (var, std::env::var(var).ok())).collect();
        let home = root.join("home");
        let config_home = root.join("config-home");
        fs::create_dir_all(&home).expect("home dir");
        fs::create_dir_all(&config_home).expect("config home dir");
        std::env::set_var("HOME", &home);
        std::env::set_var("USERPROFILE", &home);
        std::env::set_var("CLAW_CONFIG_HOME", &config_home);
        std::env::remove_var("RUSTY_CLAUDE_PERMISSION_MODE");
        std::env::remove_var("DEEPSEEK_MODEL");
        std::env::remove_var("ANTHROPIC_MODEL");
        Self { values }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (var, value) in self.values.drain(..) {
            match value {
                Some(value) => std::env::set_var(var, value),
                None => std::env::remove_var(var),
            }
        }
    }
}

/// Set one environment variable and put it back on drop.
///
/// `EnvRestore` covers the variables it is told to isolate; this covers the ones
/// a test injects itself. Without it a test that fails midway leaves its dummy
/// credential behind, and the next test then passes or fails depending on the
/// order the harness happened to pick - which is a false signal in both
/// directions. Any test that sets a variable another test reads must hold
/// `env_lock`, or the lock is held on one side only and is not isolation.
struct EnvVarGuard {
    name: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    fn set(name: &'static str, value: &str) -> Self {
        let previous = std::env::var(name).ok();
        std::env::set_var(name, value);
        Self { name, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(self.name, value),
            None => std::env::remove_var(self.name),
        }
    }
}

fn python_command() -> String {
    std::env::var("PYTHON").unwrap_or_else(|_| {
        if cfg!(windows) {
            "python".to_string()
        } else {
            "python3".to_string()
        }
    })
}

/// True when the interpreter this host would use actually runs.
///
/// Resolving a name is not the same as having a working interpreter: on
/// macOS `/usr/bin/python3` can be a shim that refuses to run, and on
/// Windows the name may resolve only to the Store alias stub. Tests whose
/// fixture is a Python script cannot tell that apart from the product
/// failing to discover an MCP server, so they check first.
fn python_is_usable() -> bool {
    std::process::Command::new(python_command())
        .args(["-c", "print(1)"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[test]
fn identifies_turn_cancelled_errors_without_usage_hint() {
    assert!(is_turn_cancelled_message(TURN_CANCELLED_MESSAGE));
    assert!(is_turn_cancelled_message("runtime failed: conversation turn cancelled by user"));
    assert!(!is_turn_cancelled_message("missing DeepSeek credentials; set DEEPSEEK_API_KEY"));
}

fn git(args: &[&str], cwd: &Path) {
    let status =
        Command::new("git").args(args).current_dir(cwd).status().expect("git command should run");
    assert!(status.success(), "git command failed: git {}", args.join(" "));
}

fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn with_current_dir<T>(cwd: &Path, f: impl FnOnce() -> T) -> T {
    let _guard = cwd_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let previous = std::env::current_dir().expect("cwd should load");
    std::env::set_current_dir(cwd).expect("cwd should change");
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    std::env::set_current_dir(previous).expect("cwd should restore");
    match result {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn write_plugin_fixture(root: &Path, name: &str, include_hooks: bool, include_lifecycle: bool) {
    fs::create_dir_all(root.join(".claude-plugin")).expect("manifest dir");
    let (hook_command, hook_body) = plugin_fixture_hook();
    let (init_command, init_body, shutdown_command, shutdown_body) = plugin_fixture_lifecycle();
    if include_hooks {
        fs::create_dir_all(root.join("hooks")).expect("hooks dir");
        fs::write(root.join(hook_command.trim_start_matches("./")), hook_body).expect("write hook");
    }
    if include_lifecycle {
        fs::create_dir_all(root.join("lifecycle")).expect("lifecycle dir");
        fs::write(root.join(init_command.trim_start_matches("./")), init_body)
            .expect("write init lifecycle");
        fs::write(root.join(shutdown_command.trim_start_matches("./")), shutdown_body)
            .expect("write shutdown lifecycle");
    }

    let hooks = if include_hooks {
        format!(",\n  \"hooks\": {{\n    \"PreToolUse\": [\"{hook_command}\"]\n  }}")
    } else {
        String::new()
    };
    let lifecycle = if include_lifecycle {
        format!(
                ",\n  \"lifecycle\": {{\n    \"Init\": [\"{init_command}\"],\n    \"Shutdown\": [\"{shutdown_command}\"]\n  }}"
            )
    } else {
        String::new()
    };
    fs::write(
            root.join(".claude-plugin").join("plugin.json"),
            format!(
                "{{\n  \"name\": \"{name}\",\n  \"version\": \"1.0.0\",\n  \"description\": \"runtime plugin fixture\"{hooks}{lifecycle}\n}}"
            ),
        )
        .expect("write plugin manifest");
}

#[cfg(windows)]
fn plugin_fixture_hook() -> (&'static str, &'static str) {
    ("./hooks/pre.cmd", "@echo off\r\necho plugin pre hook\r\n")
}

#[cfg(not(windows))]
fn plugin_fixture_hook() -> (&'static str, &'static str) {
    ("./hooks/pre.sh", "#!/bin/sh\nprintf 'plugin pre hook'\n")
}

#[cfg(windows)]
fn plugin_fixture_lifecycle() -> (&'static str, &'static str, &'static str, &'static str) {
    (
        "./lifecycle/init.cmd",
        "@echo off\r\n>> lifecycle.log echo init\r\n",
        "./lifecycle/shutdown.cmd",
        "@echo off\r\n>> lifecycle.log echo shutdown\r\n",
    )
}

#[cfg(not(windows))]
fn plugin_fixture_lifecycle() -> (&'static str, &'static str, &'static str, &'static str) {
    (
        "./lifecycle/init.sh",
        "#!/bin/sh\nprintf 'init\\n' >> lifecycle.log\n",
        "./lifecycle/shutdown.sh",
        "#!/bin/sh\nprintf 'shutdown\\n' >> lifecycle.log\n",
    )
}
#[test]
fn defaults_to_repl_when_no_args() {
    let _guard = env_lock();
    std::env::remove_var("RUSTY_CLAUDE_PERMISSION_MODE");
    assert_eq!(
        parse_args(&[]).expect("args should parse"),
        CliAction::Repl {
            model: default_model(),
            allowed_tools: None,
            permission_mode: PermissionMode::ReadOnly,
        }
    );
}

#[test]
fn default_permission_mode_uses_project_config_when_env_is_unset() {
    let _guard = env_lock();
    let root = temp_dir();
    let cwd = root.join("project");
    let config_home = root.join("config-home");
    std::fs::create_dir_all(cwd.join(".claw")).expect("project config dir should exist");
    std::fs::create_dir_all(&config_home).expect("config home should exist");
    std::fs::write(cwd.join(".claw").join("settings.json"), r#"{"permissionMode":"acceptEdits"}"#)
        .expect("project config should write");

    let original_config_home = std::env::var("CLAW_CONFIG_HOME").ok();
    let original_permission_mode = std::env::var("RUSTY_CLAUDE_PERMISSION_MODE").ok();
    std::env::set_var("CLAW_CONFIG_HOME", &config_home);
    std::env::remove_var("RUSTY_CLAUDE_PERMISSION_MODE");

    let resolved = with_current_dir(&cwd, super::default_permission_mode);

    match original_config_home {
        Some(value) => std::env::set_var("CLAW_CONFIG_HOME", value),
        None => std::env::remove_var("CLAW_CONFIG_HOME"),
    }
    match original_permission_mode {
        Some(value) => std::env::set_var("RUSTY_CLAUDE_PERMISSION_MODE", value),
        None => std::env::remove_var("RUSTY_CLAUDE_PERMISSION_MODE"),
    }
    let _ = std::fs::remove_dir_all(root);

    assert_eq!(resolved, PermissionMode::WorkspaceWrite);
}

#[test]
fn env_permission_mode_overrides_project_config_default() {
    let _guard = env_lock();
    let root = temp_dir();
    let cwd = root.join("project");
    let config_home = root.join("config-home");
    std::fs::create_dir_all(cwd.join(".claw")).expect("project config dir should exist");
    std::fs::create_dir_all(&config_home).expect("config home should exist");
    std::fs::write(cwd.join(".claw").join("settings.json"), r#"{"permissionMode":"acceptEdits"}"#)
        .expect("project config should write");

    let original_config_home = std::env::var("CLAW_CONFIG_HOME").ok();
    let original_permission_mode = std::env::var("RUSTY_CLAUDE_PERMISSION_MODE").ok();
    std::env::set_var("CLAW_CONFIG_HOME", &config_home);
    std::env::set_var("RUSTY_CLAUDE_PERMISSION_MODE", "read-only");

    let resolved = with_current_dir(&cwd, super::default_permission_mode);

    match original_config_home {
        Some(value) => std::env::set_var("CLAW_CONFIG_HOME", value),
        None => std::env::remove_var("CLAW_CONFIG_HOME"),
    }
    match original_permission_mode {
        Some(value) => std::env::set_var("RUSTY_CLAUDE_PERMISSION_MODE", value),
        None => std::env::remove_var("RUSTY_CLAUDE_PERMISSION_MODE"),
    }
    let _ = std::fs::remove_dir_all(root);

    assert_eq!(resolved, PermissionMode::ReadOnly);
}

#[test]
fn parses_prompt_subcommand() {
    let _guard = env_lock();
    std::env::remove_var("RUSTY_CLAUDE_PERMISSION_MODE");
    let args = vec!["prompt".to_string(), "hello".to_string(), "world".to_string()];
    assert_eq!(
        parse_args(&args).expect("args should parse"),
        CliAction::Prompt {
            prompt: "hello world".to_string(),
            model: default_model(),
            output_format: CliOutputFormat::Text,
            allowed_tools: None,
            permission_mode: PermissionMode::ReadOnly,
        }
    );
}

#[test]
fn parses_bare_prompt_and_json_output_flag() {
    let _guard = env_lock();
    std::env::remove_var("RUSTY_CLAUDE_PERMISSION_MODE");
    let args = vec![
        "--output-format=json".to_string(),
        "--model".to_string(),
        "claude-opus".to_string(),
        "explain".to_string(),
        "this".to_string(),
    ];
    assert_eq!(
        parse_args(&args).expect("args should parse"),
        CliAction::Prompt {
            prompt: "explain this".to_string(),
            model: "claude-opus".to_string(),
            output_format: CliOutputFormat::Json,
            allowed_tools: None,
            permission_mode: PermissionMode::ReadOnly,
        }
    );
}

#[test]
fn resolves_model_aliases_in_args() {
    let _guard = env_lock();
    let env_root = temp_dir();
    let _env = EnvRestore::isolate(&env_root);
    let args =
        vec!["--model".to_string(), "opus".to_string(), "explain".to_string(), "this".to_string()];
    assert_eq!(
        parse_args(&args).expect("args should parse"),
        CliAction::Prompt {
            prompt: "explain this".to_string(),
            model: "claude-opus-4-7".to_string(),
            output_format: CliOutputFormat::Text,
            allowed_tools: None,
            permission_mode: PermissionMode::ReadOnly,
        }
    );
}

#[test]
fn resolves_known_model_aliases() {
    let _guard = env_lock();
    let env_root = temp_dir();
    let _env = EnvRestore::isolate(&env_root);
    assert_eq!(resolve_model_alias("deepseek"), "deepseek");
    assert_eq!(resolve_model_alias("ds"), "ds");
    assert_eq!(resolve_model_alias("deepseek-pro"), "deepseek-pro");
    assert_eq!(resolve_model_alias("mimo"), "mimo");
    assert_eq!(resolve_model_alias("gpt"), "gpt");
    assert_eq!(resolve_model_alias("unknown-model"), "unknown-model");
}

#[test]
fn parses_version_flags_without_initializing_prompt_mode() {
    assert_eq!(
        parse_args(&["--version".to_string()]).expect("args should parse"),
        CliAction::Version
    );
    assert_eq!(parse_args(&["-V".to_string()]).expect("args should parse"), CliAction::Version);
}

#[test]
fn parses_update_command_without_initializing_prompt_mode() {
    assert_eq!(
        parse_args(&["update".to_string()]).expect("update should parse"),
        CliAction::Update { check_only: false }
    );
    assert_eq!(
        parse_args(&["update".to_string(), "--check".to_string()]).expect("update should parse"),
        CliAction::Update { check_only: true }
    );
}

#[test]
fn parses_workspace_command_aliases_without_initializing_prompt_mode() {
    for command in ["workspace", "workdir", "pwd", "cwd", "/workspace", "/pwd"] {
        assert_eq!(
            parse_args(&[command.to_string()]).expect("workspace command should parse"),
            CliAction::Workspace { output_format: CliOutputFormat::Text }
        );
    }
}

#[test]
fn parses_direct_dir_slash_command_without_prompt_mode() {
    assert_eq!(parse_args(&["/dir".to_string()]).expect("/dir should parse"), CliAction::Dir);
    let directory = render_natural_language_directory();
    assert!(directory.contains("Sego 常用动作目录"));
    assert!(directory.contains("切换到 D:\\YourProject"));
    assert!(directory.contains("把刚才的审查结果写成 E:\\code\\review.md"));
}

#[test]
fn parses_global_cwd_before_workspace_action() {
    let root = temp_dir();
    let workspace = root.join("中文项目");
    fs::create_dir_all(&workspace).expect("workspace dir");

    let action = with_current_dir(&root, || {
        parse_args(&["--cwd".to_string(), workspace.display().to_string(), "workspace".to_string()])
            .expect("args should parse")
    });

    let _ = fs::remove_dir_all(root);
    assert_eq!(action, CliAction::Workspace { output_format: CliOutputFormat::Text });
}

#[test]
fn compares_release_versions() {
    assert!(super::is_newer_version("v0.1.4", "0.1.3"));
    assert!(!super::is_newer_version("v0.1.3", "0.1.3"));
    assert!(!super::is_newer_version("v0.1.2", "0.1.3"));
}

#[test]
fn update_script_branches_on_every_step_and_can_roll_back() {
    let script = super::build_update_script(
        "v9.9.9",
        Path::new("C:\\sego\\sego.exe"),
        Path::new("C:\\sego\\sego.previous.exe"),
        Path::new("C:\\sego\\sego.update.exe"),
        "https://example.invalid/release",
    );

    // Each mutating step is followed by an exit-code check. The `move`
    // lines may or may not be preceded by a space, so match the bare token.
    let moves = script.matches("move /y \"").count();
    let checks = script.matches("if errorlevel 1 goto :").count();
    assert!(moves >= 3, "expected the swap steps to be present: {script}");
    assert!(
        checks >= 4,
        "every move and the smoke test need an error branch, found {checks}: {script}"
    );
    // The failure paths exist and lead somewhere terminal.
    for label in [":backup_failed", ":replace_failed", ":launch_failed", ":restore", ":failed"] {
        assert!(script.contains(label), "missing {label} in {script}");
    }
    // A rollback is actually attempted, not just announced.
    assert!(script.contains("move /y \"C:\\sego\\sego.previous.exe\" \"C:\\sego\\sego.exe\""));
    // Both outcomes report their status to the caller.
    assert!(script.contains("exit /b 1"));
    assert!(script.contains("exit /b 0"));
    // The tag and the manual download link are carried through.
    assert!(script.contains("v9.9.9"));
    assert!(script.contains("https://example.invalid/release"));
}

/// Minimal one-shot HTTP server, enough to serve a `checksums.txt`.
fn serve_checksums_once(body: String) -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read as _, Write as _};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind checksums server");
    let addr = listener.local_addr().expect("addr");
    let handle = std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (format!("http://{addr}/checksums.txt"), handle)
}

/// A release whose only published asset is a `checksums.txt` at `url`.
fn release_with_checksums(url: String) -> super::GithubRelease {
    super::GithubRelease {
        tag_name: "v9.9.9".to_string(),
        html_url: "https://example.invalid/release".to_string(),
        assets: vec![super::GithubReleaseAsset {
            name: super::UPDATE_CHECKSUMS_ASSET.to_string(),
            browser_download_url: url,
        }],
    }
}

// Every test below shares the environment lock: one of them sets an override
// variable, and without serialising them it would leak into a sibling and
// turn "refused" into "allowed".
#[test]
fn update_verification_refuses_and_discards_a_tampered_download() {
    let _guard = env_lock();
    let root = temp_dir();
    fs::create_dir_all(&root).expect("root");
    let downloaded = root.join("sego.exe");
    fs::write(&downloaded, b"tampered payload").expect("write tampered file");

    let (url, server) = serve_checksums_once(
        "0000000000000000000000000000000000000000000000000000000000000000  sego.exe\n".to_string(),
    );
    let release = release_with_checksums(url);

    let error = super::verify_release_checksum(&release, "sego.exe", &downloaded)
        .expect_err("a mismatched hash must be refused");
    assert!(format!("{error}").contains("checksum mismatch"), "{error}");
    assert!(
        !downloaded.exists(),
        "the tampered download must be deleted, not left on disk to be run by hand"
    );

    let _ = server.join();
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn update_verification_refuses_a_checksums_file_without_the_asset() {
    let _guard = env_lock();
    let root = temp_dir();
    fs::create_dir_all(&root).expect("root");
    let downloaded = root.join("sego.exe");
    fs::write(&downloaded, b"payload").expect("write file");

    // The listing is well-formed but never mentions the binary.
    let (url, server) = serve_checksums_once(
        "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef  sego-windows.zip\n"
            .to_string(),
    );
    let release = release_with_checksums(url);

    let error = super::verify_release_checksum(&release, "sego.exe", &downloaded)
        .expect_err("an unlisted asset must be refused");
    assert!(format!("{error}").contains("does not list"), "{error}");
    assert!(!downloaded.exists(), "an unverifiable download must be discarded");

    let _ = server.join();
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn update_verification_accepts_a_matching_download() {
    let _guard = env_lock();
    let root = temp_dir();
    fs::create_dir_all(&root).expect("root");
    let downloaded = root.join("sego.exe");
    fs::write(&downloaded, b"genuine payload").expect("write file");

    let digest = super::sha256_of_file(&downloaded).expect("hash the file");
    let (url, server) = serve_checksums_once(format!("{digest}  sego.exe\n"));
    let release = release_with_checksums(url);

    super::verify_release_checksum(&release, "sego.exe", &downloaded)
        .expect("a matching hash must be accepted");
    assert!(downloaded.exists(), "a verified download must be kept");

    let _ = server.join();
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn update_verification_skips_only_with_the_explicit_override() {
    let _guard = env_lock();
    let root = temp_dir();
    fs::create_dir_all(&root).expect("root");
    let downloaded = root.join("sego.exe");
    std::fs::write(&downloaded, b"payload").expect("write file");

    // No checksums asset at all: refused by default...
    let release = super::GithubRelease {
        tag_name: "v9.9.9".to_string(),
        html_url: "https://example.invalid/release".to_string(),
        assets: Vec::new(),
    };
    let error = super::verify_release_checksum(&release, "sego.exe", &downloaded)
        .expect_err("a release without checksums must be refused by default");
    assert!(format!("{error}").contains("does not publish"), "{error}");

    // ...and skipped only when the override is set explicitly.
    std::env::set_var(super::UPDATE_ALLOW_UNVERIFIED_ENV, "1");
    let skipped = super::verify_release_checksum(&release, "sego.exe", &downloaded);
    std::env::remove_var(super::UPDATE_ALLOW_UNVERIFIED_ENV);
    assert!(skipped.is_ok(), "the documented override must be honoured: {skipped:?}");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn reads_expected_hash_from_a_checksums_listing() {
    let listing = "\
0a1b2c3d4e5f60718293a4b5c6d7e8f90011223344556677889900aabbccddee  sego.exe
fe1b2c3d4e5f60718293a4b5c6d7e8f90011223344556677889900aabbccddee  sego-windows.zip
1234567890abcdef  sego
abcdef1234567890  sego-macos
";
    assert_eq!(
        super::expected_hash_for(listing, "sego.exe").as_deref(),
        Some("0a1b2c3d4e5f60718293a4b5c6d7e8f90011223344556677889900aabbccddee")
    );
    assert_eq!(
        super::expected_hash_for(listing, "sego-macos").as_deref(),
        Some("abcdef1234567890")
    );
    // `sha256sum -b` marks binary mode with a leading asterisk.
    assert_eq!(
        super::expected_hash_for("deadbeef  *sego.exe\n", "sego.exe").as_deref(),
        Some("deadbeef")
    );
    // Case is normalised so a listing in upper case still matches.
    assert_eq!(
        super::expected_hash_for("DEADBEEF  sego.exe\n", "sego.exe").as_deref(),
        Some("deadbeef")
    );
    // A missing entry is what makes the installer refuse, so it must be None.
    assert_eq!(super::expected_hash_for(listing, "not-shipped"), None);
    assert_eq!(super::expected_hash_for("", "sego.exe"), None);
}

#[test]
fn hashes_a_file_with_sha256() {
    let path = std::env::temp_dir().join(format!(
        "sego-sha256-test-{}.txt",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::write(&path, b"abc").expect("write fixture");
    let digest = super::sha256_of_file(&path).expect("hash should succeed");
    let _ = std::fs::remove_file(&path);
    // SHA-256("abc")
    assert_eq!(digest, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
}

#[test]
fn parses_permission_mode_flag() {
    let args = vec!["--permission-mode=read-only".to_string()];
    assert_eq!(
        parse_args(&args).expect("args should parse"),
        CliAction::Repl {
            model: default_model(),
            allowed_tools: None,
            permission_mode: PermissionMode::ReadOnly,
        }
    );
}

#[test]
fn parses_allowed_tools_flags_with_aliases_and_lists() {
    let _guard = env_lock();
    std::env::remove_var("RUSTY_CLAUDE_PERMISSION_MODE");
    let args = vec![
        "--allowedTools".to_string(),
        "read,glob".to_string(),
        "--allowed-tools=write_file".to_string(),
    ];
    assert_eq!(
        parse_args(&args).expect("args should parse"),
        CliAction::Repl {
            model: default_model(),
            allowed_tools: Some(
                ["glob_search", "read_file", "write_file"]
                    .into_iter()
                    .map(str::to_string)
                    .collect()
            ),
            permission_mode: PermissionMode::ReadOnly,
        }
    );
}

#[test]
fn rejects_unknown_allowed_tools() {
    let _guard = env_lock();
    let env_root = temp_dir();
    let _env = EnvRestore::isolate(&env_root);
    let error = parse_args(&["--allowedTools".to_string(), "teleport".to_string()])
        .expect_err("tool should be rejected");
    assert!(error.contains("unsupported tool in --allowedTools: teleport"));
}

#[test]
fn parses_system_prompt_options() {
    let args = vec![
        "system-prompt".to_string(),
        "--cwd".to_string(),
        "/tmp/project".to_string(),
        "--date".to_string(),
        "2026-04-01".to_string(),
    ];
    assert_eq!(
        parse_args(&args).expect("args should parse"),
        CliAction::PrintSystemPrompt {
            cwd: PathBuf::from("/tmp/project"),
            date: "2026-04-01".to_string(),
        }
    );
}

#[test]
fn parses_login_and_logout_subcommands() {
    let _guard = env_lock();
    let env_root = temp_dir();
    let _env = EnvRestore::isolate(&env_root);
    assert_eq!(parse_args(&["login".to_string()]).expect("login should parse"), CliAction::Login);
    assert_eq!(
        parse_args(&["logout".to_string()]).expect("logout should parse"),
        CliAction::Logout
    );
    assert_eq!(parse_args(&["init".to_string()]).expect("init should parse"), CliAction::Init);
    assert_eq!(
        parse_args(&["agents".to_string()]).expect("agents should parse"),
        CliAction::Agents { args: None }
    );
    assert_eq!(
        parse_args(&["mcp".to_string()]).expect("mcp should parse"),
        CliAction::Mcp { args: None }
    );
    assert_eq!(
        parse_args(&["skills".to_string()]).expect("skills should parse"),
        CliAction::Skills { args: None }
    );
    assert_eq!(
        parse_args(&["agents".to_string(), "--help".to_string()])
            .expect("agents help should parse"),
        CliAction::Agents { args: Some("--help".to_string()) }
    );
}

#[test]
fn parses_single_word_command_aliases_without_falling_back_to_prompt_mode() {
    let _guard = env_lock();
    std::env::remove_var("RUSTY_CLAUDE_PERMISSION_MODE");
    assert_eq!(parse_args(&["help".to_string()]).expect("help should parse"), CliAction::Help);
    assert_eq!(
        parse_args(&["version".to_string()]).expect("version should parse"),
        CliAction::Version
    );
    assert_eq!(
        parse_args(&["status".to_string()]).expect("status should parse"),
        CliAction::Status {
            model: default_model(),
            permission_mode: PermissionMode::ReadOnly,
            output_format: CliOutputFormat::Text,
        }
    );
    assert_eq!(
        parse_args(&["sandbox".to_string()]).expect("sandbox should parse"),
        CliAction::Sandbox { output_format: CliOutputFormat::Text }
    );
}

#[test]
fn single_word_slash_command_names_return_guidance_instead_of_hitting_prompt_mode() {
    let error = parse_args(&["cost".to_string()]).expect_err("cost should return guidance");
    assert!(error.contains("slash command"));
    assert!(error.contains("/cost"));
}

#[test]
fn multi_word_prompt_still_uses_shorthand_prompt_mode() {
    let _guard = env_lock();
    std::env::remove_var("RUSTY_CLAUDE_PERMISSION_MODE");
    assert_eq!(
        parse_args(&["help".to_string(), "me".to_string(), "debug".to_string()])
            .expect("prompt shorthand should still work"),
        CliAction::Prompt {
            prompt: "help me debug".to_string(),
            model: default_model(),
            output_format: CliOutputFormat::Text,
            allowed_tools: None,
            permission_mode: PermissionMode::ReadOnly,
        }
    );
}

#[test]
fn parses_direct_agents_mcp_and_skills_slash_commands() {
    let _guard = env_lock();
    let env_root = temp_dir();
    let _env = EnvRestore::isolate(&env_root);
    assert_eq!(
        parse_args(&["/agents".to_string()]).expect("/agents should parse"),
        CliAction::Agents { args: None }
    );
    assert_eq!(
        parse_args(&["/mcp".to_string(), "show".to_string(), "demo".to_string()])
            .expect("/mcp show demo should parse"),
        CliAction::Mcp { args: Some("show demo".to_string()) }
    );
    assert_eq!(
        parse_args(&["/skills".to_string()]).expect("/skills should parse"),
        CliAction::Skills { args: None }
    );
    assert_eq!(
        parse_args(&["/skills".to_string(), "help".to_string()])
            .expect("/skills help should parse"),
        CliAction::Skills { args: Some("help".to_string()) }
    );
    assert_eq!(
        parse_args(&[
            "/skills".to_string(),
            "install".to_string(),
            "./fixtures/help-skill".to_string(),
        ])
        .expect("/skills install should parse"),
        CliAction::Skills { args: Some("install ./fixtures/help-skill".to_string()) }
    );
    assert_eq!(
        parse_args(&["/review".to_string(), "staged".to_string()])
            .expect("/review staged should parse"),
        CliAction::CodeReview {
            scope: Some("staged".to_string()),
            model: default_model(),
            allowed_tools: None,
            permission_mode: PermissionMode::ReadOnly,
        }
    );
    assert_eq!(
        parse_args(&["/review".to_string(), "list".to_string()])
            .expect("/review list should parse"),
        CliAction::CodeReviewList
    );
    assert_eq!(
        parse_args(&["/review".to_string(), "tools".to_string()])
            .expect("/review tools should parse"),
        CliAction::CodeReviewTools
    );
    assert_eq!(
        parse_args(&["/review".to_string(), "ready".to_string()])
            .expect("/review ready should parse"),
        CliAction::CodeReviewReady
    );
    assert_eq!(
        parse_args(&["/review".to_string(), "summary".to_string()])
            .expect("/review summary should parse"),
        CliAction::CodeReviewSummary
    );
    assert_eq!(
        parse_args(&["/review".to_string(), "safety".to_string()])
            .expect("/review safety should parse"),
        CliAction::CodeReviewSafety { scope: SafetyReviewScope::Workspace }
    );
    assert_eq!(
        parse_args(&["/review".to_string(), "safety".to_string(), "staged".to_string()])
            .expect("/review safety staged should parse"),
        CliAction::CodeReviewSafety { scope: SafetyReviewScope::Staged }
    );
    assert_eq!(
        parse_args(&["/review".to_string(), "show".to_string(), "review-123".to_string(),])
            .expect("/review show should parse"),
        CliAction::CodeReviewShow { id: "review-123".to_string() }
    );
    assert_eq!(
        parse_args(&["/review".to_string(), "show".to_string(), "latest".to_string()])
            .expect("/review show latest should parse"),
        CliAction::CodeReviewShow { id: "latest".to_string() }
    );
    assert_eq!(
        parse_args(&[
            "/review".to_string(),
            "show".to_string(),
            "latest".to_string(),
            "--json".to_string(),
        ])
        .expect("/review show latest --json should parse"),
        CliAction::CodeReviewShowJson { id: "latest".to_string() }
    );
    assert_eq!(
        parse_args(&[
            "review".to_string(),
            "show".to_string(),
            "latest".to_string(),
            "--json".to_string(),
        ])
        .expect("sego review show latest --json should parse"),
        CliAction::CodeReviewShowJson { id: "latest".to_string() }
    );
    assert_eq!(
        parse_args(&["/review".to_string(), "status".to_string(), "review-123".to_string()])
            .expect("/review status should parse"),
        CliAction::CodeReviewStatus { id: "review-123".to_string() }
    );
    assert_eq!(
        parse_args(&[
            "/review".to_string(),
            "mark".to_string(),
            "review-123".to_string(),
            "finding-456".to_string(),
            "fixed".to_string(),
            "covered".to_string(),
            "by".to_string(),
            "tests".to_string(),
        ])
        .expect("/review mark should parse"),
        CliAction::CodeReviewMark {
            id: "review-123".to_string(),
            finding_id: "finding-456".to_string(),
            status: ReviewFindingStatus::Fixed,
            note: Some("covered by tests".to_string()),
        }
    );
    let summary_error =
        parse_args(&["/review".to_string(), "summary".to_string(), "extra".to_string()])
            .expect_err("/review summary should reject extra args");
    assert!(summary_error.contains("unexpected arguments for /review summary"));
    assert_eq!(
        parse_args(&["/verify".to_string(), "fast".to_string()])
            .expect("/verify fast should parse"),
        CliAction::CodeVerify { scope: Some("fast".to_string()) }
    );
    let error = parse_args(&["/status".to_string()])
        .expect_err("/status should remain REPL-only when invoked directly");
    assert!(error.contains("interactive-only"));
    assert!(error.contains("claw --resume SESSION.jsonl /status"));
}

#[test]
fn review_show_latest_resolves_newest_index_entry() {
    let older = ReviewIndexEntry {
        id: "review-old".to_string(),
        created_at_epoch_seconds: 10,
        scope: "staged".to_string(),
        diff_hash: "oldhash".to_string(),
        finding_count: 1,
        highest_severity: Some(runtime::ReviewSeverity::Low),
        parse_status: runtime::ReviewParseStatus::Structured,
        json_path: ".sego/reviews/review-old.json".to_string(),
        markdown_path: ".sego/reviews/review-old.md".to_string(),
    };
    let newer = ReviewIndexEntry {
        id: "review-new".to_string(),
        created_at_epoch_seconds: 20,
        scope: "workspace".to_string(),
        diff_hash: "newhash".to_string(),
        finding_count: 2,
        highest_severity: Some(runtime::ReviewSeverity::High),
        parse_status: runtime::ReviewParseStatus::Structured,
        json_path: ".sego/reviews/review-new.json".to_string(),
        markdown_path: ".sego/reviews/review-new.md".to_string(),
    };
    let entries = vec![newer.clone(), older];
    assert_eq!(latest_review_index_entry(&entries).unwrap().id, "review-new");
    assert_eq!(resolve_review_entry(&entries, "latest").unwrap().id, "review-new");
}

#[test]
fn review_show_latest_tie_breaks_same_timestamp_by_id() {
    let review_a = ReviewIndexEntry {
        id: "review-a".to_string(),
        created_at_epoch_seconds: 20,
        scope: "staged".to_string(),
        diff_hash: "hash-a".to_string(),
        finding_count: 1,
        highest_severity: Some(runtime::ReviewSeverity::Low),
        parse_status: runtime::ReviewParseStatus::Structured,
        json_path: ".sego/reviews/review-a.json".to_string(),
        markdown_path: ".sego/reviews/review-a.md".to_string(),
    };
    let review_b = ReviewIndexEntry {
        id: "review-b".to_string(),
        created_at_epoch_seconds: 20,
        scope: "workspace".to_string(),
        diff_hash: "hash-b".to_string(),
        finding_count: 2,
        highest_severity: Some(runtime::ReviewSeverity::Medium),
        parse_status: runtime::ReviewParseStatus::Structured,
        json_path: ".sego/reviews/review-b.json".to_string(),
        markdown_path: ".sego/reviews/review-b.md".to_string(),
    };
    let entries = vec![review_b, review_a];
    assert_eq!(latest_review_index_entry(&entries).unwrap().id, "review-b");
    assert_eq!(resolve_review_entry(&entries, "latest").unwrap().id, "review-b");
}

#[test]
fn review_summary_json_contains_stable_summary_without_full_findings() {
    let entry = ReviewIndexEntry {
        id: "review-123".to_string(),
        created_at_epoch_seconds: 1_782_600_000,
        scope: "staged".to_string(),
        diff_hash: "abc123".to_string(),
        finding_count: 3,
        highest_severity: Some(runtime::ReviewSeverity::Medium),
        parse_status: runtime::ReviewParseStatus::Structured,
        json_path: ".sego/reviews/review-123.json".to_string(),
        markdown_path: ".sego/reviews/review-123.md".to_string(),
    };
    let summary = build_review_summary_json_value(
        "latest",
        &entry,
        ReviewFindingStatusCounts {
            open: 2,
            acknowledged: 1,
            fixed: 0,
            accepted_risk: 0,
            false_positive: 0,
            ignored: 0,
        },
    );
    assert_eq!(summary["schema_version"], 1);
    assert_eq!(summary["kind"], "sego_latest_review_summary");
    assert_eq!(summary["found"], true);
    assert_eq!(summary["review"]["id"], "review-123");
    assert_eq!(summary["review"]["finding_count"], 3);
    assert_eq!(summary["review"]["highest_severity"], "medium");
    assert_eq!(summary["review"]["parse_status"], "structured");
    assert_eq!(summary["review"]["json_path"], ".sego/reviews/review-123.json");
    assert_eq!(summary["review"]["markdown_path"], ".sego/reviews/review-123.md");
    assert_eq!(summary["status_counts"]["open"], 2);
    assert_eq!(summary["status_counts"]["accepted_risk"], 0);
    assert_eq!(summary["status_counts"]["false_positive"], 0);
    assert!(summary.get("raw_text").is_none());
    assert!(summary.get("findings").is_none());
}

#[test]
fn review_summary_json_counts_explicit_terminal_dispositions() {
    let entry = ReviewIndexEntry {
        id: "review-456".to_string(),
        created_at_epoch_seconds: 1_782_600_001,
        scope: "workspace".to_string(),
        diff_hash: "def456".to_string(),
        finding_count: 4,
        highest_severity: Some(runtime::ReviewSeverity::High),
        parse_status: runtime::ReviewParseStatus::Structured,
        json_path: ".sego/reviews/review-456.json".to_string(),
        markdown_path: ".sego/reviews/review-456.md".to_string(),
    };
    let summary = build_review_summary_json_value(
        "review-456",
        &entry,
        ReviewFindingStatusCounts {
            open: 1,
            acknowledged: 0,
            fixed: 1,
            accepted_risk: 1,
            false_positive: 1,
            ignored: 0,
        },
    );

    assert_eq!(summary["status_counts"]["open"], 1);
    assert_eq!(summary["status_counts"]["fixed"], 1);
    assert_eq!(summary["status_counts"]["accepted_risk"], 1);
    assert_eq!(summary["status_counts"]["false_positive"], 1);
}

#[test]
fn review_status_counts_render_explicit_terminal_dispositions() {
    let counts = ReviewFindingStatusCounts {
        open: 1,
        acknowledged: 1,
        fixed: 1,
        accepted_risk: 1,
        false_positive: 1,
        ignored: 1,
    };

    let rendered = counts.render();
    assert!(rendered.contains("accepted_risk 1"));
    assert!(rendered.contains("false_positive 1"));
}

#[test]
fn review_card_commands_parse_default_latest_and_explicit_id() {
    assert_eq!(
        parse_args(&["review".to_string(), "card".to_string()]).expect("review card should parse"),
        CliAction::CodeReviewCard { id: "latest".to_string() }
    );
    assert_eq!(
        parse_args(&["review".to_string(), "card".to_string(), "review-123".to_string()])
            .expect("review card explicit id should parse"),
        CliAction::CodeReviewCard { id: "review-123".to_string() }
    );
    let error = parse_args(&[
        "review".to_string(),
        "card".to_string(),
        "review-123".to_string(),
        "extra".to_string(),
    ])
    .expect_err("review card should reject extra arguments");
    assert!(error.contains("unexpected arguments for /review card"));
}

#[test]
fn review_card_latest_selects_the_newest_artifact() {
    let root = temp_dir();
    let reviews = root.join(".sego").join("reviews");
    fs::create_dir_all(&reviews).expect("review output directory");
    for (id, created_at) in [("review-old", 10_u64), ("review-new", 20_u64)] {
        let artifact = json!({
            "id": id,
            "findings": [],
            "raw_text": "No findings.",
            "parse_status": "structured"
        });
        fs::write(
            reviews.join(format!("{id}.json")),
            serde_json::to_string(&artifact).expect("serialize artifact"),
        )
        .expect("write artifact");
        fs::write(reviews.join(format!("{id}.md")), format!("# {id}\n")).expect("write markdown");
        let entry = ReviewIndexEntry {
            id: id.to_string(),
            created_at_epoch_seconds: created_at,
            scope: "staged".to_string(),
            diff_hash: format!("hash-{id}"),
            finding_count: 0,
            highest_severity: None,
            parse_status: runtime::ReviewParseStatus::Structured,
            json_path: format!(".sego/reviews/{id}.json"),
            markdown_path: format!(".sego/reviews/{id}.md"),
        };
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(reviews.join("index.jsonl"))
            .expect("open index")
            .write_all(
                format!("{}\n", serde_json::to_string(&entry).expect("serialize index")).as_bytes(),
            )
            .expect("append index");
    }

    let generated = generate_review_card_for(&root, "latest").expect("generate latest card");
    assert_eq!(generated.review_id, "review-new");
    assert!(generated.card_path.is_file());
    assert!(generated.latest_card_path.is_file());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn review_card_normalizes_extended_paths_from_review_index() {
    let root = temp_dir();
    let reviews = root.join(".sego").join("reviews");
    fs::create_dir_all(&reviews).expect("review dir");
    let id = "review-extended";
    let artifact = json!({
        "schema_version": 1,
        "id": id,
        "findings": [],
        "raw_text": "No findings.",
        "parse_status": "structured"
    });
    let json_path = reviews.join(format!("{id}.json"));
    let markdown_path = reviews.join(format!("{id}.md"));
    fs::write(&json_path, serde_json::to_string(&artifact).expect("serialize artifact"))
        .expect("write artifact");
    fs::write(&markdown_path, format!("# {id}\n")).expect("write markdown");
    let extended_root = format!("//?/{}", root.to_string_lossy().replace('\\', "/"));
    let entry = ReviewIndexEntry {
        id: id.to_string(),
        created_at_epoch_seconds: 1_783_877_710,
        scope: "full_repo:.".to_string(),
        diff_hash: "hash-extended".to_string(),
        finding_count: 0,
        highest_severity: None,
        parse_status: runtime::ReviewParseStatus::Structured,
        json_path: format!("{extended_root}/.sego/reviews/{id}.json"),
        markdown_path: format!("{extended_root}/.sego/reviews/{id}.md"),
    };
    fs::write(
        reviews.join("index.jsonl"),
        format!("{}\n", serde_json::to_string(&entry).expect("serialize index")),
    )
    .expect("write index");

    let generated = generate_review_card_for(&root, id).expect("generate card");
    let html = fs::read_to_string(generated.card_path).expect("read generated card");
    assert!(html.contains(&format!("href=\"{}\"", crate::local_file_url(&json_path))));
    assert!(html.contains(&format!("href=\"{}\"", crate::local_file_url(&markdown_path))));
    assert!(!html.contains("file:////%3F/"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn review_card_requires_an_existing_artifact() {
    let root = temp_dir();
    fs::create_dir_all(&root).expect("temp workspace");
    let error = generate_review_card_for(&root, "latest")
        .expect_err("review card should require an existing artifact");
    assert!(error.to_string().contains("run `sego review` first"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn review_history_commands_reject_invalid_shapes() {
    let missing_id = parse_args(&["/review".to_string(), "show".to_string()])
        .expect_err("/review show should require an id");
    assert!(missing_id.contains("missing review id"));

    let extra_list_arg =
        parse_args(&["/review".to_string(), "list".to_string(), "extra".to_string()])
            .expect_err("/review list should not accept extra args");
    assert!(extra_list_arg.contains("unexpected arguments"));

    let show_json_missing_id =
        parse_args(&["/review".to_string(), "show".to_string(), "--json".to_string()])
            .expect_err("/review show --json should require id/latest");
    assert!(show_json_missing_id.contains("missing review id"));

    let show_json_extra = parse_args(&[
        "/review".to_string(),
        "show".to_string(),
        "latest".to_string(),
        "--json".to_string(),
        "extra".to_string(),
    ])
    .expect_err("/review show latest --json extra should reject extra args");
    assert!(show_json_extra.contains("unexpected arguments"));

    let extra_ready_arg =
        parse_args(&["/review".to_string(), "ready".to_string(), "extra".to_string()])
            .expect_err("/review ready should not accept extra args");
    assert!(extra_ready_arg.contains("unexpected arguments"));

    let invalid_status = parse_args(&[
        "/review".to_string(),
        "mark".to_string(),
        "review-123".to_string(),
        "finding-456".to_string(),
        "done".to_string(),
    ])
    .expect_err("/review mark should reject invalid status");
    assert!(invalid_status.contains("unsupported review finding status"));
}

#[test]
fn parses_bare_review_as_code_review() {
    assert_eq!(
        parse_args(&["review".to_string()]).expect("review should parse"),
        CliAction::CodeReview {
            scope: None,
            model: default_model(),
            allowed_tools: None,
            permission_mode: PermissionMode::ReadOnly,
        }
    );
    assert_eq!(
        parse_args(&["review".to_string(), "staged".to_string()])
            .expect("review staged should parse"),
        CliAction::CodeReview {
            scope: Some("staged".to_string()),
            model: default_model(),
            allowed_tools: None,
            permission_mode: PermissionMode::ReadOnly,
        }
    );
    assert_eq!(
        parse_args(&["review".to_string(), "list".to_string()]).expect("review list should parse"),
        CliAction::CodeReviewList
    );
}

#[test]
fn parses_explicit_workflow_review_commands() {
    assert_eq!(
        parse_args(&["workflow-review".to_string()]).expect("workflow-review should parse"),
        CliAction::Review { last_n: None, output_format: CliOutputFormat::Text }
    );
    assert_eq!(
        parse_args(&["session-review".to_string(), "--last".to_string(), "5".to_string()])
            .expect("session-review --last should parse"),
        CliAction::Review { last_n: Some(5), output_format: CliOutputFormat::Text }
    );
    let legacy_error = parse_args(&["review".to_string(), "--last".to_string(), "5".to_string()])
        .expect_err("legacy review --last should point to workflow-review");
    assert!(legacy_error.contains("sego workflow-review --last N"));
    let invalid_last =
        parse_args(&["workflow-review".to_string(), "--last".to_string(), "0".to_string()])
            .expect_err("workflow-review --last 0 should fail");
    assert!(invalid_last.contains("positive integer"));
}

#[test]
fn direct_slash_commands_surface_shared_validation_errors() {
    let compact_error = parse_args(&["/compact".to_string(), "now".to_string()])
        .expect_err("invalid /compact shape should be rejected");
    assert!(compact_error.contains("Unexpected arguments for /compact."));
    assert!(compact_error.contains("Usage            /compact"));

    let plugins_error =
        parse_args(&["/plugins".to_string(), "list".to_string(), "extra".to_string()])
            .expect_err("invalid /plugins list shape should be rejected");
    assert!(plugins_error.contains("Usage: /plugin list"));
    assert!(plugins_error.contains("Aliases          /plugins, /marketplace"));
}

#[test]
fn formats_unknown_slash_command_with_suggestions() {
    let report = format_unknown_slash_command_message("statsu");
    assert!(report.contains("unknown slash command: /statsu"));
    assert!(report.contains("Did you mean"));
    assert!(report.contains("Use /help"));
}

#[test]
fn r6_direct_slash_combined_returns_print_and_exit_not_error() {
    let action = parse_args(&[
        "/cd".to_string(),
        "E:\\Sego\\source".to_string(),
        "&&".to_string(),
        "/review".to_string(),
        "staged".to_string(),
    ])
    .expect("combined /cd && /review must return Ok, not error");
    match action {
        CliAction::PrintAndExit { message } => {
            assert!(message.starts_with("Task command blocked"));
            assert!(!message.contains("error:"));
            assert!(!message.contains("Run `sego --help`"));
            assert!(!message.contains("Run sego --help"));
            assert!(message.contains("E:\\Sego\\source"));
            assert!(message.contains("/review staged"));
        }
        other => panic!("expected PrintAndExit, got {other:?}"),
    }
}

#[test]
fn parses_resume_flag_with_slash_command() {
    let args = vec!["--resume".to_string(), "session.jsonl".to_string(), "/compact".to_string()];
    assert_eq!(
        parse_args(&args).expect("args should parse"),
        CliAction::ResumeSession {
            session_path: PathBuf::from("session.jsonl"),
            commands: vec!["/compact".to_string()],
        }
    );
}

#[test]
fn parses_resume_flag_without_path_as_latest_session() {
    assert_eq!(
        parse_args(&["--resume".to_string()]).expect("args should parse"),
        CliAction::ResumeSession { session_path: PathBuf::from("latest"), commands: vec![] }
    );
    assert_eq!(
        parse_args(&["--resume".to_string(), "/status".to_string()])
            .expect("resume shortcut should parse"),
        CliAction::ResumeSession {
            session_path: PathBuf::from("latest"),
            commands: vec!["/status".to_string()],
        }
    );
}

#[test]
fn parses_resume_flag_with_multiple_slash_commands() {
    let args = vec![
        "--resume".to_string(),
        "session.jsonl".to_string(),
        "/status".to_string(),
        "/compact".to_string(),
        "/cost".to_string(),
    ];
    assert_eq!(
        parse_args(&args).expect("args should parse"),
        CliAction::ResumeSession {
            session_path: PathBuf::from("session.jsonl"),
            commands: vec!["/status".to_string(), "/compact".to_string(), "/cost".to_string(),],
        }
    );
}

#[test]
fn rejects_unknown_options_with_helpful_guidance() {
    let error = parse_args(&["--resum".to_string()]).expect_err("unknown option should fail");
    assert!(error.contains("unknown option: --resum"));
    assert!(error.contains("Did you mean --resume?"));
    assert!(error.contains("sego --help"));
}

#[test]
fn parses_resume_flag_with_slash_command_arguments() {
    let args = vec![
        "--resume".to_string(),
        "session.jsonl".to_string(),
        "/export".to_string(),
        "notes.txt".to_string(),
        "/clear".to_string(),
        "--confirm".to_string(),
    ];
    assert_eq!(
        parse_args(&args).expect("args should parse"),
        CliAction::ResumeSession {
            session_path: PathBuf::from("session.jsonl"),
            commands: vec!["/export notes.txt".to_string(), "/clear --confirm".to_string(),],
        }
    );
}

#[test]
fn parses_resume_flag_with_absolute_export_path() {
    let args = vec![
        "--resume".to_string(),
        "session.jsonl".to_string(),
        "/export".to_string(),
        "/tmp/notes.txt".to_string(),
        "/status".to_string(),
    ];
    assert_eq!(
        parse_args(&args).expect("args should parse"),
        CliAction::ResumeSession {
            session_path: PathBuf::from("session.jsonl"),
            commands: vec!["/export /tmp/notes.txt".to_string(), "/status".to_string()],
        }
    );
}

#[test]
fn filtered_tool_specs_respect_allowlist() {
    let allowed = ["read_file", "grep_search"].into_iter().map(str::to_string).collect();
    let filtered = filter_tool_specs(&GlobalToolRegistry::builtin(), Some(&allowed));
    let names = filtered.into_iter().map(|spec| spec.name).collect::<Vec<_>>();
    assert_eq!(names, vec!["read_file", "grep_search"]);
}

#[test]
fn filtered_tool_specs_include_plugin_tools() {
    let filtered = filter_tool_specs(&registry_with_plugin_tool(), None);
    let names = filtered.into_iter().map(|definition| definition.name).collect::<Vec<_>>();
    assert!(names.contains(&"bash".to_string()));
    assert!(names.contains(&"plugin_echo".to_string()));
}

#[test]
fn permission_policy_uses_plugin_tool_permissions() {
    let feature_config = runtime::RuntimeFeatureConfig::default();
    let policy =
        permission_policy(PermissionMode::ReadOnly, &feature_config, &registry_with_plugin_tool())
            .expect("permission policy should build");
    let required = policy.required_mode_for("plugin_echo");
    assert_eq!(required, PermissionMode::WorkspaceWrite);
}

#[test]
fn shared_help_uses_resume_annotation_copy() {
    let help = commands::render_slash_command_help();
    assert!(help.contains("Slash commands"));
    assert!(help.contains("works with --resume SESSION.jsonl"));
}

#[test]
fn repl_help_includes_shared_commands_and_exit() {
    let help = render_repl_help();
    assert!(help.contains("REPL"));
    assert!(help.contains("/help"));
    assert!(help.contains("/dir"));
    assert!(help.contains("Complete commands, modes, and recent sessions"));
    assert!(help.contains("/status"));
    assert!(help.contains("/sandbox"));
    assert!(help.contains("/model [model]"));
    assert!(help.contains("/permissions [read-only|workspace-write|danger-full-access]"));
    assert!(help.contains("/clear [--confirm]"));
    assert!(help.contains("/cost"));
    assert!(help.contains("/resume <session-path>"));
    assert!(help.contains("/config [env|hooks|model|plugins]"));
    assert!(help.contains("/mcp [list|show <server>|help]"));
    assert!(help.contains("/memory"));
    assert!(help.contains("/init"));
    assert!(help.contains("/diff"));
    assert!(help.contains("/version"));
    assert!(help.contains("/export [file]"));
    assert!(help.contains("/session [list|switch <session-id>|fork [branch-name]]"));
    assert!(help.contains(
        "/plugin [list|install <path>|enable <name>|disable <name>|uninstall <id>|update <id>]"
    ));
    assert!(help.contains("aliases: /plugins, /marketplace"));
    assert!(help.contains("/agents"));
    assert!(help.contains("/skills"));
    assert!(help.contains("/exit"));
    assert!(help.contains("Auto-save            .claw/sessions/<session-id>.jsonl"));
    assert!(help.contains("Resume latest        /resume latest"));
}

#[test]
fn completion_candidates_include_workflow_shortcuts_and_dynamic_sessions() {
    let completions = slash_command_completion_candidates_with_sessions(
        "sonnet",
        Some("session-current"),
        vec!["session-old".to_string()],
    );

    assert!(completions.contains(&"/model claude-sonnet-4-6".to_string()));
    assert!(completions.contains(&"/permissions workspace-write".to_string()));
    assert!(completions.contains(&"/session list".to_string()));
    assert!(completions.contains(&"/session switch session-current".to_string()));
    assert!(completions.contains(&"/resume session-old".to_string()));
    assert!(completions.contains(&"/mcp list".to_string()));
    assert!(completions.contains(&"/ultraplan ".to_string()));
}

#[test]
fn startup_banner_mentions_workflow_completions() {
    let _guard = env_lock();
    let env_root = temp_dir();
    let _env = EnvRestore::isolate(&env_root);
    // Inject dummy credentials so LiveCli can construct without real Anthropic key
    let _api_key = EnvVarGuard::set("ANTHROPIC_API_KEY", "test-dummy-key-for-banner-test");
    let _auth_token = EnvVarGuard::set("ANTHROPIC_AUTH_TOKEN", "test-dummy-token-for-banner-test");
    let root = temp_dir();
    fs::create_dir_all(&root).expect("root dir");

    let banner = with_current_dir(&root, || {
        LiveCli::new("claude-sonnet-4-6".to_string(), true, None, PermissionMode::DangerFullAccess)
            .expect("cli should initialize")
            .startup_banner()
    });

    assert!(banner.contains("Tab"));
    assert!(banner.contains("workflow completions"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn resume_supported_command_list_matches_expected_surface() {
    let names =
        resume_supported_slash_commands().into_iter().map(|spec| spec.name).collect::<Vec<_>>();
    // Now with 135+ slash commands, verify minimum resume support
    assert!(
        names.len() >= 39,
        "expected at least 39 resume-supported commands, got {}",
        names.len()
    );
    // Verify key resume commands still exist
    assert!(names.contains(&"help"));
    assert!(names.contains(&"status"));
    assert!(names.contains(&"compact"));
}

#[test]
fn resume_report_uses_sectioned_layout() {
    let report = format_resume_report("session.jsonl", 14, 6);
    assert!(report.contains("Session resumed"));
    assert!(report.contains("Session file     session.jsonl"));
    assert!(report.contains("Messages         14"));
    assert!(report.contains("Turns            6"));
}

#[test]
fn compact_report_uses_structured_output() {
    let compacted = format_compact_report(8, 5, false);
    assert!(compacted.contains("Compact"));
    assert!(compacted.contains("Result           compacted"));
    assert!(compacted.contains("Messages removed 8"));
    let skipped = format_compact_report(0, 3, true);
    assert!(skipped.contains("Result           skipped"));
}

#[test]
fn cost_report_uses_sectioned_layout() {
    let report = format_cost_report(runtime::TokenUsage {
        input_tokens: 20,
        output_tokens: 8,
        cache_creation_input_tokens: 3,
        cache_read_input_tokens: 1,
    });
    assert!(report.contains("Cost"));
    assert!(report.contains("Input tokens     20"));
    assert!(report.contains("Output tokens    8"));
    assert!(report.contains("Cache create     3"));
    assert!(report.contains("Cache read       1"));
    assert!(report.contains("Total tokens     32"));
}

#[test]
fn permissions_report_uses_sectioned_layout() {
    let report = format_permissions_report("workspace-write");
    assert!(report.contains("Permissions"));
    assert!(report.contains("Active mode      workspace-write"));
    assert!(report.contains("Modes"));
    assert!(report.contains("read-only          ○ available Read/search tools only"));
    assert!(report.contains("workspace-write    ● current   Edit files inside the workspace"));
    assert!(report.contains("danger-full-access ○ available Unrestricted tool access"));
}

#[test]
fn permissions_switch_report_is_structured() {
    let report = format_permissions_switch_report("read-only", "workspace-write");
    assert!(report.contains("Permissions updated"));
    assert!(report.contains("Result           mode switched"));
    assert!(report.contains("Previous mode    read-only"));
    assert!(report.contains("Active mode      workspace-write"));
    assert!(report.contains("Applies to       subsequent tool calls"));
}

#[test]
fn init_help_mentions_direct_subcommand() {
    let mut help = Vec::new();
    print_help_to(&mut help).expect("help should render");
    let help = String::from_utf8(help).expect("help should be utf8");
    assert!(help.contains("sego help"));
    assert!(help.contains("sego version"));
    assert!(help.contains("sego status"));
    assert!(help.contains("sego sandbox"));
    assert!(help.contains("sego init"));
    assert!(help.contains("sego agents"));
    assert!(help.contains("sego mcp"));
    assert!(help.contains("sego skills"));
    assert!(help.contains("sego /skills"));
}

#[test]
fn model_report_uses_sectioned_layout() {
    let report = format_model_report("claude-sonnet", 12, 4);
    assert!(report.contains("Model"));
    assert!(report.contains("Current model    claude-sonnet"));
    assert!(report.contains("Session messages 12"));
    assert!(report.contains("Switch models with /model <name>"));
}

#[test]
fn model_switch_report_preserves_context_summary() {
    let report = format_model_switch_report("claude-sonnet", "claude-opus", 9);
    assert!(report.contains("Model updated"));
    assert!(report.contains("Previous         claude-sonnet"));
    assert!(report.contains("Current          claude-opus"));
    assert!(report.contains("Preserved msgs   9"));
}

#[test]
fn status_line_reports_model_and_token_totals() {
    let status = format_status_report(
        "claude-sonnet",
        StatusUsage {
            message_count: 7,
            turns: 3,
            latest: runtime::TokenUsage {
                input_tokens: 5,
                output_tokens: 4,
                cache_creation_input_tokens: 1,
                cache_read_input_tokens: 0,
            },
            cumulative: runtime::TokenUsage {
                input_tokens: 20,
                output_tokens: 8,
                cache_creation_input_tokens: 2,
                cache_read_input_tokens: 1,
            },
            estimated_tokens: 128,
        },
        "workspace-write",
        &super::StatusContext {
            cwd: PathBuf::from("/tmp/project"),
            session_path: Some(PathBuf::from("session.jsonl")),
            loaded_config_files: 2,
            discovered_config_files: 3,
            memory_file_count: 4,
            project_root: Some(PathBuf::from("/tmp")),
            git_branch: Some("main".to_string()),
            git_summary: GitWorkspaceSummary {
                changed_files: 3,
                staged_files: 1,
                unstaged_files: 1,
                untracked_files: 1,
                conflicted_files: 0,
            },
            sandbox_status: runtime::SandboxStatus::default(),
        },
    );
    assert!(status.contains("Status"));
    assert!(status.contains("Model            claude-sonnet"));
    assert!(status.contains("Permission mode  workspace-write"));
    assert!(status.contains("Messages         7"));
    assert!(status.contains("Latest total     10"));
    assert!(status.contains("Cumulative total 31"));
    assert!(status.contains("Provider/cache"));
    assert!(status.contains("Provider         anthropic"));
    assert!(status.contains("Latest cache     create 1, read 0"));
    assert!(status.contains("Cumulative cache create 2, read 1"));
    assert!(status.contains("Cwd              /tmp/project"));
    assert!(status.contains("Project root     /tmp"));
    assert!(status.contains("Git branch       main"));
    assert!(status.contains("Git state        dirty · 3 files · 1 staged, 1 unstaged, 1 untracked"));
    assert!(status.contains("Changed files    3"));
    assert!(status.contains("Staged           1"));
    assert!(status.contains("Unstaged         1"));
    assert!(status.contains("Untracked        1"));
    assert!(status.contains("Session          session.jsonl"));
    assert!(status.contains("Config files     loaded 2/3"));
    assert!(status.contains("Memory files     4"));
    assert!(status.contains("Suggested flow   /status → /diff → /commit"));
}

#[test]
fn commit_reports_surface_workspace_context() {
    let summary = GitWorkspaceSummary {
        changed_files: 2,
        staged_files: 1,
        unstaged_files: 1,
        untracked_files: 0,
        conflicted_files: 0,
    };

    let preflight = format_commit_preflight_report(Some("feature/ux"), summary);
    assert!(preflight.contains("Result           ready"));
    assert!(preflight.contains("Branch           feature/ux"));
    assert!(preflight.contains("Workspace        dirty · 2 files · 1 staged, 1 unstaged"));
    assert!(preflight
        .contains("Action           create a git commit from the current workspace changes"));
}

#[test]
fn commit_skipped_report_points_to_next_steps() {
    let report = format_commit_skipped_report();
    assert!(report.contains("Reason           no workspace changes"));
    assert!(
        report.contains("Action           create a git commit from the current workspace changes")
    );
    assert!(report.contains("/status to inspect context"));
    assert!(report.contains("/diff to inspect repo changes"));
}

#[test]
fn runtime_slash_reports_describe_command_behavior() {
    let bughunter = format_bughunter_report(Some("runtime"));
    assert!(bughunter.contains("Scope            runtime"));
    assert!(bughunter.contains("inspect the selected code for likely bugs"));

    let ultraplan = format_ultraplan_report(Some("ship the release"));
    assert!(ultraplan.contains("Task             ship the release"));
    assert!(ultraplan.contains("break work into a multi-step execution plan"));

    let pr = format_pr_report("feature/ux", Some("ready for review"));
    assert!(pr.contains("Branch           feature/ux"));
    assert!(pr.contains("draft or create a pull request"));

    let issue = format_issue_report(Some("flaky test"));
    assert!(issue.contains("Context          flaky test"));
    assert!(issue.contains("draft or create a GitHub issue"));
}

#[test]
fn no_arg_commands_reject_unexpected_arguments() {
    assert!(validate_no_args("/commit", None).is_ok());

    let error = validate_no_args("/commit", Some("now"))
        .expect_err("unexpected arguments should fail")
        .to_string();
    assert!(error.contains("/commit does not accept arguments"));
    assert!(error.contains("Received: now"));
}

#[test]
fn config_report_supports_section_views() {
    let report = render_config_report(Some("env")).expect("config report should render");
    assert!(report.contains("Merged section: env"));
    let plugins_report =
        render_config_report(Some("plugins")).expect("plugins config report should render");
    assert!(plugins_report.contains("Merged section: plugins"));
}

#[test]
fn memory_report_uses_sectioned_layout() {
    let report = render_memory_report().expect("memory report should render");
    assert!(report.contains("Memory"));
    assert!(report.contains("Working directory"));
    assert!(report.contains("Instruction files"));
    assert!(report.contains("Discovered files"));
}

#[test]
fn config_report_uses_sectioned_layout() {
    let report = render_config_report(None).expect("config report should render");
    assert!(report.contains("Config"));
    assert!(report.contains("Discovered files"));
    assert!(report.contains("Merged JSON"));
}

#[test]
fn parses_git_status_metadata() {
    let _guard = env_lock();
    let temp_root = temp_dir();
    fs::create_dir_all(&temp_root).expect("root dir");
    let (project_root, branch) = parse_git_status_metadata_for(
        &temp_root,
        Some(
            "## rcc/cli...origin/rcc/cli
 M src/main.rs",
        ),
    );
    assert_eq!(branch.as_deref(), Some("rcc/cli"));
    assert!(project_root.is_none());
    let _ = fs::remove_dir_all(temp_root);
}

#[test]
fn parses_detached_head_from_status_snapshot() {
    let _guard = env_lock();
    assert_eq!(
        parse_git_status_branch(Some(
            "## HEAD (no branch)
 M src/main.rs"
        )),
        Some("detached HEAD".to_string())
    );
}

#[test]
fn parses_git_workspace_summary_counts() {
    let summary = parse_git_workspace_summary(Some(
        "## feature/ux
M  src/main.rs
 M README.md
?? notes.md
UU conflicted.rs",
    ));

    assert_eq!(
        summary,
        GitWorkspaceSummary {
            changed_files: 4,
            staged_files: 2,
            unstaged_files: 2,
            untracked_files: 1,
            conflicted_files: 1,
        }
    );
    assert_eq!(
        summary.headline(),
        "dirty · 4 files · 2 staged, 2 unstaged, 1 untracked, 1 conflicted"
    );
}

#[test]
fn render_diff_report_shows_clean_tree_for_committed_repo() {
    let _guard = env_lock();
    let root = temp_dir();
    fs::create_dir_all(&root).expect("root dir");
    git(&["init", "--quiet"], &root);
    git(&["config", "user.email", "tests@example.com"], &root);
    git(&["config", "user.name", "Rusty Claude Tests"], &root);
    fs::write(root.join("tracked.txt"), "hello\n").expect("write file");
    git(&["add", "tracked.txt"], &root);
    git(&["commit", "-m", "init", "--quiet"], &root);

    let report = render_diff_report_for(&root).expect("diff report should render");
    assert!(report.contains("clean working tree"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn render_diff_report_includes_staged_and_unstaged_sections() {
    let _guard = env_lock();
    let root = temp_dir();
    fs::create_dir_all(&root).expect("root dir");
    git(&["init", "--quiet"], &root);
    git(&["config", "user.email", "tests@example.com"], &root);
    git(&["config", "user.name", "Rusty Claude Tests"], &root);
    fs::write(root.join("tracked.txt"), "hello\n").expect("write file");
    git(&["add", "tracked.txt"], &root);
    git(&["commit", "-m", "init", "--quiet"], &root);

    fs::write(root.join("tracked.txt"), "hello\nstaged\n").expect("update file");
    git(&["add", "tracked.txt"], &root);
    fs::write(root.join("tracked.txt"), "hello\nstaged\nunstaged\n").expect("update file twice");

    let report = render_diff_report_for(&root).expect("diff report should render");
    assert!(report.contains("Staged changes:"));
    assert!(report.contains("Unstaged changes:"));
    assert!(report.contains("tracked.txt"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn review_readiness_blocks_when_no_files_are_staged() {
    let _guard = env_lock();
    let root = temp_dir();
    fs::create_dir_all(&root).expect("root dir");
    git(&["init", "--quiet"], &root);

    let report = render_code_review_readiness_for(&root).expect("readiness report");

    assert!(report.contains("Review Readiness"));
    assert!(report.contains("Result           blocked"));
    assert!(report.contains("Staged files     0"));
    assert!(report.contains("stage changes before running /review ready"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn non_git_directory_review_shows_friendly_error() {
    let _guard = env_lock();
    let root = temp_dir();
    fs::create_dir_all(&root).expect("root dir");
    assert!(!is_git_worktree(&root));
    let error = non_git_review_error(&root);
    // C20.5-B: recovery hint format.
    let lines = error.lines().collect::<Vec<_>>();
    assert_eq!(lines.first(), Some(&"Review"));
    assert!(lines.get(1).is_some_and(|line| line.contains("Result") && line.contains("failed")));
    assert!(error.contains("no Git repository found"));
    assert!(error.contains("Workspace"));
    assert!(error.contains("review --full"));
    assert!(error.contains("/dir"));
    assert!(error.contains(&root.display().to_string()));
    assert!(!error.contains("fatal:"));
    git(&["init", "--quiet"], &root);
    assert!(is_git_worktree(&root));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn review_completion_summary_includes_expected_sections() {
    let report = runtime::ReviewReport {
        findings: vec![runtime::ReviewFinding {
            id: "f1".into(),
            severity: runtime::ReviewSeverity::Low,
            file: "src/main.rs".into(),
            line: Some(1966),
            title: "REPL non-Git warning".into(),
            evidence: "warning appears before the prompt".into(),
            risk: "users may miss the active workspace".into(),
            suggestion: "show the active workspace clearly".into(),
            confidence: 0.8,
            verification_hint: Some("run sego from a non-Git folder".into()),
            evidence_status: None,
        }],
        raw_text: String::new(),
        parse_status: runtime::ReviewParseStatus::Structured,
        parse_error: String::new(),
        parse_repair: None,
    };
    let artifact = runtime::PersistedReviewArtifact {
        id: "review-abc123".into(),
        diff_hash: "hash123".into(),
        json_path: std::path::PathBuf::from(".sego/reviews/r.json"),
        markdown_path: std::path::PathBuf::from(".sego/reviews/r.md"),
        index_path: std::path::PathBuf::from(".sego/reviews/index.jsonl"),
    };
    let summary = format_review_completion_summary(&report, &artifact);
    assert!(summary.contains("Review Report"));
    assert!(summary.contains("Diff hash        hash123"));
    assert!(summary.contains("Parse status     structured"));
    assert!(summary.contains("Findings         1"));
    assert!(summary.contains("Highest severity low"));
    assert!(summary.contains("Findings"));
    assert!(summary.contains("1. [low] src/main.rs:1966"));
    assert!(summary.contains("Title          REPL non-Git warning"));
    assert!(summary.contains("Evidence       warning appears before the prompt"));
    assert!(summary.contains("Risk           users may miss the active workspace"));
    assert!(summary.contains("Suggestion     show the active workspace clearly"));
    assert!(summary.contains("Verify         run sego from a non-Git folder"));
    assert!(summary.contains("Reports"));
    assert!(summary.contains("Markdown"));
    assert!(summary.contains("JSON"));
    assert!(!summary.contains("\"findings\":["));
    assert!(summary.contains("Next step"));
}

#[test]
fn review_completion_summary_shows_at_most_ten_terminal_findings() {
    let findings: Vec<_> = (1..=11)
        .map(|i| runtime::ReviewFinding {
            id: format!("f{i}"),
            severity: runtime::ReviewSeverity::Low,
            file: format!("src/file{i}.rs"),
            line: Some(i as u32 * 10),
            title: format!("Finding number {i}"),
            evidence: "...".into(),
            risk: "low".into(),
            suggestion: "...".into(),
            confidence: 0.5,
            verification_hint: None,
            evidence_status: None,
        })
        .collect();
    let report = runtime::ReviewReport {
        findings,
        raw_text: String::new(),
        parse_status: runtime::ReviewParseStatus::Structured,
        parse_error: String::new(),
        parse_repair: None,
    };
    let artifact = runtime::PersistedReviewArtifact {
        id: "review-limit".into(),
        diff_hash: "hash".into(),
        json_path: std::path::PathBuf::from(".sego/reviews/r.json"),
        markdown_path: std::path::PathBuf::from(".sego/reviews/r.md"),
        index_path: std::path::PathBuf::from(".sego/reviews/index.jsonl"),
    };
    let summary = format_review_completion_summary(&report, &artifact);
    assert!(summary.contains("Finding number 1"));
    assert!(summary.contains("Finding number 10"));
    assert!(!summary.contains("Finding number 11"));
    assert!(summary.contains("... 1 more finding(s)"));
}

#[test]
fn review_completion_summary_shows_no_findings() {
    let report = runtime::ReviewReport {
        findings: vec![],
        raw_text: String::new(),
        parse_status: runtime::ReviewParseStatus::FallbackRawText,
        parse_error: String::new(),
        parse_repair: None,
    };
    let artifact = runtime::PersistedReviewArtifact {
        id: "review-empty".into(),
        diff_hash: "hash0".into(),
        json_path: std::path::PathBuf::from(".sego/reviews/r.json"),
        markdown_path: std::path::PathBuf::from(".sego/reviews/r.md"),
        index_path: std::path::PathBuf::from(".sego/reviews/index.jsonl"),
    };
    let summary = format_review_completion_summary(&report, &artifact);
    assert!(summary.contains("No structured findings."));
    assert!(summary.contains("Structured parsing failed"));
}

#[test]
fn review_completion_summary_marks_parse_attempted_as_unknown() {
    // C20.5-A R3: ParseAttemptedButFailed must show "unknown (parse failed)",
    // not misleading "Findings 0" / "No structured findings."
    let report = runtime::ReviewReport {
        findings: vec![],
        raw_text: String::new(),
        parse_status: runtime::ReviewParseStatus::ParseAttemptedButFailed,
        parse_error: String::new(),
        parse_repair: None,
    };
    let artifact = runtime::PersistedReviewArtifact {
        id: "review-parse-failed".into(),
        diff_hash: "hash1".into(),
        json_path: std::path::PathBuf::from(".sego/reviews/r.json"),
        markdown_path: std::path::PathBuf::from(".sego/reviews/r.md"),
        index_path: std::path::PathBuf::from(".sego/reviews/index.jsonl"),
    };
    let summary = format_review_completion_summary(&report, &artifact);

    assert!(summary.contains("parse_attempted_but_failed"));
    assert!(summary.contains("unknown (parse failed)"));
    assert!(summary.contains("Structured findings could not be parsed"));
    assert!(summary.contains("Open the Markdown report"));
    assert!(!summary.contains("Findings         0"));
    assert!(!summary.contains("No structured findings."));
}

#[test]
fn display_path_for_user_strips_windows_long_path_prefixes() {
    if cfg!(windows) {
        assert_eq!(
            display_path_for_user(std::path::Path::new(r"\\?\E:\Sego\source\.sego\r.md")),
            r"E:\Sego\source\.sego\r.md"
        );
        assert_eq!(
            display_path_for_user(std::path::Path::new(r"\\?\UNC\server\share\r.md")),
            r"\\server\share\r.md"
        );
    } else {
        assert_eq!(display_path_for_user(std::path::Path::new("/tmp/r.md")), "/tmp/r.md");
    }
}

#[test]
fn review_readiness_plans_manual_review_and_fast_verify_for_staged_files() {
    let _guard = env_lock();
    let root = temp_dir();
    fs::create_dir_all(&root).expect("root dir");
    git(&["init", "--quiet"], &root);
    fs::write(root.join("Cargo.toml"), "[package]\nname='demo'\n").expect("write manifest");
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::write(root.join("src").join("main.rs"), "fn main() {}\n").expect("write main");
    git(&["add", "Cargo.toml", "src/main.rs"], &root);

    let report = render_code_review_readiness_for(&root).expect("readiness report");

    assert!(report.contains("Result           ready with manual gates"));
    assert!(report.contains("Staged files     2"));
    assert!(report.contains("Safety lock      passed"));
    assert!(report.contains("Verify fast      planned 1 command(s)"));
    assert!(report.contains("cargo build (.)"));
    assert!(report.contains("Command        sego /review staged"));
    assert!(report.contains("Command        sego /verify fast"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn review_summary_renders_empty_git_repo_without_side_effects() {
    let _guard = env_lock();
    let root = temp_dir();
    fs::create_dir_all(&root).expect("root dir");
    git(&["init", "--quiet"], &root);

    let report = render_code_review_summary_for(&root).expect("summary report");

    assert!(report.contains("Review Summary"));
    assert!(report.contains("Mode             read-only"));
    assert!(report.contains("Staged safety"));
    assert!(report.contains("Result         no staged files"));
    assert!(report.contains("Latest review"));
    assert!(report.contains("Verify fast"));
    assert!(report.contains("Suggested next steps"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn review_summary_includes_staged_safety_and_fast_verify_plan() {
    let _guard = env_lock();
    let root = temp_dir();
    fs::create_dir_all(&root).expect("root dir");
    git(&["init", "--quiet"], &root);
    fs::write(root.join("Cargo.toml"), "[package]\nname='demo'\n").expect("write manifest");
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::write(root.join("src").join("main.rs"), "fn main() {}\n").expect("write main");
    git(&["add", "Cargo.toml", "src/main.rs"], &root);

    let report = render_code_review_summary_for(&root).expect("summary report");

    assert!(report.contains("Review Summary"));
    assert!(report.contains("Staged files   2"));
    assert!(report.contains("Staged diff    yes"));
    assert!(report.contains("Result         passed"));
    assert!(report.contains("Plan           1 command(s)"));
    assert!(report.contains("cargo build (.)"));
    assert!(report.contains("sego /review staged"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn render_diff_report_omits_ignored_files() {
    let _guard = env_lock();
    let root = temp_dir();
    fs::create_dir_all(&root).expect("root dir");
    git(&["init", "--quiet"], &root);
    git(&["config", "user.email", "tests@example.com"], &root);
    git(&["config", "user.name", "Rusty Claude Tests"], &root);
    fs::write(root.join(".gitignore"), ".omx/\nignored.txt\n").expect("write gitignore");
    fs::write(root.join("tracked.txt"), "hello\n").expect("write tracked");
    git(&["add", ".gitignore", "tracked.txt"], &root);
    git(&["commit", "-m", "init", "--quiet"], &root);
    fs::create_dir_all(root.join(".omx")).expect("write omx dir");
    fs::write(root.join(".omx").join("state.json"), "{}").expect("write ignored omx");
    fs::write(root.join("ignored.txt"), "secret\n").expect("write ignored file");
    fs::write(root.join("tracked.txt"), "hello\nworld\n").expect("write tracked change");

    let report = render_diff_report_for(&root).expect("diff report should render");
    assert!(report.contains("tracked.txt"));
    assert!(!report.contains("+++ b/ignored.txt"));
    assert!(!report.contains("+++ b/.omx/state.json"));

    // Same reason as the test below: a Windows handle race on a temporary
    // directory must not be reported as a product failure.
    let _ = fs::remove_dir_all(root);
}

#[test]
fn resume_diff_command_renders_report_for_saved_session() {
    let _guard = env_lock();
    let root = temp_dir();
    fs::create_dir_all(&root).expect("root dir");
    git(&["init", "--quiet"], &root);
    git(&["config", "user.email", "tests@example.com"], &root);
    git(&["config", "user.name", "Rusty Claude Tests"], &root);
    fs::write(root.join("tracked.txt"), "hello\n").expect("write tracked");
    git(&["add", "tracked.txt"], &root);
    git(&["commit", "-m", "init", "--quiet"], &root);
    fs::write(root.join("tracked.txt"), "hello\nworld\n").expect("modify tracked");
    let session_path = root.join("session.json");
    Session::new().save_to_path(&session_path).expect("session should save");

    let session = Session::load_from_path(&session_path).expect("session should load");
    let outcome = with_current_dir(&root, || {
        run_resume_command(&session_path, &session, &SlashCommand::Diff)
            .expect("resume diff should work")
    });
    let message = outcome.message.expect("diff message should exist");
    assert!(message.contains("Unstaged changes:"));
    assert!(message.contains("tracked.txt"));

    // Cleanup must tolerate failure. This test leaves no handle of its own,
    // but it runs `git` in `root`, and on Windows a just-exited child or a
    // scanner can still hold the directory, which turns a passing test into
    // `ERROR_SHARING_VIOLATION` under parallel load. The assertions above
    // are the test; deleting the directory is not.
    let _ = fs::remove_dir_all(root);
}

#[test]
fn status_context_reads_real_workspace_metadata() {
    let context = status_context(None).expect("status context should load");
    assert!(context.cwd.is_absolute());
    assert!(context.discovered_config_files >= context.loaded_config_files);
    assert!(context.loaded_config_files <= context.discovered_config_files);
}

#[test]
fn normalizes_supported_permission_modes() {
    assert_eq!(normalize_permission_mode("read-only"), Some("read-only"));
    assert_eq!(normalize_permission_mode("workspace-write"), Some("workspace-write"));
    assert_eq!(normalize_permission_mode("danger-full-access"), Some("danger-full-access"));
    assert_eq!(normalize_permission_mode("unknown"), None);
}

#[test]
fn clear_command_requires_explicit_confirmation_flag() {
    assert_eq!(SlashCommand::parse("/clear"), Ok(Some(SlashCommand::Clear { confirm: false })));
    assert_eq!(
        SlashCommand::parse("/clear --confirm"),
        Ok(Some(SlashCommand::Clear { confirm: true }))
    );
}

#[test]
fn parses_resume_and_config_slash_commands() {
    assert_eq!(
        SlashCommand::parse("/resume saved-session.jsonl"),
        Ok(Some(SlashCommand::Resume { session_path: Some("saved-session.jsonl".to_string()) }))
    );
    assert_eq!(
        SlashCommand::parse("/clear --confirm"),
        Ok(Some(SlashCommand::Clear { confirm: true }))
    );
    assert_eq!(SlashCommand::parse("/config"), Ok(Some(SlashCommand::Config { section: None })));
    assert_eq!(
        SlashCommand::parse("/config env"),
        Ok(Some(SlashCommand::Config { section: Some("env".to_string()) }))
    );
    assert_eq!(SlashCommand::parse("/memory"), Ok(Some(SlashCommand::Memory)));
    assert_eq!(SlashCommand::parse("/init"), Ok(Some(SlashCommand::Init)));
    assert_eq!(
        SlashCommand::parse("/session fork incident-review"),
        Ok(Some(SlashCommand::Session {
            action: Some("fork".to_string()),
            target: Some("incident-review".to_string())
        }))
    );
}

#[test]
fn help_mentions_jsonl_resume_examples() {
    let mut help = Vec::new();
    print_help_to(&mut help).expect("help should render");
    let help = String::from_utf8(help).expect("help should be utf8");
    assert!(help.contains("sego --resume [SESSION.jsonl|session-id|latest]"));
    assert!(help.contains("Use `latest` with --resume, /resume, or /session switch"));
    assert!(help.contains("sego --resume latest"));
    assert!(help.contains("sego --resume latest /status /diff /export notes.txt"));
}

#[test]
fn managed_sessions_default_to_jsonl_and_resolve_legacy_json() {
    let _guard = cwd_lock().lock().expect("cwd lock");
    let workspace = temp_workspace("session-resolution");
    std::fs::create_dir_all(&workspace).expect("workspace should create");
    let previous = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(&workspace).expect("switch cwd");

    let handle = create_managed_session_handle("session-alpha").expect("jsonl handle");
    assert!(handle.path.ends_with("session-alpha.jsonl"));

    let legacy_path = workspace.join(".claw/sessions/legacy.json");
    std::fs::create_dir_all(
        legacy_path.parent().expect("legacy path should have parent directory"),
    )
    .expect("session dir should exist");
    Session::new()
        .with_persistence_path(legacy_path.clone())
        .save_to_path(&legacy_path)
        .expect("legacy session should save");

    let resolved = resolve_session_reference("legacy").expect("legacy session should resolve");
    assert_eq!(
        resolved.path.canonicalize().expect("resolved path should exist"),
        legacy_path.canonicalize().expect("legacy path should exist")
    );

    std::env::set_current_dir(previous).expect("restore cwd");
    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn latest_session_alias_resolves_most_recent_managed_session() {
    let _guard = cwd_lock().lock().expect("cwd lock");
    let workspace = temp_workspace("latest-session-alias");
    std::fs::create_dir_all(&workspace).expect("workspace should create");
    let previous = std::env::current_dir().expect("cwd");
    std::env::set_current_dir(&workspace).expect("switch cwd");

    let older = create_managed_session_handle("session-older").expect("older handle");
    Session::new()
        .with_persistence_path(older.path.clone())
        .save_to_path(&older.path)
        .expect("older session should save");
    std::thread::sleep(Duration::from_millis(20));
    let newer = create_managed_session_handle("session-newer").expect("newer handle");
    Session::new()
        .with_persistence_path(newer.path.clone())
        .save_to_path(&newer.path)
        .expect("newer session should save");

    let resolved = resolve_session_reference("latest").expect("latest session should resolve");
    assert_eq!(
        resolved.path.canonicalize().expect("resolved path should exist"),
        newer.path.canonicalize().expect("newer path should exist")
    );

    std::env::set_current_dir(previous).expect("restore cwd");
    let _ = std::fs::remove_dir_all(workspace);
}

#[test]
fn unknown_slash_command_guidance_suggests_nearby_commands() {
    let message = format_unknown_slash_command("stats");
    assert!(message.contains("Unknown slash command: /stats"));
    assert!(message.contains("/status"));
    assert!(message.contains("/help"));
}

#[test]
fn resume_usage_mentions_latest_shortcut() {
    let usage = render_resume_usage();
    assert!(usage.contains("/resume <session-path|session-id|latest>"));
    assert!(usage.contains(".claw/sessions/<session-id>.jsonl"));
    assert!(usage.contains("/session list"));
}

fn cwd_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn temp_workspace(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("claw-cli-{label}-{nanos}"))
}

#[test]
fn init_template_mentions_detected_rust_workspace() {
    let _guard = cwd_lock().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rendered = crate::init::render_init_claude_md(&workspace_root);
    assert!(rendered.contains("# CLAUDE.md"));
    assert!(rendered.contains("cargo clippy --workspace --all-targets -- -D warnings"));
}

#[test]
fn converts_tool_roundtrip_messages() {
    let messages = vec![
        ConversationMessage::user_text("hello"),
        ConversationMessage::assistant(vec![ContentBlock::ToolUse {
            id: "tool-1".to_string(),
            name: "bash".to_string(),
            input: "{\"command\":\"pwd\"}".to_string(),
        }]),
        ConversationMessage {
            role: MessageRole::Tool,
            blocks: vec![ContentBlock::ToolResult {
                tool_use_id: "tool-1".to_string(),
                tool_name: "bash".to_string(),
                output: "ok".to_string(),
                is_error: false,
            }],
            usage: None,
        },
    ];

    let converted = super::convert_messages(&messages);
    assert_eq!(converted.len(), 3);
    assert_eq!(converted[1].role, "assistant");
    assert_eq!(converted[2].role, "user");
}
#[test]
fn repl_help_mentions_history_completion_and_multiline() {
    let help = render_repl_help();
    assert!(help.contains("Up/Down"));
    assert!(help.contains("Tab"));
    assert!(help.contains("Shift+Enter/Ctrl+J"));
}

#[test]
fn tool_rendering_helpers_compact_output() {
    let start = format_tool_call_start("read_file", r#"{"path":"src/main.rs"}"#);
    assert!(start.contains("read_file"));
    assert!(start.contains("src/main.rs"));

    let done = format_tool_result(
        "read_file",
        r#"{"file":{"filePath":"src/main.rs","content":"hello","numLines":1,"startLine":1,"totalLines":1}}"#,
        false,
    );
    assert!(done.contains("📄 Read src/main.rs"));
    assert!(done.contains("hello"));
}

#[test]
fn tool_rendering_truncates_large_read_output_for_display_only() {
    let content = (0..200).map(|index| format!("line {index:03}")).collect::<Vec<_>>().join("\n");
    let output = json!({
        "file": {
            "filePath": "src/main.rs",
            "content": content,
            "numLines": 200,
            "startLine": 1,
            "totalLines": 200
        }
    })
    .to_string();

    let rendered = format_tool_result("read_file", &output, false);

    assert!(rendered.contains("line 000"));
    assert!(rendered.contains("line 039"));
    assert!(!rendered.contains("line 040"));
    assert!(!rendered.contains("line 199"));
    assert!(rendered.contains("full result preserved in session"));
    assert!(output.contains("line 199"));
}

#[test]
fn tool_rendering_truncates_large_bash_output_for_display_only() {
    let stdout = (0..120).map(|index| format!("stdout {index:03}")).collect::<Vec<_>>().join("\n");
    let output = json!({
        "stdout": stdout,
        "stderr": "",
        "returnCodeInterpretation": "completed successfully"
    })
    .to_string();

    let rendered = format_tool_result("bash", &output, false);

    assert!(rendered.contains("stdout 000"));
    assert!(!rendered.contains("stdout 014"));
    assert!(!rendered.contains("stdout 119"));
    assert!(output.contains("stdout 119"));
}

#[test]
fn tool_rendering_truncates_generic_long_output_for_display_only() {
    let items = (0..120).map(|index| format!("payload {index:03}")).collect::<Vec<_>>();
    let output = json!({
        "summary": "plugin payload",
        "items": items,
    })
    .to_string();

    let rendered = format_tool_result("plugin_echo", &output, false);

    assert!(rendered.contains("plugin_echo"));
    assert!(rendered.contains("payload 000"));
    assert!(rendered.contains("payload 010"));
    assert!(!rendered.contains("payload 020"));
    assert!(!rendered.contains("payload 080"));
    assert!(!rendered.contains("payload 119"));
    assert!(rendered.contains("full result preserved in session"));
    assert!(output.contains("payload 119"));
}

#[test]
fn tool_rendering_truncates_raw_generic_output_for_display_only() {
    let output = (0..120).map(|index| format!("raw {index:03}")).collect::<Vec<_>>().join("\n");

    let rendered = format_tool_result("plugin_echo", &output, false);

    assert!(rendered.contains("plugin_echo"));
    assert!(rendered.contains("raw 000"));
    assert!(rendered.contains("raw 014"));
    assert!(!rendered.contains("raw 015"));
    assert!(!rendered.contains("raw 119"));
    assert!(rendered.contains("full result preserved in session"));
    assert!(output.contains("raw 119"));
}

#[test]
fn ultraplan_progress_lines_include_phase_step_and_elapsed_status() {
    let snapshot = InternalPromptProgressState {
        command_label: "Ultraplan",
        task_label: "ship plugin progress".to_string(),
        step: 3,
        phase: "running read_file".to_string(),
        detail: Some("reading rust/crates/rusty-claude-cli/src/main.rs".to_string()),
        saw_final_text: false,
    };

    let started = format_internal_prompt_progress_line(
        InternalPromptProgressEvent::Started,
        &snapshot,
        Duration::from_secs(0),
        None,
    );
    let heartbeat = format_internal_prompt_progress_line(
        InternalPromptProgressEvent::Heartbeat,
        &snapshot,
        Duration::from_secs(9),
        None,
    );
    let completed = format_internal_prompt_progress_line(
        InternalPromptProgressEvent::Complete,
        &snapshot,
        Duration::from_secs(12),
        None,
    );
    let failed = format_internal_prompt_progress_line(
        InternalPromptProgressEvent::Failed,
        &snapshot,
        Duration::from_secs(12),
        Some("network timeout"),
    );

    assert!(started.contains("planning started"));
    assert!(started.contains("current step 3"));
    assert!(heartbeat.contains("heartbeat"));
    assert!(heartbeat.contains("9s elapsed"));
    assert!(heartbeat.contains("phase running read_file"));
    assert!(completed.contains("completed"));
    assert!(completed.contains("3 steps total"));
    assert!(failed.contains("failed"));
    assert!(failed.contains("network timeout"));
}

#[test]
fn describe_tool_progress_summarizes_known_tools() {
    assert_eq!(
        describe_tool_progress("read_file", r#"{"path":"src/main.rs"}"#),
        "reading src/main.rs"
    );
    assert!(describe_tool_progress("bash", r#"{"command":"cargo test -p rusty-claude-cli"}"#)
        .contains("cargo test -p rusty-claude-cli"));
    assert_eq!(
        describe_tool_progress("grep_search", r#"{"pattern":"ultraplan","path":"rust"}"#),
        "grep `ultraplan` in rust"
    );
}

#[test]
fn push_output_block_renders_markdown_text() {
    let mut out = Vec::new();
    let mut events = Vec::new();
    let mut pending_tool = None;

    push_output_block(
        OutputContentBlock::Text { text: "# Heading".to_string() },
        &mut out,
        &mut events,
        &mut pending_tool,
        false,
    )
    .expect("text block should render");

    let rendered = String::from_utf8(out).expect("utf8");
    assert!(rendered.contains("Heading"));
    assert!(rendered.contains('\u{1b}'));
}

#[test]
fn push_output_block_skips_empty_object_prefix_for_tool_streams() {
    let mut out = Vec::new();
    let mut events = Vec::new();
    let mut pending_tool = None;

    push_output_block(
        OutputContentBlock::ToolUse {
            id: "tool-1".to_string(),
            name: "read_file".to_string(),
            input: json!({}),
        },
        &mut out,
        &mut events,
        &mut pending_tool,
        true,
    )
    .expect("tool block should accumulate");

    assert!(events.is_empty());
    assert_eq!(pending_tool, Some(("tool-1".to_string(), "read_file".to_string(), String::new(),)));
}

#[test]
fn response_to_events_preserves_empty_object_json_input_outside_streaming() {
    let mut out = Vec::new();
    let events = response_to_events(
        MessageResponse {
            id: "msg-1".to_string(),
            kind: "message".to_string(),
            model: "claude-opus-4-6".to_string(),
            role: "assistant".to_string(),
            content: vec![OutputContentBlock::ToolUse {
                id: "tool-1".to_string(),
                name: "read_file".to_string(),
                input: json!({}),
            }],
            stop_reason: Some("tool_use".to_string()),
            stop_sequence: None,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
            request_id: None,
        },
        &mut out,
    )
    .expect("response conversion should succeed");

    assert!(matches!(
        &events[0],
        AssistantEvent::ToolUse { name, input, .. }
            if name == "read_file" && input == "{}"
    ));
}

#[test]
fn response_to_events_preserves_non_empty_json_input_outside_streaming() {
    let mut out = Vec::new();
    let events = response_to_events(
        MessageResponse {
            id: "msg-2".to_string(),
            kind: "message".to_string(),
            model: "claude-opus-4-6".to_string(),
            role: "assistant".to_string(),
            content: vec![OutputContentBlock::ToolUse {
                id: "tool-2".to_string(),
                name: "read_file".to_string(),
                input: json!({ "path": "rust/Cargo.toml" }),
            }],
            stop_reason: Some("tool_use".to_string()),
            stop_sequence: None,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
            request_id: None,
        },
        &mut out,
    )
    .expect("response conversion should succeed");

    assert!(matches!(
        &events[0],
        AssistantEvent::ToolUse { name, input, .. }
            if name == "read_file" && input == "{\"path\":\"rust/Cargo.toml\"}"
    ));
}

#[test]
fn response_to_events_ignores_thinking_blocks() {
    let mut out = Vec::new();
    let events = response_to_events(
        MessageResponse {
            id: "msg-3".to_string(),
            kind: "message".to_string(),
            model: "claude-opus-4-6".to_string(),
            role: "assistant".to_string(),
            content: vec![
                OutputContentBlock::Thinking {
                    thinking: "step 1".to_string(),
                    signature: Some("sig_123".to_string()),
                },
                OutputContentBlock::Text { text: "Final answer".to_string() },
            ],
            stop_reason: Some("end_turn".to_string()),
            stop_sequence: None,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
            request_id: None,
        },
        &mut out,
    )
    .expect("response conversion should succeed");

    assert!(matches!(
        &events[0],
        AssistantEvent::Thinking { thinking, signature }
            if thinking == "step 1" && signature.as_deref() == Some("sig_123")
    ));
    assert!(matches!(
        &events[1],
        AssistantEvent::TextDelta(text) if text == "Final answer"
    ));
    assert!(!String::from_utf8(out).expect("utf8").contains("step 1"));
}

#[test]
fn build_runtime_plugin_state_merges_plugin_hooks_into_runtime_features() {
    let config_home = temp_dir();
    let workspace = temp_dir();
    let source_root = temp_dir();
    fs::create_dir_all(&config_home).expect("config home");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&source_root).expect("source root");
    write_plugin_fixture(&source_root, "hook-runtime-demo", true, false);

    let mut manager = PluginManager::new(PluginManagerConfig::new(&config_home));
    manager
        .install(source_root.to_str().expect("utf8 source path"))
        .expect("plugin install should succeed");
    let loader = ConfigLoader::new(&workspace, &config_home);
    let runtime_config = loader.load().expect("runtime config should load");
    let state = build_runtime_plugin_state_with_loader(&workspace, &loader, &runtime_config)
        .expect("plugin state should load");
    let pre_hooks = state.feature_config.hooks().pre_tool_use();
    assert_eq!(pre_hooks.len(), 1);
    let (fixture_hook_path, _) = plugin_fixture_hook();
    assert!(
        Path::new(&pre_hooks[0]).ends_with(fixture_hook_path.trim_start_matches("./")),
        "expected installed plugin hook path, got {pre_hooks:?}"
    );

    let _ = fs::remove_dir_all(config_home);
    let _ = fs::remove_dir_all(workspace);
    let _ = fs::remove_dir_all(source_root);
}

#[test]
fn build_runtime_plugin_state_discovers_mcp_tools_and_surfaces_pending_servers() {
    // The MCP fixture is a Python script, so this test needs a working
    // interpreter to exist at all. Without the check, a host that only has
    // a non-running python shim reports "mcp tools should be allow-listable"
    // - a product defect - when the real situation is a missing fixture.
    if !python_is_usable() {
        eprintln!(
                "skipping build_runtime_plugin_state_discovers_mcp_tools_and_surfaces_pending_servers: \
                 `{}` is not a working interpreter on this host",
                python_command()
            );
        return;
    }
    let config_home = temp_dir();
    let workspace = temp_dir();
    fs::create_dir_all(&config_home).expect("config home");
    fs::create_dir_all(&workspace).expect("workspace");
    let script_path = workspace.join("fixture-mcp.py");
    write_mcp_server_fixture(&script_path);
    let settings = json!({
        "mcpServers": {
            "alpha": {
                "command": python_command(),
                "args": [script_path.to_string_lossy()]
            },
            "broken": {
                "command": python_command(),
                "args": ["-c", "import sys; sys.exit(0)"]
            }
        }
    });
    fs::write(config_home.join("settings.json"), serde_json::to_string_pretty(&settings).unwrap())
        .expect("write mcp settings");

    let loader = ConfigLoader::new(&workspace, &config_home);
    let runtime_config = loader.load().expect("runtime config should load");
    let state = build_runtime_plugin_state_with_loader(&workspace, &loader, &runtime_config)
        .expect("runtime plugin state should load");

    let allowed = state
        .tool_registry
        .normalize_allowed_tools(&["mcp__alpha__echo".to_string(), "MCPTool".to_string()])
        .expect("mcp tools should be allow-listable")
        .expect("allow-list should exist");
    assert!(allowed.contains("mcp__alpha__echo"));
    assert!(allowed.contains("MCPTool"));

    let mut executor =
        CliToolExecutor::new(None, false, state.tool_registry.clone(), state.mcp_state.clone());

    let tool_output = executor
        .execute("mcp__alpha__echo", r#"{"text":"hello"}"#)
        .expect("discovered mcp tool should execute");
    let tool_json: serde_json::Value =
        serde_json::from_str(&tool_output).expect("tool output should be json");
    assert_eq!(tool_json["structuredContent"]["echoed"], "hello");

    let wrapped_output = executor
        .execute(
            "MCPTool",
            r#"{"qualifiedName":"mcp__alpha__echo","arguments":{"text":"wrapped"}}"#,
        )
        .expect("generic mcp wrapper should execute");
    let wrapped_json: serde_json::Value =
        serde_json::from_str(&wrapped_output).expect("wrapped output should be json");
    assert_eq!(wrapped_json["structuredContent"]["echoed"], "wrapped");

    let search_output = executor
        .execute("ToolSearch", r#"{"query":"alpha echo","max_results":5}"#)
        .expect("tool search should execute");
    let search_json: serde_json::Value =
        serde_json::from_str(&search_output).expect("search output should be json");
    assert_eq!(search_json["matches"][0], "mcp__alpha__echo");
    assert_eq!(search_json["pending_mcp_servers"][0], "broken");
    assert_eq!(search_json["mcp_degraded"]["failed_servers"][0]["server_name"], "broken");
    assert_eq!(search_json["mcp_degraded"]["failed_servers"][0]["phase"], "tool_discovery");
    assert_eq!(search_json["mcp_degraded"]["available_tools"][0], "mcp__alpha__echo");

    let listed = executor
        .execute("ListMcpResourcesTool", r#"{"server":"alpha"}"#)
        .expect("resources should list");
    let listed_json: serde_json::Value =
        serde_json::from_str(&listed).expect("resource output should be json");
    assert_eq!(listed_json["resources"][0]["uri"], "file://guide.txt");

    let read = executor
        .execute("ReadMcpResourceTool", r#"{"server":"alpha","uri":"file://guide.txt"}"#)
        .expect("resource should read");
    let read_json: serde_json::Value =
        serde_json::from_str(&read).expect("resource read output should be json");
    assert_eq!(read_json["contents"][0]["text"], "contents for file://guide.txt");

    if let Some(mcp_state) = state.mcp_state {
        mcp_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .shutdown()
            .expect("mcp shutdown should succeed");
    }

    let _ = fs::remove_dir_all(config_home);
    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn build_runtime_plugin_state_surfaces_unsupported_mcp_servers_structurally() {
    let config_home = temp_dir();
    let workspace = temp_dir();
    fs::create_dir_all(&config_home).expect("config home");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(
        config_home.join("settings.json"),
        r#"{
              "mcpServers": {
                "remote": {
                  "url": "https://example.test/mcp"
                }
              }
            }"#,
    )
    .expect("write mcp settings");

    let loader = ConfigLoader::new(&workspace, &config_home);
    let runtime_config = loader.load().expect("runtime config should load");
    let state = build_runtime_plugin_state_with_loader(&workspace, &loader, &runtime_config)
        .expect("runtime plugin state should load");
    let mut executor =
        CliToolExecutor::new(None, false, state.tool_registry.clone(), state.mcp_state.clone());

    let search_output = executor
        .execute("ToolSearch", r#"{"query":"remote","max_results":5}"#)
        .expect("tool search should execute");
    let search_json: serde_json::Value =
        serde_json::from_str(&search_output).expect("search output should be json");
    assert_eq!(search_json["pending_mcp_servers"][0], "remote");
    assert_eq!(search_json["mcp_degraded"]["failed_servers"][0]["server_name"], "remote");
    assert_eq!(search_json["mcp_degraded"]["failed_servers"][0]["phase"], "server_registration");
    assert_eq!(
        search_json["mcp_degraded"]["failed_servers"][0]["error"]["context"]["transport"],
        "http"
    );

    let _ = fs::remove_dir_all(config_home);
    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn build_runtime_runs_plugin_lifecycle_init_and_shutdown() {
    // This test injects DEEPSEEK_API_KEY and other tests read it (rebuilding the
    // runtime for a DeepSeek model needs it), so the mutex is what stops the two
    // from racing. The guard is what stops a failure here from leaking the key.
    let _env_guard = env_lock();
    let config_home = temp_dir();
    // Inject a dummy API key so runtime construction succeeds without real credentials.
    // This test only exercises plugin lifecycle (init/shutdown), never calls the API.
    let _deepseek_key = EnvVarGuard::set("DEEPSEEK_API_KEY", "test-dummy-key-for-plugin-lifecycle");
    let workspace = temp_dir();
    let source_root = temp_dir();
    fs::create_dir_all(&config_home).expect("config home");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&source_root).expect("source root");
    write_plugin_fixture(&source_root, "lifecycle-runtime-demo", false, true);

    let mut manager = PluginManager::new(PluginManagerConfig::new(&config_home));
    let install = manager
        .install(source_root.to_str().expect("utf8 source path"))
        .expect("plugin install should succeed");
    let log_path = install.install_path.join("lifecycle.log");
    let loader = ConfigLoader::new(&workspace, &config_home);
    let runtime_config = loader.load().expect("runtime config should load");
    let runtime_plugin_state =
        build_runtime_plugin_state_with_loader(&workspace, &loader, &runtime_config)
            .expect("plugin state should load");
    let mut runtime = build_runtime_with_plugin_state(
        Session::new(),
        "runtime-plugin-lifecycle",
        default_model(),
        vec!["test system prompt".to_string()],
        true,
        false,
        None,
        PermissionMode::DangerFullAccess,
        None,
        runtime_plugin_state,
    )
    .expect("runtime should build");

    assert_eq!(
        fs::read_to_string(&log_path).expect("init log should exist").replace("\r\n", "\n"),
        "init\n"
    );

    runtime.shutdown_plugins().expect("plugin shutdown should succeed");

    assert_eq!(
        fs::read_to_string(&log_path).expect("shutdown log should exist").replace("\r\n", "\n"),
        "init\nshutdown\n"
    );

    let _ = fs::remove_dir_all(config_home);
    let _ = fs::remove_dir_all(workspace);
    let _ = fs::remove_dir_all(source_root);
    // `_deepseek_key` restores the variable on drop, including if this test fails.
}

// ---------------------------------------------------------------------------
// §1.23 class B/C: the return-value contract of `LiveCli`'s state-changing
// methods, and of the 189-line REPL command dispatcher.
//
// The `bool` is not "did it work" - it is "the session changed, so persist it".
// `run_repl` writes the session file exactly when `handle_repl_command` returns
// true. So the contract has two failure directions and both matter: a change
// reported as `false` is lost when the user exits, and a read-only command
// reported as `true` costs a pointless write. These tests fix both directions
// without asserting on stdout, which is class E's job, not this one's.
// ---------------------------------------------------------------------------

const LIVE_CLI_TEST_API_KEY: &str = "test-dummy-key-for-live-cli-tests";
const LIVE_CLI_TEST_AUTH_TOKEN: &str = "test-dummy-token-for-live-cli-tests";
const LIVE_CLI_TEST_DEEPSEEK_KEY: &str = "test-dummy-deepseek-key-for-live-cli-tests";

/// Run `body` inside a fresh, isolated workspace directory.
///
/// The isolation recipe: `env_lock` keeps other tests out of the process-global
/// environment, `EnvRestore::isolate` redirects HOME and the config home into a
/// temp root, and `with_current_dir` makes the temp root the working directory -
/// which is what both `LiveCli::new` and the `cli_context` collectors read.
///
/// This layer holds the environment setup for every test that needs a workspace,
/// so there is one copy of it rather than several that can drift apart. Concrete
/// credentials are injected because switching the model rebuilds the runtime for
/// that model's provider: a test that changes to a DeepSeek model needs a DeepSeek
/// key, and taking that from a neighbour's leak made this file's outcome depend on
/// thread scheduling. Nothing here reaches the network.
fn with_isolated_workspace<T>(body: impl FnOnce(&Path) -> T) -> T {
    let _env_guard = env_lock();
    let env_root = temp_dir();
    let _env = EnvRestore::isolate(&env_root);
    let _api_key = EnvVarGuard::set("ANTHROPIC_API_KEY", LIVE_CLI_TEST_API_KEY);
    let _auth_token = EnvVarGuard::set("ANTHROPIC_AUTH_TOKEN", LIVE_CLI_TEST_AUTH_TOKEN);
    let _deepseek_key = EnvVarGuard::set("DEEPSEEK_API_KEY", LIVE_CLI_TEST_DEEPSEEK_KEY);

    let root = temp_dir();
    fs::create_dir_all(&root).expect("workspace root");
    let result = with_current_dir(&root, || body(&root));
    // Cleanup may fail while a handle is still closing; that must not manufacture
    // a failure in a test that already passed.
    let _ = fs::remove_dir_all(&root);
    result
}

/// Run `body` against a `LiveCli` built inside an isolated workspace, handing
/// over ownership.
///
/// The consuming builder methods (`with_machine_output`) cannot be reached
/// through a `&mut` borrow, so the setup hands the value over and `with_live_cli`
/// is a thin borrow-shaped wrapper over it.
fn with_owned_live_cli<T>(body: impl FnOnce(LiveCli, &Path) -> T) -> T {
    with_isolated_workspace(|root| {
        let cli = LiveCli::new(
            "claude-sonnet-4-6".to_string(),
            true,
            None,
            PermissionMode::DangerFullAccess,
        )
        .expect("cli should initialize");
        body(cli, root)
    })
}

/// Run `body` against a `LiveCli` built in an isolated workspace, by reference.
fn with_live_cli<T>(body: impl FnOnce(&mut LiveCli, &Path) -> T) -> T {
    with_owned_live_cli(|mut cli, root| body(&mut cli, root))
}

#[test]
fn live_cli_writes_its_session_file_before_the_first_command() {
    with_live_cli(|cli, _root| {
        let path = cli.session_path().to_path_buf();
        assert!(
            path.is_file(),
            "the session file must exist after construction, because that is the file \
             `persist_session` - and therefore every `true` from the dispatcher - writes: {}",
            path.display()
        );
    });
}

#[test]
fn set_model_reports_a_change_only_when_the_resolved_model_differs() {
    with_live_cli(|cli, _root| {
        assert_eq!(cli.model_name(), "claude-sonnet-4-6");

        assert!(
            !cli.set_model(None).expect("a model query must not fail"),
            "asking for the model is a query, not a change"
        );
        assert_eq!(cli.model_name(), "claude-sonnet-4-6");

        assert!(
            !cli.set_model(Some("sonnet".to_string())).expect("an alias must resolve"),
            "`sonnet` resolves to the model already in use, so nothing changed"
        );

        assert!(
            cli.set_model(Some("deepseek-v4.1-flash".to_string()))
                .expect("switching must not fail"),
            "a real switch must be reported, otherwise the new model is never persisted"
        );
        assert_eq!(
            cli.model_name(),
            "deepseek-flash",
            "the resolved name is stored, not the alias the user typed"
        );
    });
}

#[test]
fn set_permissions_reports_a_change_and_names_every_valid_mode_when_rejecting() {
    with_live_cli(|cli, _root| {
        assert!(
            !cli.set_permissions(None).expect("a permissions query must not fail"),
            "asking for the permission mode is a query, not a change"
        );

        let error = cli
            .set_permissions(Some("yolo".to_string()))
            .expect_err("an unknown mode must be rejected rather than ignored");
        let message = error.to_string();
        for mode in ["read-only", "workspace-write", "danger-full-access"] {
            assert!(
                message.contains(mode),
                "the rejection must name `{mode}` so the user can recover: {message}"
            );
        }

        assert!(
            cli.set_permissions(Some("read-only".to_string())).expect("a valid mode must not fail"),
            "leaving danger-full-access for read-only is a change and must be persisted"
        );
        assert!(
            !cli.set_permissions(Some("read-only".to_string()))
                .expect("re-selecting must not fail"),
            "re-selecting the mode already in force is not a change"
        );
    });
}

#[test]
fn clear_session_requires_confirmation_and_only_then_switches_identity() {
    with_live_cli(|cli, _root| {
        let before_id = cli.session_id().to_string();
        let before_path = cli.session_path().to_path_buf();

        assert!(
            !cli.clear_session(false).expect("refusing must not fail"),
            "without --confirm nothing may change"
        );
        assert_eq!(cli.session_id(), before_id, "an unconfirmed clear must not switch sessions");

        assert!(
            cli.clear_session(true).expect("a confirmed clear must not fail"),
            "a confirmed clear is a change and must be persisted"
        );
        assert_ne!(cli.session_id(), before_id, "a confirmed clear must move to a new session id");
        assert!(
            before_path.is_file(),
            "the previous session file must survive, because the report tells the user to \
             /resume it: {}",
            before_path.display()
        );
    });
}

#[test]
fn resume_session_without_a_reference_is_a_no_op_and_a_bad_reference_fails() {
    with_live_cli(|cli, _root| {
        let before = cli.session_id().to_string();

        assert!(
            !cli.resume_session(None).expect("usage must not fail"),
            "a bare /resume only prints usage"
        );
        assert_eq!(cli.session_id(), before);

        let error = cli
            .resume_session(Some("definitely-not-a-session".to_string()))
            .expect_err("an unknown session must fail rather than silently keep the current one");
        assert!(!error.to_string().is_empty(), "the failure must say something");
    });
}

#[test]
fn switching_to_the_current_workspace_is_not_a_change() {
    with_live_cli(|cli, _root| {
        assert!(
            !cli.switch_workspace(".").expect("the current directory must resolve"),
            "re-selecting the current workspace must not tear the session down and replace it"
        );
    });
}

#[test]
fn handle_plugins_command_reports_without_claiming_a_change() {
    with_live_cli(|cli, _root| {
        assert!(
            !cli.handle_plugins_command(None, None).expect("listing plugins must not fail"),
            "reading the plugin list does not change the session"
        );
    });
}

#[test]
fn read_only_repl_commands_never_ask_for_a_persist() {
    let read_only = [
        SlashCommand::Help,
        SlashCommand::Dir,
        SlashCommand::Sandbox,
        SlashCommand::Cost,
        SlashCommand::Version,
        SlashCommand::Memory,
        SlashCommand::Config { section: None },
        SlashCommand::Unknown("definitely-not-a-command".to_string()),
    ];
    with_live_cli(|cli, _root| {
        for command in read_only {
            let label = format!("{command:?}");
            let reported = cli
                .handle_repl_command(command)
                .unwrap_or_else(|error| panic!("{label} must not fail: {error}"));
            assert!(
                !reported,
                "{label} changes nothing, so it must not ask for the session to be written"
            );
        }
    });
}

#[test]
fn not_implemented_commands_report_and_never_ask_for_a_persist() {
    // The family that prints "Command registered but not yet implemented."
    // Reporting a change here would be worse than the message: it would look
    // like the command had done something worth keeping.
    let unimplemented = [
        SlashCommand::Login,
        SlashCommand::Logout,
        SlashCommand::Vim,
        SlashCommand::Upgrade,
        SlashCommand::Stats,
        SlashCommand::Share,
        SlashCommand::Feedback,
        SlashCommand::Files,
        SlashCommand::Fast,
        SlashCommand::Exit,
        SlashCommand::Summary,
        SlashCommand::Desktop,
    ];
    with_live_cli(|cli, _root| {
        for command in unimplemented {
            let label = format!("{command:?}");
            let reported = cli
                .handle_repl_command(command)
                .unwrap_or_else(|error| panic!("{label} must not fail: {error}"));
            assert!(!reported, "{label} is not implemented, so it must not ask for a persist");
        }
    });
}

#[test]
fn the_dispatcher_propagates_the_state_changing_answers() {
    // Most arms of the dispatcher return `false` themselves, so the risk this
    // test covers is an arm that calls a method which *did* change state and
    // then drops that fact - the change would never be written.
    with_live_cli(|cli, _root| {
        assert!(
            !cli.handle_repl_command(SlashCommand::Model { model: None })
                .expect("a model query must not fail"),
            "a model query changes nothing"
        );

        assert!(
            cli.handle_repl_command(SlashCommand::Model {
                model: Some("deepseek-v4.1-flash".to_string()),
            })
            .expect("switching the model must not fail"),
            "the dispatcher must pass on the fact that the model changed"
        );
        assert_eq!(cli.model_name(), "deepseek-flash");

        assert!(
            cli.handle_repl_command(SlashCommand::Permissions {
                mode: Some("read-only".to_string())
            })
            .expect("switching permissions must not fail"),
            "the dispatcher must pass on the fact that the permission mode changed"
        );

        assert!(
            !cli.handle_repl_command(SlashCommand::Clear { confirm: false })
                .expect("an unconfirmed clear must not fail"),
            "an unconfirmed clear changes nothing"
        );

        assert!(
            cli.handle_repl_command(SlashCommand::Clear { confirm: true })
                .expect("a confirmed clear must not fail"),
            "the dispatcher must pass on the fact that the session was replaced"
        );
    });
}

#[test]
fn switching_to_a_different_workspace_reports_the_change_and_moves() {
    // The companion to the test above: it is not enough for a re-selection to be
    // a no-op, a real switch must still happen. Without this pair, "always
    // return false" would satisfy the other test.
    with_live_cli(|cli, root| {
        let other = root.join("other-workspace");
        fs::create_dir_all(&other).expect("other workspace");
        let expected = other.canonicalize().expect("the sibling must resolve");

        assert!(
            cli.switch_workspace(&other.display().to_string())
                .expect("a real switch must not fail"),
            "moving to a different directory is a change and must be persisted"
        );
        let actual = std::env::current_dir()
            .expect("cwd must load")
            .canonicalize()
            .expect("cwd must resolve");
        assert_eq!(actual, expected, "the process must actually be in the new workspace");
        assert!(
            cli.session_path().is_file(),
            "the new workspace gets its own session file, written before the move completes"
        );
    });
}

// ---------------------------------------------------------------------------
// §1.23 class A: the pure-logic surface of `LiveCli` - the accessors, the
// consuming builder, and the banner. No network, and the only I/O is the
// session file `LiveCli::new` writes.
// ---------------------------------------------------------------------------

#[test]
fn accessors_report_the_identity_the_constructor_established() {
    with_live_cli(|cli, _root| {
        assert_eq!(cli.model_name(), "claude-sonnet-4-6");
        assert!(
            !cli.session_id().is_empty(),
            "an empty id would make the `contains(id)` checks below vacuous"
        );

        let path = cli.session_path();
        assert!(
            path.is_absolute(),
            "this accessor is the path the recovery record and /resume use, so it must be \
             absolute: {}",
            path.display()
        );

        // `render_resume_usage` and the REPL help both promise this layout, and
        // `/resume <session-id>` resolves through it - so the promise and the
        // behaviour must not drift. Comparing components also keeps this
        // platform-neutral, which a string comparison would not.
        let documented =
            Path::new(".claw").join("sessions").join(format!("{}.jsonl", cli.session_id()));
        assert!(
            path.ends_with(&documented),
            "a managed session must live where the help says it does: {} should end with {}",
            path.display(),
            documented.display()
        );
    });
}

#[test]
fn startup_banner_states_the_identity_and_a_workspace_relative_session_path() {
    with_live_cli(|cli, _root| {
        let banner = cli.startup_banner();

        // The banner is the first thing a user reads, and it states the model
        // and the permission mode. A stale permission mode here would mislead
        // the user about what the agent is allowed to do.
        assert!(banner.contains("claude-sonnet-4-6"), "the model must be stated:\n{banner}");
        assert!(
            banner.contains("danger-full-access"),
            "the permission mode must be stated, because it is what the agent may do:\n{banner}"
        );
        assert!(banner.contains("Permissions"), "{banner}");
        assert!(banner.contains("Auto-save"), "{banner}");

        assert!(banner.contains(cli.session_id()), "the session id must be stated:\n{banner}");
        assert!(banner.contains(".claw"), "the auto-save location must be stated:\n{banner}");

        // The banner prints the path relative to the workspace - it strips the
        // directory it just printed one line above - while the accessor returns
        // the absolute form. The two differ on purpose, and if the strip ever
        // stopped working the banner would repeat a long absolute path on every
        // line that mentions the session.
        let absolute = cli.session_path().display().to_string();
        assert!(
            !banner.contains(&absolute),
            "the session path in the banner must be relative to the workspace, so the \
             absolute form must not appear:\n{banner}"
        );
    });
}

#[test]
fn with_machine_output_keeps_the_identity_it_was_built_with() {
    with_owned_live_cli(|cli, _root| {
        let id = cli.session_id().to_string();
        let path = cli.session_path().to_path_buf();
        let model = cli.model_name().to_string();

        let machine = cli.with_machine_output();

        // A consuming builder must not disturb what it carries. The
        // machine-output CLI is the same session, at the same file, on the same
        // model: the governed sidecar path persists its review under exactly
        // this identity, so a builder that quietly replaced any part of it would
        // write the result somewhere the caller is not looking.
        assert_eq!(machine.session_id(), id);
        assert_eq!(machine.session_path(), path);
        assert_eq!(machine.model_name(), model);
        assert!(
            machine.session_path().is_file(),
            "the builder must not disturb the session file either: {}",
            machine.session_path().display()
        );
    });
}

// ---------------------------------------------------------------------------
// The `cli_context` collectors: they build the view types that `cli_reports`
// formats. These run them for real - against a workspace the test owns - rather
// than against the machine the suite happens to be on, which is the only way to
// assert what they report without asserting this developer's git state.
// ---------------------------------------------------------------------------

#[test]
fn workspace_context_describes_the_directory_it_ran_in() {
    with_isolated_workspace(|root| {
        let context = workspace_context().expect("the workspace context must load");

        // Resolve both sides before comparing. `temp_dir()` and
        // `env::current_dir()` are not spelled the same way for the same
        // directory on two platforms: Windows adds a verbatim prefix when it
        // canonicalizes, and macOS resolves `/var` to `/private/var`. Comparing
        // the raw values compares spellings, not directories.
        assert_eq!(
            fs::canonicalize(&context.cwd).expect("cwd must resolve"),
            fs::canonicalize(root).expect("root must resolve"),
            "the context must describe the directory the command ran in"
        );
        assert!(
            context.session_dir.ends_with(Path::new(".claw").join("sessions")),
            "the session directory must be the one the help documents: {}",
            context.session_dir.display()
        );
        // Stated as a rule rather than as an equality against a path built from
        // `root`: what matters is that the recovery directory sits inside the
        // workspace the context describes and is named where the help says. An
        // equality would also be comparing spellings, since the collector derives
        // it from `current_dir()` and `root` is the unresolved temp path.
        assert!(
            context.recovery_dir.ends_with(Path::new(".sego").join("recovery")),
            "the recovery directory must be the documented one: {}",
            context.recovery_dir.display()
        );
        assert!(
            context.recovery_dir.starts_with(&context.cwd),
            "the recovery directory must sit inside the workspace the context describes: {} vs {}",
            context.recovery_dir.display(),
            context.cwd.display()
        );

        // A directory that is not a git worktree has no project root, and the
        // collector must say so rather than inventing one - `/workspace` prints
        // exactly this value.
        assert!(
            context.project_root.is_none(),
            "a non-git workspace must report no project root, got {:?}",
            context.project_root
        );
    });
}

#[test]
fn status_context_reports_the_workspace_and_carries_the_session_path() {
    with_isolated_workspace(|root| {
        let session = root.join(".claw").join("sessions").join("carried.jsonl");

        let context = status_context(Some(&session)).expect("the status context must load");

        assert_eq!(
            fs::canonicalize(&context.cwd).expect("cwd must resolve"),
            fs::canonicalize(root).expect("root must resolve")
        );

        // Whatever path it is handed must come back unchanged: that is the value
        // `/status` prints under `Session`, so dropping it would leave the report
        // claiming the live-REPL placeholder for a session that has a file.
        assert_eq!(
            context.session_path.as_deref(),
            Some(session.as_path()),
            "the session path must be carried through untouched"
        );

        // The two config counters answer different questions, and conflating them
        // is easy: `discover` is the list of locations the loader *looks at*
        // (two user, two project, one local) and does not depend on whether any
        // of them exist, while `loaded` counts the ones that did. In an empty
        // workspace nothing loads, so the numbers differ - and `/status` prints
        // them as `loaded N/M`, which reads as "N of the M places checked".
        assert_eq!(
            context.discovered_config_files, 5,
            "the candidate set is fixed: two user locations, two project, one local"
        );
        assert_eq!(
            context.loaded_config_files, 0,
            "an empty workspace must load nothing, whatever it looks at"
        );
        assert_eq!(context.memory_file_count, 0, "an empty workspace has no memory files");

        // No git status was available, so every count is zero. Note what that
        // means for the report: `GitWorkspaceSummary::is_clean` is
        // `changed_files == 0`, so a non-git directory is reported as `clean`.
        // That reading is pre-existing behaviour and this test pins the counts,
        // not a claim that "clean" is the right word for "not a repository".
        assert_eq!((context.git_summary.changed_files, context.git_summary.staged_files), (0, 0));
        assert_eq!(context.git_branch, None, "no repository means no branch to name");
    });
}
