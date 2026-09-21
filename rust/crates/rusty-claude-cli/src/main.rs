#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    clippy::unneeded_struct_pattern,
    clippy::unnecessary_wraps,
    clippy::unused_self
)]
mod acceptance_display;
mod full_scope_preflight;
mod init;
mod input;
mod nl_intent;
mod render;
mod review_card;
mod sidecar;
mod task_parser;

use std::collections::{BTreeSet, HashSet};
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, UNIX_EPOCH};

use api::{
    detect_provider_kind, ContentBlockDelta, InputContentBlock, InputMessage, MessageRequest,
    MessageResponse, OutputContentBlock, ProviderClient, ProviderKind,
    StreamEvent as ApiStreamEvent, ToolChoice, ToolDefinition, ToolResultContentBlock,
};

use commands::{
    handle_agents_slash_command, handle_mcp_slash_command, handle_plugins_slash_command,
    handle_skills_slash_command, render_slash_command_help, resume_supported_slash_commands,
    slash_command_specs, validate_slash_command_input, SlashCommand,
};
use compat_harness::{extract_manifest, UpstreamPaths};
use init::initialize_repo;
use nl_intent::{classify_nl_intent_miss, parse_nl_intent, NlIntent, NlIntentMiss};
use plugins::{PluginHooks, PluginManager, PluginManagerConfig, PluginRegistry};
use render::{MarkdownStreamState, Spinner, TerminalRenderer};
use review_card::{
    local_file_url, render_html as render_review_card_html,
    render_terminal_summary as render_review_card_terminal_summary, ReviewCardData,
};
use runtime::{
    build_review_prompt, build_verification_plan, clear_oauth_credentials,
    community_learning::CommunityLearning,
    evaluate, format_usd, generate_pkce_pair, generate_state,
    green_contract::GreenLevel,
    latest_review_finding_statuses, load_review_index, load_system_prompt,
    parse_oauth_callback_request_target, persist_review_artifact, pricing_for_model,
    record_review_finding_status, resolve_sandbox_status, save_oauth_credentials,
    workflow::{SessionReport, WorkflowSnapshot, WorkflowStore},
    ApiClient, ApiRequest, AssistantEvent, CompactionConfig, ConfigLoader, ConfigSource,
    ContentBlock, ConversationMessage, ConversationRuntime, DiffScope, LaneBlocker, LaneContext,
    LaneEvent, LaneEventBlocker, LaneEventName, LaneEventStatus, LaneFailureClass,
    McpServerManager, McpTool, MessageRole, ModelPricing, OAuthAuthorizationRequest, OAuthConfig,
    OAuthTokenExchangeRequest, PermissionMode, PermissionPolicy, Phase, PhaseLogEntry, PhaseStatus,
    ProgressUI, ProjectContext, PromptCacheEvent, ResolvedPermissionMode, ReviewContext,
    ReviewFindingStatus, ReviewIndexEntry, ReviewPromptOptions, ReviewReport, ReviewScope,
    ReviewStatus, ReviewTarget, RuntimeError, Session, TokenUsage, ToolError, ToolExecutor,
    UsageTracker, VerificationCommand, VerificationPlanStatus, VerificationScope,
};
use serde::Deserialize;
use serde_json::json;
use tools::{GlobalToolRegistry, RuntimeToolDefinition, ToolSearchOutput};

const NO_ASSISTANT_RESPONSE_EXPORT_REASON: &str =
    "no assistant response is available to export yet";

#[derive(Debug)]
struct NoAssistantResponseExportError;

impl std::fmt::Display for NoAssistantResponseExportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(NO_ASSISTANT_RESPONSE_EXPORT_REASON)
    }
}

impl std::error::Error for NoAssistantResponseExportError {}

fn is_no_assistant_response_export_error(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(error) = current {
        if error.is::<NoAssistantResponseExportError>() {
            return true;
        }
        current = error.source();
    }
    false
}

fn max_tokens_for_model(model: &str) -> u32 {
    // DeepSeek, MiMo, and GPT all support 64K output
    64_000
}

fn context_window_limit(model: &str) -> u32 {
    // DeepSeek V4 is the only family here with a window wider than 128K. MiMo
    // and the GPT models are both assumed to be 128K, which is also the
    // conservative default for anything unrecognised - so there is one
    // threshold, not four branches that happen to agree.
    if model.contains("deepseek") || model.contains("v4") {
        1_000_000
    } else {
        128_000
    }
}

fn check_context_preflight(
    model: &str,
    estimated_input_tokens: u32,
    requested_output: u32,
) -> Option<String> {
    let limit = context_window_limit(model);
    let total = estimated_input_tokens + requested_output;
    if total > limit {
        Some(format!(
            "⚠ Context warning: estimated {estimated_input_tokens} input + {requested_output} output tokens ({total} total) exceeds model limit ({limit}). Consider running /compact."
        ))
    } else if total > limit * 90 / 100 {
        Some(format!(
            "⚠ Approaching context limit: {total} / {limit} tokens ({}%). Consider /compact soon.",
            total * 100 / limit
        ))
    } else {
        None
    }
}
const DEFAULT_OAUTH_CALLBACK_PORT: u16 = 4545;
const VERSION: &str = env!("CARGO_PKG_VERSION");
const BUILD_TARGET: Option<&str> = option_env!("TARGET");
const GIT_SHA: Option<&str> = option_env!("GIT_SHA");
const INTERNAL_PROGRESS_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(3);
const PRIMARY_SESSION_EXTENSION: &str = "jsonl";
const LEGACY_SESSION_EXTENSION: &str = "json";
const LATEST_SESSION_REFERENCE: &str = "latest";
const SESSION_REFERENCE_ALIASES: &[&str] = &[LATEST_SESSION_REFERENCE, "last", "recent"];
const UPDATE_CHECK_ENV: &str = "SEGO_SKIP_UPDATE_CHECK";
const UPDATE_LATEST_URL: &str = "https://api.github.com/repos/007M7/Sego-Agent/releases/latest";
const UPDATE_WINDOWS_ASSET: &str = "sego.exe";
const UPDATE_CHECKSUMS_ASSET: &str = "checksums.txt";
/// Explicit opt-out from verifying a downloaded update. Named so it can be
/// documented and so an accidental set is visible in the warning it prints.
const UPDATE_ALLOW_UNVERIFIED_ENV: &str = "SEGO_UPDATE_ALLOW_UNVERIFIED";
type AllowedToolSet = BTreeSet<String>;

const TURN_CANCELLED_MESSAGE: &str = "conversation turn cancelled by user";

fn main() {
    if let Err(error) = run() {
        let message = error.to_string();
        if is_turn_cancelled_message(&message) {
            eprintln!("Sego cancelled the current task.");
            maybe_pause_after_error();
            std::process::exit(130);
        } else if message.contains("`sego --help`") || message == "verification failed" {
            eprintln!("error: {message}");
        } else {
            eprintln!(
                "error: {message}

Run `sego --help` for usage."
            );
        }
        maybe_pause_after_error();
        std::process::exit(1);
    }
}

fn is_turn_cancelled_message(message: &str) -> bool {
    message.contains(TURN_CANCELLED_MESSAGE)
}

fn is_turn_cancelled_error(error: &(dyn std::error::Error + 'static)) -> bool {
    is_turn_cancelled_message(&error.to_string())
}

fn maybe_pause_after_error() {
    if !cfg!(windows) || env::var_os("SEGO_PAUSE_ON_ERROR").is_none() {
        return;
    }

    eprintln!();
    eprintln!("Press Enter to close this window.");
    let mut buffer = String::new();
    let _ = io::stdin().read_line(&mut buffer);
}

// The dispatch table for every CLI action, kept in one place so the set of
// actions and their argument shapes can be read together.
#[allow(clippy::too_many_lines)]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let action = parse_args(&args)?;

    // A2 启动提示层：只对会创建/恢复可持久化 session 且可能执行模型/工具的动作触发。
    // 只读 recovery JSON，不写、不扫描、不调模型（Codex 审查两层架构第 1 层）。
    let triggers_recovery = matches!(
        action,
        CliAction::Repl { .. }
            | CliAction::Prompt { .. }
            | CliAction::CodeReview { .. }
            | CliAction::ResumeSession { .. }
    );
    let triggers_update_check = matches!(
        action,
        CliAction::Repl { .. } | CliAction::Prompt { .. } | CliAction::CodeReview { .. }
    );
    maybe_print_update_notice(triggers_update_check);
    maybe_print_recovery_notice(triggers_recovery);

    match action {
        CliAction::DumpManifests => dump_manifests(),
        CliAction::BootstrapPlan => print_bootstrap_plan(),
        CliAction::SidecarReview => {
            // c9/c PoC: stdin JSON → stdout JSON. Exit code from pipeline.
            let exit_code = sidecar::run_sidecar_review_pipeline();
            std::process::exit(exit_code);
        }
        CliAction::Agents { args } => LiveCli::print_agents(args.as_deref())?,
        CliAction::Mcp { args } => LiveCli::print_mcp(args.as_deref())?,
        CliAction::Skills { args } => LiveCli::print_skills(args.as_deref())?,
        CliAction::PrintSystemPrompt { cwd, date } => print_system_prompt(cwd, date),
        CliAction::Version => print_version(),
        CliAction::Update { check_only } => run_update(check_only)?,
        CliAction::Workspace { output_format } => print_workspace_snapshot(output_format)?,
        CliAction::ResumeSession { session_path, commands } => {
            resume_session(&session_path, &commands);
        }
        CliAction::Status { model, permission_mode, output_format } => {
            print_status_snapshot(&model, permission_mode, output_format)?;
        }
        CliAction::Sandbox { output_format } => print_sandbox_status_snapshot(output_format)?,
        CliAction::Prompt { prompt, model, output_format, allowed_tools, permission_mode } => {
            // C20.6-C R2: check for required review commands in task-like input first.
            match task_parser::parse_required_review_command(&prompt) {
                task_parser::RequiredReviewResult::Execute { scope } => {
                    run_code_review_cli(model, allowed_tools, permission_mode, Some(&scope))?;
                    return Ok(());
                }
                task_parser::RequiredReviewResult::Blocked { detected, reason, guidance } => {
                    println!(
                        "Task command blocked
  Detected         {detected}
  Reason           {reason}
  Guidance         {guidance}"
                    );
                    return Ok(());
                }
                task_parser::RequiredReviewResult::None => {}
            }

            // Route high-confidence NL review intents to deterministic code review
            // before falling through to the model conversation path. (Cycle 15 P0-B)
            if let Some(intent) = parse_nl_intent(&prompt) {
                match intent {
                    NlIntent::Review { scope } => {
                        run_code_review_cli(
                            model,
                            allowed_tools,
                            permission_mode,
                            scope.as_deref(),
                        )?;
                        return Ok(());
                    }
                    NlIntent::ReviewSafety { staged } => {
                        let scope = if staged { Some("staged") } else { Some("workspace") };
                        run_code_review_cli(model, allowed_tools, permission_mode, scope)?;
                        return Ok(());
                    }
                    _ => {}
                }
            }

            // A2 session 状态写入：active 在 LiveCli 创建后写，graceful 在成功返回后写。
            // 错误路径（?返回 / 崩溃）不写 graceful，保留 active，下次启动提示可恢复。
            let mut cli = LiveCli::new(model.clone(), true, allowed_tools, permission_mode)?;
            persist_recovery_for_cli(
                runtime::recovery::RecoveryExitState::Active,
                cli.session_id(),
                cli.session_path(),
                Some(cli.model_name()),
                Some(&prompt),
            );
            // The run was launched to answer this prompt, so that is what it is for.
            begin_run_task(cli.session_id(), &prompt);
            cli.run_turn_with_output(&prompt, output_format)?;
            persist_recovery_for_cli(
                runtime::recovery::RecoveryExitState::Graceful,
                cli.session_id(),
                cli.session_path(),
                Some(cli.model_name()),
                Some(&prompt),
            );
            complete_run_task();
        }
        CliAction::CodeReview { scope, model, allowed_tools, permission_mode } => {
            run_code_review_cli(model, allowed_tools, permission_mode, scope.as_deref())?;
        }
        CliAction::CodeReviewList => print_code_review_history()?,
        CliAction::CodeReviewShow { id } => print_code_review_report(&id)?,
        CliAction::CodeReviewShowJson { id } => print_code_review_summary_json(&id)?,
        CliAction::CodeReviewCard { id } => print_code_review_card(&id)?,
        CliAction::CodeReviewStatus { id } => print_code_review_finding_status(&id)?,
        CliAction::CodeReviewMark { id, finding_id, status, note } => {
            mark_code_review_finding(&id, &finding_id, status, note)?;
        }
        CliAction::CodeReviewReady => print_code_review_readiness()?,
        CliAction::CodeReviewSummary => print_code_review_summary()?,
        CliAction::CodeReviewTools => print_code_review_tools()?,
        CliAction::CodeReviewSafety { scope } => print_code_review_safety(scope)?,
        CliAction::CodeVerify { scope } => run_code_verify_cli(scope.as_deref())?,
        CliAction::Login => run_login()?,
        CliAction::Logout => run_logout()?,
        CliAction::Init => run_init()?,
        CliAction::Repl { model, allowed_tools, permission_mode } => {
            run_repl(model, allowed_tools, permission_mode)?;
        }
        CliAction::Help => print_help(),
        CliAction::Dir => println!("{}", render_natural_language_directory()),
        CliAction::Review { last_n, output_format } => {
            print_workflow_review(last_n, output_format)?;
        }
        CliAction::Learn { output_format } => print_workflow_learn(output_format)?,
        CliAction::Doctor { output_format } => print_doctor(output_format)?,
        CliAction::Telemetry { action, output_format } => {
            print_telemetry(action.as_deref(), output_format)?;
        }
        CliAction::PrintAndExit { message } => {
            println!("{message}");
        }
    }
    Ok(())
}

fn permission_mode_from_label(mode: &str) -> PermissionMode {
    match mode {
        "read-only" => PermissionMode::ReadOnly,
        "workspace-write" => PermissionMode::WorkspaceWrite,
        "danger-full-access" => PermissionMode::DangerFullAccess,
        other => panic!("unsupported permission mode label: {other}"),
    }
}

fn permission_mode_from_resolved(mode: ResolvedPermissionMode) -> PermissionMode {
    match mode {
        ResolvedPermissionMode::ReadOnly => PermissionMode::ReadOnly,
        ResolvedPermissionMode::WorkspaceWrite => PermissionMode::WorkspaceWrite,
        ResolvedPermissionMode::DangerFullAccess => PermissionMode::DangerFullAccess,
    }
}

fn default_permission_mode() -> PermissionMode {
    env::var("RUSTY_CLAUDE_PERMISSION_MODE")
        .ok()
        .as_deref()
        .and_then(normalize_permission_mode)
        .map(permission_mode_from_label)
        .or_else(config_permission_mode_for_current_dir)
        .unwrap_or(PermissionMode::ReadOnly)
}

fn config_permission_mode_for_current_dir() -> Option<PermissionMode> {
    let cwd = env::current_dir().ok()?;
    let loader = ConfigLoader::default_for(&cwd);
    loader.load().ok()?.permission_mode().map(permission_mode_from_resolved)
}

fn filter_tool_specs(
    tool_registry: &GlobalToolRegistry,
    allowed_tools: Option<&AllowedToolSet>,
) -> Vec<ToolDefinition> {
    // Token-saving: by default, only send essential tools (~15 instead of 54)
    // Use --allowedTools all for full toolset
    if let Some(set) = allowed_tools {
        if set.contains("all") {
            return tool_registry.definitions(None);
        }
        return tool_registry.definitions(allowed_tools);
    }
    let lite: &AllowedToolSet = &LITE_TOOLS;
    let mut default_tools = lite.clone();
    default_tools.extend(tool_registry.extension_tool_names());
    tool_registry.definitions(Some(&default_tools))
}

fn dump_manifests() {
    let workspace_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let paths = UpstreamPaths::from_workspace_dir(&workspace_dir);
    match extract_manifest(&paths) {
        Ok(manifest) => {
            println!("commands: {}", manifest.commands.entries().len());
            println!("tools: {}", manifest.tools.entries().len());
            println!("bootstrap phases: {}", manifest.bootstrap.phases().len());
        }
        Err(error) => {
            eprintln!("failed to extract manifests: {error}");
            std::process::exit(1);
        }
    }
}

fn print_bootstrap_plan() {
    for phase in runtime::BootstrapPlan::claude_code_default().phases() {
        println!("- {phase:?}");
    }
}

fn default_oauth_config() -> OAuthConfig {
    OAuthConfig {
        client_id: String::from("9d1c250a-e61b-44d9-88ed-5944d1962f5e"),
        authorize_url: String::from("https://platform.claude.com/oauth/authorize"),
        token_url: String::from("https://platform.claude.com/v1/oauth/token"),
        callback_port: None,
        manual_redirect_url: None,
        scopes: vec![
            String::from("user:profile"),
            String::from("user:inference"),
            String::from("user:sessions:claude_code"),
        ],
    }
}

fn run_login() -> Result<(), Box<dyn std::error::Error>> {
    println!("OAuth login is not supported with the current provider setup.");
    println!("Set DEEPSEEK_API_KEY, MIMO_API_KEY, or OPENAI_API_KEY env vars to authenticate.");
    Ok(())
}

fn run_logout() -> Result<(), Box<dyn std::error::Error>> {
    clear_oauth_credentials()?;
    println!("OAuth credentials cleared.");
    Ok(())
}

fn open_browser(url: &str) -> io::Result<()> {
    let commands = if cfg!(target_os = "macos") {
        vec![("open", vec![url])]
    } else if cfg!(target_os = "windows") {
        vec![("cmd", vec!["/C", "start", "", url])]
    } else {
        vec![("xdg-open", vec![url])]
    };
    for (program, args) in commands {
        match Command::new(program).args(args).spawn() {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound, "no supported browser opener command found"))
}

fn wait_for_oauth_callback(
    port: u16,
) -> Result<runtime::OAuthCallbackParams, Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let (mut stream, _) = listener.accept()?;
    let mut buffer = [0_u8; 4096];
    let bytes_read = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let request_line = request.lines().next().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing callback request line")
    })?;
    let target = request_line.split_whitespace().nth(1).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing callback request target")
    })?;
    let callback = parse_oauth_callback_request_target(target)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let body = if callback.error.is_some() {
        "Claude OAuth login failed. You can close this window."
    } else {
        "Claude OAuth login succeeded. You can close this window."
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/plain; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes())?;
    Ok(callback)
}

fn print_system_prompt(cwd: PathBuf, date: String) {
    match load_system_prompt(cwd, date, env::consts::OS, "unknown") {
        Ok(sections) => println!("{}", sections.join("\n\n")),
        Err(error) => {
            eprintln!("failed to build system prompt: {error}");
            std::process::exit(1);
        }
    }
}

fn print_version() {
    println!("{}", render_version_report());
}

fn resume_session(session_path: &Path, commands: &[String]) {
    let resolved_path = if session_path.exists() {
        session_path.to_path_buf()
    } else {
        match resolve_session_reference(&session_path.display().to_string()) {
            Ok(handle) => handle.path,
            Err(error) => {
                eprintln!("failed to restore session: {error}");
                std::process::exit(1);
            }
        }
    };

    let session = match Session::load_from_path(&resolved_path) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("failed to restore session: {error}");
            std::process::exit(1);
        }
    };

    // A2 session 状态写入：session 加载成功后写 active。
    // commands 全部成功完成后写 graceful（函数末尾）。
    // 错误路径（process::exit）不写 graceful，保留 active。
    persist_recovery_for_cli(
        runtime::recovery::RecoveryExitState::Active,
        &session.session_id,
        &resolved_path,
        None,
        None,
    );
    // The run was launched to continue this session, so the file it restored is
    // what the ledger should name if the run is interrupted.
    begin_run_task(&session.session_id, &format!("resume {}", resolved_path.display()));

    if commands.is_empty() {
        println!(
            "Restored session from {} ({} messages).",
            resolved_path.display(),
            session.messages.len()
        );
        // 无命令的纯恢复视为正常退出。
        persist_recovery_for_cli(
            runtime::recovery::RecoveryExitState::Graceful,
            &session.session_id,
            &resolved_path,
            None,
            None,
        );
        complete_run_task();
        return;
    }

    let mut session = session;
    for raw_command in commands {
        let command = match SlashCommand::parse(raw_command) {
            Ok(Some(command)) => command,
            Ok(None) => {
                eprintln!("unsupported resumed command: {raw_command}");
                std::process::exit(2);
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(2);
            }
        };
        match run_resume_command(&resolved_path, &session, &command) {
            Ok(ResumeCommandOutcome { session: next_session, message }) => {
                session = next_session;
                if let Some(message) = message {
                    println!("{message}");
                }
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(2);
            }
        }
    }

    // 所有 resume 命令成功完成，写 graceful。
    persist_recovery_for_cli(
        runtime::recovery::RecoveryExitState::Graceful,
        &session.session_id,
        &resolved_path,
        None,
        None,
    );
    complete_run_task();
}

#[derive(Debug, Clone)]
struct ResumeCommandOutcome {
    session: Session,
    message: Option<String>,
}

#[cfg(test)]
fn format_unknown_slash_command_message(name: &str) -> String {
    let suggestions = suggest_slash_commands(name);
    if suggestions.is_empty() {
        format!("unknown slash command: /{name}. Use /help to list available commands.")
    } else {
        format!(
            "unknown slash command: /{name}. Did you mean {}? Use /help to list available commands.",
            suggestions.join(", ")
        )
    }
}

#[allow(clippy::too_many_lines)]
fn run_resume_command(
    session_path: &Path,
    session: &Session,
    command: &SlashCommand,
) -> Result<ResumeCommandOutcome, Box<dyn std::error::Error>> {
    match command {
        SlashCommand::Help => {
            Ok(ResumeCommandOutcome { session: session.clone(), message: Some(render_repl_help()) })
        }
        SlashCommand::Dir => Ok(ResumeCommandOutcome {
            session: session.clone(),
            message: Some(render_natural_language_directory()),
        }),
        SlashCommand::Compact => {
            let result = runtime::compact_session(
                session,
                CompactionConfig { max_estimated_tokens: 0, ..CompactionConfig::default() },
            );
            let removed = result.removed_message_count;
            let kept = result.compacted_session.messages.len();
            let skipped = removed == 0;
            result.compacted_session.save_to_path(session_path)?;
            Ok(ResumeCommandOutcome {
                session: result.compacted_session,
                message: Some(format_compact_report(removed, kept, skipped)),
            })
        }
        SlashCommand::Clear { confirm } => {
            if !confirm {
                return Ok(ResumeCommandOutcome {
                    session: session.clone(),
                    message: Some(
                        "clear: confirmation required; rerun with /clear --confirm".to_string(),
                    ),
                });
            }
            let backup_path = write_session_clear_backup(session, session_path)?;
            let previous_session_id = session.session_id.clone();
            let cleared = Session::new();
            let new_session_id = cleared.session_id.clone();
            cleared.save_to_path(session_path)?;
            Ok(ResumeCommandOutcome {
                session: cleared,
                message: Some(format!(
                    "Session cleared\n  Mode             resumed session reset\n  Previous session {previous_session_id}\n  Backup           {}\n  Resume previous  claw --resume {}\n  New session      {new_session_id}\n  Session file     {}",
                    backup_path.display(),
                    backup_path.display(),
                    session_path.display()
                )),
            })
        }
        SlashCommand::Status => {
            let tracker = UsageTracker::from_session(session);
            let usage = tracker.cumulative_usage();
            Ok(ResumeCommandOutcome {
                session: session.clone(),
                message: Some(format_status_report(
                    "restored-session",
                    StatusUsage {
                        message_count: session.messages.len(),
                        turns: tracker.turns(),
                        latest: tracker.current_turn_usage(),
                        cumulative: usage,
                        estimated_tokens: 0,
                    },
                    default_permission_mode().as_str(),
                    &status_context(Some(session_path))?,
                )),
            })
        }
        SlashCommand::Sandbox => {
            let cwd = env::current_dir()?;
            let loader = ConfigLoader::default_for(&cwd);
            let runtime_config = loader.load()?;
            Ok(ResumeCommandOutcome {
                session: session.clone(),
                message: Some(format_sandbox_report(&resolve_sandbox_status(
                    runtime_config.sandbox(),
                    &cwd,
                ))),
            })
        }
        SlashCommand::Pwd | SlashCommand::Workspace { path: None } => Ok(ResumeCommandOutcome {
            session: session.clone(),
            message: Some(format_workspace_report(&workspace_context()?)),
        }),
        SlashCommand::Cost => {
            let usage = UsageTracker::from_session(session).cumulative_usage();
            Ok(ResumeCommandOutcome {
                session: session.clone(),
                message: Some(format_cost_report(usage)),
            })
        }
        SlashCommand::Config { section } => Ok(ResumeCommandOutcome {
            session: session.clone(),
            message: Some(render_config_report(section.as_deref())?),
        }),
        SlashCommand::Mcp { action, target } => {
            let cwd = env::current_dir()?;
            let args = match (action.as_deref(), target.as_deref()) {
                (None, None) => None,
                (Some(action), None) => Some(action.to_string()),
                (Some(action), Some(target)) => Some(format!("{action} {target}")),
                (None, Some(target)) => Some(target.to_string()),
            };
            Ok(ResumeCommandOutcome {
                session: session.clone(),
                message: Some(handle_mcp_slash_command(args.as_deref(), &cwd)?),
            })
        }
        SlashCommand::Memory => Ok(ResumeCommandOutcome {
            session: session.clone(),
            message: Some(render_memory_report()?),
        }),
        SlashCommand::Init => {
            Ok(ResumeCommandOutcome { session: session.clone(), message: Some(init_claude_md()?) })
        }
        SlashCommand::Diff => Ok(ResumeCommandOutcome {
            session: session.clone(),
            message: Some(render_diff_report_for(
                session_path.parent().unwrap_or_else(|| Path::new(".")),
            )?),
        }),
        SlashCommand::Version => Ok(ResumeCommandOutcome {
            session: session.clone(),
            message: Some(render_version_report()),
        }),
        SlashCommand::Export { path } => {
            let export_path = resolve_export_path(path.as_deref(), session)?;
            fs::write(&export_path, render_export_text(session))?;
            Ok(ResumeCommandOutcome {
                session: session.clone(),
                message: Some(format!(
                    "Export\n  Result           wrote transcript\n  File             {}\n  Messages         {}",
                    export_path.display(),
                    session.messages.len(),
                )),
            })
        }
        SlashCommand::RecoveryExport { path } => {
            let export_path = write_recovery_export(path.as_deref())?;
            Ok(ResumeCommandOutcome {
                session: session.clone(),
                message: Some(format!(
                    "RecoveryExport\n  Result           wrote recovery summary\n  File             {}",
                    export_path.display(),
                )),
            })
        }
        SlashCommand::Agents { args } => {
            let cwd = env::current_dir()?;
            Ok(ResumeCommandOutcome {
                session: session.clone(),
                message: Some(handle_agents_slash_command(args.as_deref(), &cwd)?),
            })
        }
        SlashCommand::Skills { args } => {
            let cwd = env::current_dir()?;
            Ok(ResumeCommandOutcome {
                session: session.clone(),
                message: Some(handle_skills_slash_command(args.as_deref(), &cwd)?),
            })
        }
        SlashCommand::Unknown(name) => Err(format_unknown_slash_command(name).into()),
        SlashCommand::Bughunter { .. }
        | SlashCommand::Commit { .. }
        | SlashCommand::Pr { .. }
        | SlashCommand::Issue { .. }
        | SlashCommand::Ultraplan { .. }
        | SlashCommand::Teleport { .. }
        | SlashCommand::DebugToolCall { .. }
        | SlashCommand::Resume { .. }
        | SlashCommand::Model { .. }
        | SlashCommand::Permissions { .. }
        | SlashCommand::Cd { .. }
        | SlashCommand::Workspace { path: Some(_) }
        | SlashCommand::Session { .. }
        | SlashCommand::Plugins { .. }
        | SlashCommand::Doctor
        | SlashCommand::Login
        | SlashCommand::Logout
        | SlashCommand::Vim
        | SlashCommand::Upgrade
        | SlashCommand::Stats
        | SlashCommand::Share
        | SlashCommand::Feedback
        | SlashCommand::Files
        | SlashCommand::Fast
        | SlashCommand::Exit
        | SlashCommand::Summary
        | SlashCommand::Desktop
        | SlashCommand::Brief
        | SlashCommand::Advisor
        | SlashCommand::Stickers
        | SlashCommand::Insights
        | SlashCommand::Thinkback
        | SlashCommand::ReleaseNotes
        | SlashCommand::SecurityReview
        | SlashCommand::Keybindings
        | SlashCommand::PrivacySettings
        | SlashCommand::Plan { .. }
        | SlashCommand::Review { .. }
        | SlashCommand::Verify { .. }
        | SlashCommand::Tasks { .. }
        | SlashCommand::Theme { .. }
        | SlashCommand::Voice { .. }
        | SlashCommand::Usage { .. }
        | SlashCommand::Rename { .. }
        | SlashCommand::Copy { .. }
        | SlashCommand::Hooks { .. }
        | SlashCommand::Context { .. }
        | SlashCommand::Color { .. }
        | SlashCommand::Effort { .. }
        | SlashCommand::Branch { .. }
        | SlashCommand::Rewind { .. }
        | SlashCommand::Ide { .. }
        | SlashCommand::Tag { .. }
        | SlashCommand::OutputStyle { .. }
        | SlashCommand::AddDir { .. } => Err("unsupported resumed slash command".into()),
    }
}

// The interactive loop: reading input, the slash-command surface, and the
// recovery bookkeeping around a turn are one state machine.
#[allow(clippy::too_many_lines)]
fn run_repl(
    model: String,
    allowed_tools: Option<AllowedToolSet>,
    permission_mode: PermissionMode,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut cli = LiveCli::new(model, true, allowed_tools, permission_mode)?;
    let mut editor =
        input::LineEditor::new("> ", cli.repl_completion_candidates().unwrap_or_default());
    println!("{}", cli.startup_banner());

    if let Ok(cwd) = env::current_dir() {
        if !is_git_worktree(&cwd) {
            eprintln!();
            eprintln!("Note: the current directory is not a Git project.");
            eprintln!("  Use /cd D:/YourProject to switch, or sego --cwd D:/YourProject");
            eprintln!();
        }
    }

    // A2 session 状态写入：进入 REPL 写 active。
    // 正常退出（/exit、/quit、ReadOutcome::Exit）写 graceful。
    // 错误路径（?返回 / 崩溃 / Ctrl+C）不写 graceful，保留 active，下次启动提示可恢复。
    persist_recovery_for_cli(
        runtime::recovery::RecoveryExitState::Active,
        cli.session_id(),
        cli.session_path(),
        Some(cli.model_name()),
        None,
    );
    // A REPL has no goal at launch beyond being a REPL; the first input that
    // reaches the model replaces this with the user's own words.
    begin_run_task(cli.session_id(), "repl");
    let mut goal_recorded = false;

    loop {
        editor.set_completions(cli.repl_completion_candidates().unwrap_or_default());
        match editor.read_line()? {
            input::ReadOutcome::Submit(input) => {
                let trimmed = input.trim().to_string();
                if trimmed.is_empty() {
                    continue;
                }
                if matches!(trimmed.as_str(), "/exit" | "/quit") {
                    cli.persist_session()?;
                    persist_recovery_for_cli(
                        runtime::recovery::RecoveryExitState::Graceful,
                        cli.session_id(),
                        cli.session_path(),
                        Some(cli.model_name()),
                        None,
                    );
                    complete_run_task();
                    break;
                }
                // C20.6-C R5: narrow REPL pre-check for combined commands containing
                // /review or sego review. This catches `/cd ... && /review staged`
                // and routes it through task_parser blocked guidance instead of
                // letting SlashCommand::parse error with the workspace-path message.
                //
                // R8-4: this pre-check is intentionally duplicated with the later
                // task-parser pass below. The pre-check exists ONLY so the combined
                // `/cd <path> && /review <scope>` case is intercepted before
                // SlashCommand::parse rejects `/cd` (which would otherwise produce
                // the historical "workspace path does not exist" error). The later
                // task-parser pass is the general entry point for ordinary
                // task-like input after slash parsing has had its chance. The gate
                // (`has_separator && has_review`) keeps the cost negligible for
                // normal REPL inputs.
                {
                    let has_separator = trimmed.contains("&&")
                        || trimmed.contains('|')
                        || trimmed.contains('>')
                        || trimmed.contains('<')
                        || trimmed.contains(';');
                    let has_review = trimmed.contains("/review") || trimmed.contains("sego review");
                    if has_separator && has_review {
                        match task_parser::parse_required_review_command(&trimmed) {
                            task_parser::RequiredReviewResult::Execute { scope } => {
                                cli.handle_review_command(Some(&scope))?;
                                continue;
                            }
                            task_parser::RequiredReviewResult::Blocked {
                                detected,
                                reason,
                                guidance,
                            } => {
                                println!(
                                    "Task command blocked\n  Detected         {detected}\n  Reason           {reason}\n  Guidance         {guidance}"
                                );
                                continue;
                            }
                            task_parser::RequiredReviewResult::None => {}
                        }
                    }
                }
                match SlashCommand::parse(&trimmed) {
                    Ok(Some(command)) => {
                        if cli.handle_repl_command(command)? {
                            cli.persist_session()?;
                        }
                        continue;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        eprintln!("{error}");
                        continue;
                    }
                }
                editor.push_history(input);
                // C20.6-C R2: check for required review commands in task-like input first.
                match task_parser::parse_required_review_command(&trimmed) {
                    task_parser::RequiredReviewResult::Execute { scope } => {
                        cli.handle_review_command(Some(&scope))?;
                        continue;
                    }
                    task_parser::RequiredReviewResult::Blocked { detected, reason, guidance } => {
                        println!(
                            "Task command blocked
  Detected         {detected}
  Reason           {reason}
  Guidance         {guidance}"
                        );
                        continue;
                    }
                    task_parser::RequiredReviewResult::None => {}
                }

                if let Some(intent) = parse_nl_intent(&trimmed) {
                    if cli.handle_nl_intent(intent)? {
                        cli.persist_session()?;
                        persist_recovery_for_cli(
                            runtime::recovery::RecoveryExitState::Graceful,
                            cli.session_id(),
                            cli.session_path(),
                            Some(cli.model_name()),
                            None,
                        );
                        break;
                    }
                    continue;
                }
                if let Some(miss) = classify_nl_intent_miss(&trimmed) {
                    println!("{}", render_nl_intent_miss(&miss));
                    continue;
                }
                if !goal_recorded {
                    // Recorded here rather than at read time: a slash command is
                    // not what the session is working on, so the goal is the first
                    // input that actually reaches the model.
                    note_run_goal(&trimmed);
                    goal_recorded = true;
                }
                match cli.run_turn(&trimmed) {
                    Ok(()) => {}
                    Err(error) if is_turn_cancelled_error(error.as_ref()) => {
                        eprintln!("Sego cancelled the current task. You can continue.");
                    }
                    Err(error) => return Err(error),
                }
            }
            input::ReadOutcome::Cancel => {}
            input::ReadOutcome::Exit => {
                cli.persist_session()?;
                persist_recovery_for_cli(
                    runtime::recovery::RecoveryExitState::Graceful,
                    cli.session_id(),
                    cli.session_path(),
                    Some(cli.model_name()),
                    None,
                );
                complete_run_task();
                break;
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct SessionHandle {
    id: String,
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct ManagedSessionSummary {
    id: String,
    path: PathBuf,
    modified_epoch_millis: u128,
    message_count: usize,
    parent_session_id: Option<String>,
    branch_name: Option<String>,
}

struct LiveCli {
    model: String,
    allowed_tools: Option<AllowedToolSet>,
    permission_mode: PermissionMode,
    /// When true, suppress all non-JSON stdout output (sidecar machine mode, D-IDE-1).
    machine_output: bool,
    system_prompt: Vec<String>,
    runtime: BuiltRuntime,
    session: SessionHandle,
    workflow: WorkflowSnapshot,
    workflow_store: WorkflowStore,
    community: CommunityLearning,
}

struct RuntimePluginState {
    feature_config: runtime::RuntimeFeatureConfig,
    tool_registry: GlobalToolRegistry,
    plugin_registry: PluginRegistry,
    mcp_state: Option<Arc<Mutex<RuntimeMcpState>>>,
}

struct BuiltRuntime {
    runtime: Option<ConversationRuntime<SegoRuntimeClient, CliToolExecutor>>,
    plugin_registry: PluginRegistry,
    plugins_active: bool,
    mcp_state: Option<Arc<Mutex<RuntimeMcpState>>>,
    mcp_active: bool,
}

impl BuiltRuntime {
    fn new(
        runtime: ConversationRuntime<SegoRuntimeClient, CliToolExecutor>,
        plugin_registry: PluginRegistry,
        mcp_state: Option<Arc<Mutex<RuntimeMcpState>>>,
    ) -> Self {
        Self {
            runtime: Some(runtime),
            plugin_registry,
            plugins_active: true,
            mcp_state,
            mcp_active: true,
        }
    }

    fn with_hook_abort_signal(mut self, hook_abort_signal: runtime::HookAbortSignal) -> Self {
        let runtime =
            self.runtime.take().expect("runtime should exist before installing hook abort signal");
        self.runtime = Some(runtime.with_hook_abort_signal(hook_abort_signal));
        self
    }

    fn with_turn_abort_signal(mut self, turn_abort_signal: runtime::TurnAbortSignal) -> Self {
        let runtime =
            self.runtime.take().expect("runtime should exist before installing turn abort signal");
        self.runtime = Some(runtime.with_turn_abort_signal(turn_abort_signal));
        self
    }

    fn shutdown_plugins(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.plugins_active {
            self.plugin_registry.shutdown()?;
            self.plugins_active = false;
        }
        Ok(())
    }

    fn shutdown_mcp(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        if self.mcp_active {
            if let Some(mcp_state) = &self.mcp_state {
                mcp_state.lock().unwrap_or_else(std::sync::PoisonError::into_inner).shutdown()?;
            }
            self.mcp_active = false;
        }
        Ok(())
    }
}

impl Deref for BuiltRuntime {
    type Target = ConversationRuntime<SegoRuntimeClient, CliToolExecutor>;

    fn deref(&self) -> &Self::Target {
        self.runtime.as_ref().expect("runtime should exist while built runtime is alive")
    }
}

impl DerefMut for BuiltRuntime {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.runtime.as_mut().expect("runtime should exist while built runtime is alive")
    }
}

impl Drop for BuiltRuntime {
    fn drop(&mut self) {
        let _ = self.shutdown_mcp();
        let _ = self.shutdown_plugins();
    }
}

#[derive(Debug, Deserialize)]
struct ToolSearchRequest {
    query: String,
    max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct McpToolRequest {
    #[serde(rename = "qualifiedName")]
    qualified_name: Option<String>,
    tool: Option<String>,
    arguments: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct ListMcpResourcesRequest {
    server: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ReadMcpResourceRequest {
    server: String,
    uri: String,
}

fn mcp_runtime_tool_definition(tool: &runtime::ManagedMcpTool) -> RuntimeToolDefinition {
    RuntimeToolDefinition {
        name: tool.qualified_name.clone(),
        description: Some(
            tool.tool
                .description
                .clone()
                .unwrap_or_else(|| format!("Invoke MCP tool `{}`.", tool.qualified_name)),
        ),
        input_schema: tool
            .tool
            .input_schema
            .clone()
            .unwrap_or_else(|| json!({ "type": "object", "additionalProperties": true })),
        required_permission: permission_mode_for_mcp_tool(&tool.tool),
    }
}

fn permission_mode_for_mcp_tool(tool: &McpTool) -> PermissionMode {
    let read_only = mcp_annotation_flag(tool, "readOnlyHint");
    let destructive = mcp_annotation_flag(tool, "destructiveHint");
    let open_world = mcp_annotation_flag(tool, "openWorldHint");

    if read_only && !destructive && !open_world {
        PermissionMode::ReadOnly
    } else if destructive || open_world {
        PermissionMode::DangerFullAccess
    } else {
        PermissionMode::WorkspaceWrite
    }
}

fn mcp_annotation_flag(tool: &McpTool, key: &str) -> bool {
    tool.annotations
        .as_ref()
        .and_then(|annotations| annotations.get(key))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

struct HookAbortMonitor {
    stop_tx: Option<Sender<()>>,
    join_handle: Option<JoinHandle<()>>,
}

impl HookAbortMonitor {
    fn spawn(
        hook_abort_signal: runtime::HookAbortSignal,
        turn_abort_signal: runtime::TurnAbortSignal,
        emit_output: bool,
    ) -> Self {
        Self::spawn_with_waiter(
            hook_abort_signal,
            turn_abort_signal,
            emit_output,
            move |stop_rx, hook_abort_signal, turn_abort_signal, emit_output| {
                let Ok(runtime) =
                    tokio::runtime::Builder::new_current_thread().enable_all().build()
                else {
                    return;
                };

                runtime.block_on(async move {
                let wait_for_stop = tokio::task::spawn_blocking(move || {
                    let _ = stop_rx.recv();
                });
                tokio::pin!(wait_for_stop);
                let mut cancelled_once = false;

                loop {
                    tokio::select! {
                        _ = &mut wait_for_stop => break,
                        result = tokio::signal::ctrl_c() => {
                            if result.is_err() {
                                break;
                            }
                            if cancelled_once {
                                if emit_output {
                                    eprintln!("Sego force exiting.");
                                }
                                std::process::exit(130);
                            }
                            cancelled_once = true;
                            hook_abort_signal.abort();
                            turn_abort_signal.abort();
                            if emit_output {
                                eprintln!();
                                eprintln!("Sego cancelled the current task. Press Ctrl+C again to force exit.");
                            }
                        }
                    }
                }
            });
            },
        )
    }

    fn spawn_with_waiter<F>(
        abort_signal: runtime::HookAbortSignal,
        turn_abort_signal: runtime::TurnAbortSignal,
        emit_output: bool,
        wait_for_interrupt: F,
    ) -> Self
    where
        F: FnOnce(Receiver<()>, runtime::HookAbortSignal, runtime::TurnAbortSignal, bool)
            + Send
            + 'static,
    {
        let (stop_tx, stop_rx) = mpsc::channel();
        let join_handle = thread::spawn(move || {
            wait_for_interrupt(stop_rx, abort_signal, turn_abort_signal, emit_output);
        });

        Self { stop_tx: Some(stop_tx), join_handle: Some(join_handle) }
    }

    fn stop(mut self) {
        if let Some(stop_tx) = self.stop_tx.take() {
            let _ = stop_tx.send(());
        }
        if let Some(join_handle) = self.join_handle.take() {
            let _ = join_handle.join();
        }
    }
}

impl LiveCli {
    fn new(
        model: String,
        enable_tools: bool,
        allowed_tools: Option<AllowedToolSet>,
        permission_mode: PermissionMode,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let system_prompt = build_system_prompt()?;
        let session_state = Session::new();
        let session = create_managed_session_handle(&session_state.session_id)?;
        let runtime = build_runtime(
            session_state.with_persistence_path(session.path.clone()),
            &session.id,
            model.clone(),
            system_prompt.clone(),
            enable_tools,
            true,
            allowed_tools.clone(),
            permission_mode,
            None,
        )?;
        let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let store = WorkflowStore::new(&cwd);
        let mut workflow = WorkflowSnapshot::new(session.id.clone());
        workflow.started_at = Some(default_date());
        workflow.record_event(LaneEvent::started(default_date()));

        let community = CommunityLearning::new(&cwd);

        let mut cli = Self {
            model,
            allowed_tools,
            permission_mode,
            machine_output: false,
            system_prompt,
            runtime,
            session,
            workflow,
            workflow_store: store,
            community,
        };
        cli.persist_session()?;
        Ok(cli)
    }

    /// 当前 session id（用于 recovery state 写入）。
    fn session_id(&self) -> &str {
        &self.session.id
    }

    /// 当前 session 文件路径（用于 recovery state 写入）。
    fn session_path(&self) -> &Path {
        &self.session.path
    }

    /// 当前模型名（用于 recovery state 写入）。
    fn model_name(&self) -> &str {
        &self.model
    }

    /// Enable machine output mode: suppress all non-JSON stdout (sidecar, D-IDE-1).
    fn with_machine_output(mut self) -> Self {
        self.machine_output = true;
        self
    }

    fn print_workspace_status() -> Result<(), Box<dyn std::error::Error>> {
        println!("{}", format_workspace_report(&workspace_context()?));
        Ok(())
    }

    fn switch_workspace(
        &mut self,
        requested_path: &str,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let previous = env::current_dir()?;
        let next = resolve_cli_cwd(requested_path)?;
        // `resolve_cli_cwd` canonicalizes, and on Windows that yields a verbatim
        // (`\\?\`) path while `current_dir` does not - the two are never equal as
        // text even when they name the same directory. Compare the canonical form
        // of both sides, or re-selecting the current workspace tears the session
        // down and starts a fresh one.
        if next == previous.canonicalize().unwrap_or_else(|_| previous.clone()) {
            println!("{}", format_workspace_switch_report(&previous, &next, false));
            return Ok(false);
        }

        self.persist_session()?;
        env::set_current_dir(&next)?;

        let system_prompt = build_system_prompt()?;
        let session_state = Session::new();
        let session = create_managed_session_handle(&session_state.session_id)?;
        let runtime = build_runtime(
            session_state.with_persistence_path(session.path.clone()),
            &session.id,
            self.model.clone(),
            system_prompt.clone(),
            true,
            true,
            self.allowed_tools.clone(),
            self.permission_mode,
            None,
        )?;
        let store = WorkflowStore::new(&next);
        let mut workflow = WorkflowSnapshot::new(session.id.clone());
        workflow.started_at = Some(default_date());
        workflow.record_event(LaneEvent::started(default_date()));

        self.system_prompt = system_prompt;
        self.runtime = runtime;
        self.session = session;
        self.workflow = workflow;
        self.workflow_store = store;
        self.community = CommunityLearning::new(&next);
        self.persist_session()?;

        println!("{}", format_workspace_switch_report(&previous, &next, true));
        Ok(true)
    }

    fn startup_banner(&self) -> String {
        let cwd = env::current_dir()
            .map_or_else(|_| "<unknown>".to_string(), |path| path.display().to_string());
        let status = status_context(None).ok();
        let git_branch =
            status.as_ref().and_then(|context| context.git_branch.as_deref()).unwrap_or("unknown");
        let workspace = status
            .as_ref()
            .map_or_else(|| "unknown".to_string(), |context| context.git_summary.headline());
        let session_path = self.session.path.strip_prefix(Path::new(&cwd)).map_or_else(
            |_| self.session.path.display().to_string(),
            |path| path.display().to_string(),
        );
        format!(
            "\x1b[38;5;51m\
███████╗███████╗ ██████╗  ██████╗ \n\
██╔════╝██╔════╝██╔════╝ ██╔═══██╗\n\
███████╗█████╗  ██║  ███╗██║   ██║\n\
╚════██║██╔══╝  ██║   ██║██║   ██║\n\
███████║███████╗╚██████╔╝╚██████╔╝\n\
╚══════╝╚══════╝ ╚═════╝  ╚═════╝\x1b[0m \x1b[38;5;51mAgent\x1b[0m 🤖\n\n\
  \x1b[2mModel\x1b[0m            {}\n\
  \x1b[2mPermissions\x1b[0m      {}\n\
  \x1b[2mBranch\x1b[0m           {}\n\
  \x1b[2mWorkspace\x1b[0m        {}\n\
  \x1b[2mDirectory\x1b[0m        {}\n\
  \x1b[2mSession\x1b[0m          {}\n\
  \x1b[2mAuto-save\x1b[0m        {}\n\n\
  Type \x1b[1m/help\x1b[0m for commands · \x1b[1m/status\x1b[0m for live context · \x1b[2m/resume latest\x1b[0m jumps back to the newest session · \x1b[1m/diff\x1b[0m then \x1b[1m/commit\x1b[0m to ship · \x1b[2mTab\x1b[0m for workflow completions · \x1b[2mShift+Enter\x1b[0m for newline",
            self.model,
            self.permission_mode.as_str(),
            git_branch,
            workspace,
            cwd,
            self.session.id,
            session_path,
        )
    }

    fn repl_completion_candidates(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        Ok(slash_command_completion_candidates_with_sessions(
            &self.model,
            Some(&self.session.id),
            list_managed_sessions()?.into_iter().map(|session| session.id).collect(),
        ))
    }

    fn prepare_turn_runtime(
        &self,
        emit_output: bool,
    ) -> Result<(BuiltRuntime, HookAbortMonitor), Box<dyn std::error::Error>> {
        let hook_abort_signal = runtime::HookAbortSignal::new();
        let turn_abort_signal = runtime::TurnAbortSignal::new();
        let runtime = build_runtime(
            self.runtime.session().clone(),
            &self.session.id,
            self.model.clone(),
            self.system_prompt.clone(),
            true,
            emit_output,
            self.allowed_tools.clone(),
            self.permission_mode,
            None,
        )?
        .with_hook_abort_signal(hook_abort_signal.clone())
        .with_turn_abort_signal(turn_abort_signal.clone());
        let hook_abort_monitor =
            HookAbortMonitor::spawn(hook_abort_signal, turn_abort_signal, emit_output);

        Ok((runtime, hook_abort_monitor))
    }

    fn replace_runtime(&mut self, runtime: BuiltRuntime) -> Result<(), Box<dyn std::error::Error>> {
        self.runtime.shutdown_plugins()?;
        self.runtime = runtime;
        Ok(())
    }

    fn run_turn(&mut self, input: &str) -> Result<(), Box<dyn std::error::Error>> {
        let (mut runtime, hook_abort_monitor) = self.prepare_turn_runtime(true)?;
        let machine = self.machine_output;
        let mut stdout = io::stdout();
        let mut stderr = io::stderr();
        let ui_target: &mut dyn std::io::Write = if machine { &mut stderr } else { &mut stdout };
        let mut ui = ProgressUI::new("Sego Agent", ui_target);
        ui.add_phase("thinking", "Thinking");
        ui.add_phase("exec", "Executing");
        let _ = ui.start();
        let thinking_phase = 0usize;
        let exec_phase = 1usize;
        ui.phase_idx(thinking_phase).start();
        let mut permission_prompter = CliPermissionPrompter::new(self.permission_mode);
        let result = runtime.run_turn(input, Some(&mut permission_prompter));
        ui.phase_idx(exec_phase).complete("Done", "");
        hook_abort_monitor.stop();
        match result {
            Ok(summary) => {
                self.replace_runtime(runtime)?;
                ui.finish("Done")?;
                if !machine {
                    println!();
                    if let Some(event) = summary.auto_compaction {
                        println!("{}", format_auto_compaction_notice(event.removed_message_count));
                    }
                }
                // Record workflow: successful turn
                self.workflow.record_event(LaneEvent::new(
                    LaneEventName::Green,
                    LaneEventStatus::Green,
                    default_date(),
                ));
                self.persist_session()?;
                Ok(())
            }
            Err(error) => {
                runtime.shutdown_plugins()?;
                // Record workflow: failure
                self.workflow.record_event(LaneEvent::blocked(
                    default_date(),
                    &LaneEventBlocker {
                        failure_class: LaneFailureClass::ToolRuntime,
                        detail: error.to_string(),
                    },
                ));
                ui.phase_idx(thinking_phase).fail("Request failed", error.to_string());
                let _ = ui.finish("Failed");
                Err(Box::new(error))
            }
        }
    }

    fn run_turn_capture_text(
        &mut self,
        input: &str,
        emit_output: bool,
    ) -> Result<String, Box<dyn std::error::Error>> {
        self.run_turn_capture_observed(input, emit_output).map(|(text, _)| text)
    }

    /// Same as [`Self::run_turn_capture_text`], and additionally returns what the
    /// turn observed about itself, so the caller can record it in a review
    /// artifact.
    ///
    /// `SEG-ADR-004` requires the egress and compute values to come from the run
    /// rather than from configuration. The run is exactly what this returns, and
    /// the only place the numbers exist.
    fn run_turn_capture_observed(
        &mut self,
        input: &str,
        emit_output: bool,
    ) -> Result<(String, runtime::code_review::ReviewEgressObservation), Box<dyn std::error::Error>>
    {
        let machine = self.machine_output;
        // C-light (D-IDE-1): machine mode uses emit_output=false to suppress
        // conversation runtime rendering, and a fail-closed prompter that never
        // writes to stdout or reads from stdin.
        let (mut runtime, hook_abort_monitor) = self.prepare_turn_runtime(emit_output)?;
        if machine {
            // Machine mode: no ProgressUI, no stdout output, fail-closed permissions.
            let mut prompter = MachinePermissionPrompter;
            let result = runtime.run_turn(input, Some(&mut prompter));
            hook_abort_monitor.stop();
            match result {
                Ok(summary) => {
                    let text = final_assistant_text(&summary);
                    let observation = observed_turn(&summary);
                    self.replace_runtime(runtime)?;
                    self.persist_session()?;
                    Ok((text, observation))
                }
                Err(error) => {
                    runtime.shutdown_plugins()?;
                    Err(error.into())
                }
            }
        } else {
            // Interactive mode: full ProgressUI + CliPermissionPrompter.
            let mut stdout = io::stdout();
            let mut ui = ProgressUI::new("Sego Agent", &mut stdout);
            ui.add_phase("thinking", "Thinking");
            ui.add_phase("exec", "Executing");
            let _ = ui.start();
            let thinking_phase = 0usize;
            let exec_phase = 1usize;
            ui.phase_idx(thinking_phase).start();
            let mut permission_prompter = CliPermissionPrompter::new(self.permission_mode);
            let result = runtime.run_turn(input, Some(&mut permission_prompter));
            ui.phase_idx(exec_phase).complete("Done", "");
            hook_abort_monitor.stop();
            match result {
                Ok(summary) => {
                    let text = final_assistant_text(&summary);
                    let observation = observed_turn(&summary);
                    self.replace_runtime(runtime)?;
                    ui.finish("Done")?;
                    println!();
                    if let Some(event) = summary.auto_compaction {
                        println!("{}", format_auto_compaction_notice(event.removed_message_count));
                    }
                    self.workflow.record_event(LaneEvent::new(
                        LaneEventName::Green,
                        LaneEventStatus::Green,
                        default_date(),
                    ));
                    self.persist_session()?;
                    Ok((text, observation))
                }
                Err(error) => {
                    runtime.shutdown_plugins()?;
                    self.workflow.record_event(LaneEvent::blocked(
                        default_date(),
                        &LaneEventBlocker {
                            failure_class: LaneFailureClass::ToolRuntime,
                            detail: error.to_string(),
                        },
                    ));
                    ui.phase_idx(thinking_phase).fail("Request failed", error.to_string());
                    let _ = ui.finish("Failed");
                    Err(error.into())
                }
            }
        }
    }
    fn run_turn_with_output(
        &mut self,
        input: &str,
        output_format: CliOutputFormat,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match output_format {
            CliOutputFormat::Text => self.run_turn(input),
            CliOutputFormat::Json => self.run_prompt_json(input),
        }
    }

    fn run_prompt_json(&mut self, input: &str) -> Result<(), Box<dyn std::error::Error>> {
        let (mut runtime, hook_abort_monitor) = self.prepare_turn_runtime(false)?;
        let mut permission_prompter = CliPermissionPrompter::new(self.permission_mode);
        let result = runtime.run_turn(input, Some(&mut permission_prompter));
        hook_abort_monitor.stop();
        let summary = result?;
        self.replace_runtime(runtime)?;
        self.persist_session()?;
        println!(
            "{}",
            json!({
                "message": final_assistant_text(&summary),
                "model": self.model,
                "iterations": summary.iterations,
                "auto_compaction": summary.auto_compaction.map(|event| json!({
                    "removed_messages": event.removed_message_count,
                    "notice": format_auto_compaction_notice(event.removed_message_count),
                })),
                "tool_uses": collect_tool_uses(&summary),
                "tool_results": collect_tool_results(&summary),
                "prompt_cache_events": collect_prompt_cache_events(&summary),
                "usage": {
                    "input_tokens": summary.usage.input_tokens,
                    "output_tokens": summary.usage.output_tokens,
                    "cache_creation_input_tokens": summary.usage.cache_creation_input_tokens,
                    "cache_read_input_tokens": summary.usage.cache_read_input_tokens,
                },
                "estimated_cost": format_usd(
                    summary.usage.estimate_cost_usd_with_pricing(
                        pricing_for_model(&self.model)
                            .unwrap_or_else(runtime::ModelPricing::default_sonnet_tier)
                    ).total_cost_usd()
                )
            })
        );
        Ok(())
    }

    fn handle_nl_intent(&mut self, intent: NlIntent) -> Result<bool, Box<dyn std::error::Error>> {
        match intent {
            NlIntent::WorkspaceShow => {
                Self::print_workspace_status()?;
                Ok(false)
            }
            NlIntent::WorkspaceSwitch { path } => {
                self.switch_workspace(&path)?;
                Ok(false)
            }
            NlIntent::Review { scope } => {
                self.handle_review_command(scope.as_deref())?;
                Ok(false)
            }
            NlIntent::ReviewSafety { staged } => {
                let scope =
                    if staged { SafetyReviewScope::Staged } else { SafetyReviewScope::Workspace };
                print_code_review_safety(scope)?;
                Ok(false)
            }
            NlIntent::ExportLastResponse { path } => {
                match self.export_last_assistant_response(path.as_deref()) {
                    Ok(()) => {}
                    Err(error) if is_no_assistant_response_export_error(error.as_ref()) => {
                        // The failed export already printed a recovery hint. Keep REPL usable.
                    }
                    Err(error) => return Err(error),
                }
                Ok(false)
            }
            NlIntent::ExportSession { path } => {
                self.export_session(path.as_deref())?;
                Ok(false)
            }
            NlIntent::UpdateCheck => {
                run_update(true)?;
                Ok(false)
            }
            NlIntent::UpdateApply => {
                run_update(false)?;
                Ok(false)
            }
            NlIntent::Exit => Ok(true),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn handle_repl_command(
        &mut self,
        command: SlashCommand,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        Ok(match command {
            SlashCommand::Help => {
                println!("{}", render_repl_help());
                false
            }
            SlashCommand::Dir => {
                println!("{}", render_natural_language_directory());
                false
            }
            SlashCommand::Status => {
                self.print_status();
                false
            }
            SlashCommand::Bughunter { scope } => {
                self.run_bughunter(scope.as_deref())?;
                false
            }
            SlashCommand::Commit => {
                self.run_commit(None)?;
                false
            }
            SlashCommand::Pr { context } => {
                self.run_pr(context.as_deref())?;
                false
            }
            SlashCommand::Issue { context } => {
                self.run_issue(context.as_deref())?;
                false
            }
            SlashCommand::Ultraplan { task } => {
                self.run_ultraplan(task.as_deref())?;
                false
            }
            SlashCommand::Teleport { target } => {
                Self::run_teleport(target.as_deref())?;
                false
            }
            SlashCommand::DebugToolCall => {
                self.run_debug_tool_call(None)?;
                false
            }
            SlashCommand::Sandbox => {
                Self::print_sandbox_status();
                false
            }
            SlashCommand::Pwd | SlashCommand::Workspace { path: None } => {
                Self::print_workspace_status()?;
                false
            }
            SlashCommand::Workspace { path: Some(path) } => self.switch_workspace(&path)?,
            SlashCommand::Cd { path } => {
                let Some(path) = path else {
                    println!("Usage: /cd <path>");
                    return Ok(false);
                };
                self.switch_workspace(&path)?
            }
            SlashCommand::Compact => {
                self.compact()?;
                false
            }
            SlashCommand::Model { model } => self.set_model(model)?,
            SlashCommand::Permissions { mode } => self.set_permissions(mode)?,
            SlashCommand::Clear { confirm } => self.clear_session(confirm)?,
            SlashCommand::Cost => {
                self.print_cost();
                false
            }
            SlashCommand::Resume { session_path } => self.resume_session(session_path)?,
            SlashCommand::Config { section } => {
                Self::print_config(section.as_deref())?;
                false
            }
            SlashCommand::Mcp { action, target } => {
                let args = match (action.as_deref(), target.as_deref()) {
                    (None, None) => None,
                    (Some(action), None) => Some(action.to_string()),
                    (Some(action), Some(target)) => Some(format!("{action} {target}")),
                    (None, Some(target)) => Some(target.to_string()),
                };
                Self::print_mcp(args.as_deref())?;
                false
            }
            SlashCommand::Memory => {
                Self::print_memory()?;
                false
            }
            SlashCommand::Init => {
                run_init()?;
                false
            }
            SlashCommand::Diff => {
                Self::print_diff()?;
                false
            }
            SlashCommand::Version => {
                Self::print_version();
                false
            }
            SlashCommand::Export { path } => {
                self.export_session(path.as_deref())?;
                false
            }
            SlashCommand::RecoveryExport { path } => {
                let export_path = write_recovery_export(path.as_deref())?;
                println!(
                    "RecoveryExport\n  Result           wrote recovery summary\n  File             {}",
                    export_path.display(),
                );
                false
            }
            SlashCommand::Session { action, target } => {
                self.handle_session_command(action.as_deref(), target.as_deref())?
            }
            SlashCommand::Plugins { action, target } => {
                self.handle_plugins_command(action.as_deref(), target.as_deref())?
            }
            SlashCommand::Agents { args } => {
                Self::print_agents(args.as_deref())?;
                false
            }
            SlashCommand::Skills { args } => {
                Self::print_skills(args.as_deref())?;
                false
            }
            SlashCommand::Doctor => {
                print_doctor(CliOutputFormat::Text)?;
                false
            }
            SlashCommand::Login
            | SlashCommand::Logout
            | SlashCommand::Vim
            | SlashCommand::Upgrade
            | SlashCommand::Stats
            | SlashCommand::Share
            | SlashCommand::Feedback
            | SlashCommand::Files
            | SlashCommand::Fast
            | SlashCommand::Exit
            | SlashCommand::Summary
            | SlashCommand::Desktop
            | SlashCommand::Brief
            | SlashCommand::Advisor
            | SlashCommand::Stickers
            | SlashCommand::Insights
            | SlashCommand::Thinkback
            | SlashCommand::ReleaseNotes
            | SlashCommand::SecurityReview
            | SlashCommand::Keybindings
            | SlashCommand::PrivacySettings
            | SlashCommand::Plan { .. }
            | SlashCommand::Tasks { .. }
            | SlashCommand::Theme { .. }
            | SlashCommand::Voice { .. }
            | SlashCommand::Usage { .. }
            | SlashCommand::Rename { .. }
            | SlashCommand::Copy { .. }
            | SlashCommand::Hooks { .. }
            | SlashCommand::Context { .. }
            | SlashCommand::Color { .. }
            | SlashCommand::Effort { .. }
            | SlashCommand::Branch { .. }
            | SlashCommand::Rewind { .. }
            | SlashCommand::Ide { .. }
            | SlashCommand::Tag { .. }
            | SlashCommand::OutputStyle { .. }
            | SlashCommand::AddDir { .. } => {
                eprintln!("Command registered but not yet implemented.");
                false
            }
            SlashCommand::Review { scope } => {
                self.handle_review_command(scope.as_deref())?;
                true
            }
            SlashCommand::Verify { scope } => {
                run_code_verify_cli(scope.as_deref())?;
                true
            }
            SlashCommand::Unknown(name) => {
                eprintln!("{}", format_unknown_slash_command(&name));
                false
            }
        })
    }

    fn persist_session(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.runtime.session().save_to_path(&self.session.path)?;

        // Evaluate Green Contract based on session performance
        // Evaluate Green Contract based on session performance
        let observed_level = if self.workflow.failure_count == 0 {
            GreenLevel::Workspace
        } else if self.workflow.failure_count <= 2 {
            GreenLevel::Package
        } else {
            GreenLevel::TargetedTests
        };
        self.workflow.green_level = Some(observed_level);

        // Run policy engine evaluation
        let lane_ctx = LaneContext::new(
            self.session.id.clone(),
            observed_level as u8,
            std::time::Duration::from_secs(0), // branch freshness
            LaneBlocker::None,
            ReviewStatus::Pending,
            DiffScope::Scoped,
            true, // completed
        );
        let engine = runtime::PolicyEngine::new(Vec::new());
        let _ = evaluate(&engine, &lane_ctx);

        // Record workflow: session snapshot updated
        self.workflow.finished_at = Some(default_date());
        self.workflow.compute_efficiency();
        let _ = self.workflow_store.save_session(&self.workflow);

        // Generate and update trends on close
        let report = SessionReport::from_snapshot(
            &self.workflow,
            self.workflow_store.load_trends().ok().map(|t| t.average_efficiency),
        );
        let _ = self.workflow_store.update_trends(&report, 0);

        // Community learning: anonymous telemetry (opt-in only)
        if self.community.is_enabled() {
            let base_url = std::env::var("ANTHROPIC_BASE_URL").unwrap_or_default();
            let _ = self.community.collect_report(&self.workflow, &self.model, &base_url);
        }
        Ok(())
    }

    fn print_status(&self) {
        let cumulative = self.runtime.usage().cumulative_usage();
        let latest = self.runtime.usage().current_turn_usage();
        println!(
            "{}",
            format_status_report(
                &self.model,
                StatusUsage {
                    message_count: self.runtime.session().messages.len(),
                    turns: self.runtime.usage().turns(),
                    latest,
                    cumulative,
                    estimated_tokens: self.runtime.estimated_tokens(),
                },
                self.permission_mode.as_str(),
                &status_context(Some(&self.session.path)).expect("status context should load"),
            )
        );
    }

    fn print_sandbox_status() {
        let cwd = env::current_dir().expect("current dir");
        let loader = ConfigLoader::default_for(&cwd);
        let runtime_config = loader.load().unwrap_or_else(|_| runtime::RuntimeConfig::empty());
        println!(
            "{}",
            format_sandbox_report(&resolve_sandbox_status(runtime_config.sandbox(), &cwd))
        );
    }

    fn set_model(&mut self, model: Option<String>) -> Result<bool, Box<dyn std::error::Error>> {
        let Some(model) = model else {
            println!(
                "{}",
                format_model_report(
                    &self.model,
                    self.runtime.session().messages.len(),
                    self.runtime.usage().turns(),
                )
            );
            return Ok(false);
        };

        let model = resolve_model_alias(&model).clone();

        if model == self.model {
            println!(
                "{}",
                format_model_report(
                    &self.model,
                    self.runtime.session().messages.len(),
                    self.runtime.usage().turns(),
                )
            );
            return Ok(false);
        }

        let previous = self.model.clone();
        let session = self.runtime.session().clone();
        let message_count = session.messages.len();
        let runtime = build_runtime(
            session,
            &self.session.id,
            model.clone(),
            self.system_prompt.clone(),
            true,
            true,
            self.allowed_tools.clone(),
            self.permission_mode,
            None,
        )?;
        self.replace_runtime(runtime)?;
        self.model.clone_from(&model);
        println!("{}", format_model_switch_report(&previous, &model, message_count));
        Ok(true)
    }

    fn set_permissions(
        &mut self,
        mode: Option<String>,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let Some(mode) = mode else {
            println!("{}", format_permissions_report(self.permission_mode.as_str()));
            return Ok(false);
        };

        let normalized = normalize_permission_mode(&mode).ok_or_else(|| {
            format!(
                "unsupported permission mode '{mode}'. Use read-only, workspace-write, or danger-full-access."
            )
        })?;

        if normalized == self.permission_mode.as_str() {
            println!("{}", format_permissions_report(normalized));
            return Ok(false);
        }

        let previous = self.permission_mode.as_str().to_string();
        let session = self.runtime.session().clone();
        self.permission_mode = permission_mode_from_label(normalized);
        let runtime = build_runtime(
            session,
            &self.session.id,
            self.model.clone(),
            self.system_prompt.clone(),
            true,
            true,
            self.allowed_tools.clone(),
            self.permission_mode,
            None,
        )?;
        self.replace_runtime(runtime)?;
        println!("{}", format_permissions_switch_report(&previous, normalized));
        Ok(true)
    }

    fn clear_session(&mut self, confirm: bool) -> Result<bool, Box<dyn std::error::Error>> {
        if !confirm {
            println!(
                "clear: confirmation required; run /clear --confirm to start a fresh session."
            );
            return Ok(false);
        }

        let previous_session = self.session.clone();
        let session_state = Session::new();
        self.session = create_managed_session_handle(&session_state.session_id)?;
        let runtime = build_runtime(
            session_state.with_persistence_path(self.session.path.clone()),
            &self.session.id,
            self.model.clone(),
            self.system_prompt.clone(),
            true,
            true,
            self.allowed_tools.clone(),
            self.permission_mode,
            None,
        )?;
        self.replace_runtime(runtime)?;
        println!(
            "Session cleared\n  Mode             fresh session\n  Previous session {}\n  Resume previous  /resume {}\n  Preserved model  {}\n  Permission mode  {}\n  New session      {}\n  Session file     {}",
            previous_session.id,
            previous_session.id,
            self.model,
            self.permission_mode.as_str(),
            self.session.id,
            self.session.path.display(),
        );
        Ok(true)
    }

    fn print_cost(&self) {
        let cumulative = self.runtime.usage().cumulative_usage();
        println!("{}", format_cost_report(cumulative));
    }

    fn resume_session(
        &mut self,
        session_path: Option<String>,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let Some(session_ref) = session_path else {
            println!("{}", render_resume_usage());
            return Ok(false);
        };

        let handle = resolve_session_reference(&session_ref)?;
        let session = Session::load_from_path(&handle.path)?;
        let message_count = session.messages.len();
        let session_id = session.session_id.clone();
        let runtime = build_runtime(
            session,
            &handle.id,
            self.model.clone(),
            self.system_prompt.clone(),
            true,
            true,
            self.allowed_tools.clone(),
            self.permission_mode,
            None,
        )?;
        self.replace_runtime(runtime)?;
        self.session = SessionHandle { id: session_id, path: handle.path };
        println!(
            "{}",
            format_resume_report(
                &self.session.path.display().to_string(),
                message_count,
                self.runtime.usage().turns(),
            )
        );
        Ok(true)
    }

    fn print_config(section: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        println!("{}", render_config_report(section)?);
        Ok(())
    }

    fn print_memory() -> Result<(), Box<dyn std::error::Error>> {
        println!("{}", render_memory_report()?);
        Ok(())
    }

    fn print_agents(args: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        let cwd = env::current_dir()?;
        println!("{}", handle_agents_slash_command(args, &cwd)?);
        Ok(())
    }

    fn print_mcp(args: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        let cwd = env::current_dir()?;
        println!("{}", handle_mcp_slash_command(args, &cwd)?);
        Ok(())
    }

    fn print_skills(args: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        let cwd = env::current_dir()?;
        println!("{}", handle_skills_slash_command(args, &cwd)?);
        Ok(())
    }

    fn print_diff() -> Result<(), Box<dyn std::error::Error>> {
        println!("{}", render_diff_report()?);
        Ok(())
    }

    fn print_version() {
        println!("{}", render_version_report());
    }

    fn export_session(
        &self,
        requested_path: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let export_path = resolve_export_path(requested_path, self.runtime.session())?;
        fs::write(&export_path, render_export_text(self.runtime.session()))?;
        println!(
            "Export\n  Result           wrote transcript\n  File             {}\n  Messages         {}",
            export_path.display(),
            self.runtime.session().messages.len(),
        );
        Ok(())
    }

    fn export_last_assistant_response(
        &self,
        requested_path: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let Some(text) = latest_assistant_text(self.runtime.session()) else {
            // C20.5-B: structured recovery hint for missing assistant response.
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let target = requested_path.unwrap_or("(default)");
            eprintln!(
                "Export\n  Result           failed\n  Reason           {NO_ASSISTANT_RESPONSE_EXPORT_REASON}\n  Target           {target}\n  Detail           target path was not written because there is no assistant response yet\n  Workspace        {}\n  Next step        Run a prompt first, or use /dir to see export commands.",
                cwd.display()
            );
            return Err(Box::new(NoAssistantResponseExportError));
        };
        let export_path = resolve_direct_export_path(requested_path, || {
            format!("sego-response-{}.md", default_date().replace('-', ""))
        })?;
        fs::write(&export_path, &text)?;
        let bytes = fs::metadata(&export_path).map_or(text.len() as u64, |metadata| metadata.len());
        println!(
            "Export\n  Result           wrote latest assistant response\n  Kind             markdown\n  Bytes            {bytes}\n  File             {}",
            export_path.display()
        );
        Ok(())
    }

    fn handle_session_command(
        &mut self,
        action: Option<&str>,
        target: Option<&str>,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        match action {
            None | Some("list") => {
                println!("{}", render_session_list(&self.session.id)?);
                Ok(false)
            }
            Some("switch") => {
                let Some(target) = target else {
                    println!("Usage: /session switch <session-id>");
                    return Ok(false);
                };
                let handle = resolve_session_reference(target)?;
                let session = Session::load_from_path(&handle.path)?;
                let message_count = session.messages.len();
                let session_id = session.session_id.clone();
                let runtime = build_runtime(
                    session,
                    &handle.id,
                    self.model.clone(),
                    self.system_prompt.clone(),
                    true,
                    true,
                    self.allowed_tools.clone(),
                    self.permission_mode,
                    None,
                )?;
                self.replace_runtime(runtime)?;
                self.session = SessionHandle { id: session_id, path: handle.path };
                println!(
                    "Session switched\n  Active session   {}\n  File             {}\n  Messages         {}",
                    self.session.id,
                    self.session.path.display(),
                    message_count,
                );
                Ok(true)
            }
            Some("fork") => {
                let forked = self.runtime.fork_session(target.map(ToOwned::to_owned));
                let parent_session_id = self.session.id.clone();
                let handle = create_managed_session_handle(&forked.session_id)?;
                let branch_name = forked.fork.as_ref().and_then(|fork| fork.branch_name.clone());
                let forked = forked.with_persistence_path(handle.path.clone());
                let message_count = forked.messages.len();
                forked.save_to_path(&handle.path)?;
                let runtime = build_runtime(
                    forked,
                    &handle.id,
                    self.model.clone(),
                    self.system_prompt.clone(),
                    true,
                    true,
                    self.allowed_tools.clone(),
                    self.permission_mode,
                    None,
                )?;
                self.replace_runtime(runtime)?;
                self.session = handle;
                println!(
                    "Session forked\n  Parent session   {}\n  Active session   {}\n  Branch           {}\n  File             {}\n  Messages         {}",
                    parent_session_id,
                    self.session.id,
                    branch_name.as_deref().unwrap_or("(unnamed)"),
                    self.session.path.display(),
                    message_count,
                );
                Ok(true)
            }
            Some(other) => {
                println!(
                    "Unknown /session action '{other}'. Use /session list, /session switch <session-id>, or /session fork [branch-name]."
                );
                Ok(false)
            }
        }
    }

    fn handle_plugins_command(
        &mut self,
        action: Option<&str>,
        target: Option<&str>,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let cwd = env::current_dir()?;
        let loader = ConfigLoader::default_for(&cwd);
        let runtime_config = loader.load()?;
        let mut manager = build_plugin_manager(&cwd, &loader, &runtime_config);
        let result = handle_plugins_slash_command(action, target, &mut manager)?;
        println!("{}", result.message);
        if result.reload_runtime {
            self.reload_runtime_features()?;
        }
        Ok(false)
    }

    fn reload_runtime_features(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let runtime = build_runtime(
            self.runtime.session().clone(),
            &self.session.id,
            self.model.clone(),
            self.system_prompt.clone(),
            true,
            true,
            self.allowed_tools.clone(),
            self.permission_mode,
            None,
        )?;
        self.replace_runtime(runtime)?;
        self.persist_session()
    }

    fn compact(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let result = self.runtime.compact(CompactionConfig::default());
        let removed = result.removed_message_count;
        let kept = result.compacted_session.messages.len();
        let skipped = removed == 0;
        let runtime = build_runtime(
            result.compacted_session,
            &self.session.id,
            self.model.clone(),
            self.system_prompt.clone(),
            true,
            true,
            self.allowed_tools.clone(),
            self.permission_mode,
            None,
        )?;
        self.replace_runtime(runtime)?;
        self.persist_session()?;
        println!("{}", format_compact_report(removed, kept, skipped));
        Ok(())
    }

    fn run_internal_prompt_text_with_progress(
        &self,
        prompt: &str,
        enable_tools: bool,
        progress: Option<InternalPromptProgressReporter>,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let session = self.runtime.session().clone();
        let mut runtime = build_runtime(
            session,
            &self.session.id,
            self.model.clone(),
            self.system_prompt.clone(),
            enable_tools,
            false,
            self.allowed_tools.clone(),
            self.permission_mode,
            progress,
        )?;
        let mut permission_prompter = CliPermissionPrompter::new(self.permission_mode);
        let summary = runtime.run_turn(prompt, Some(&mut permission_prompter))?;
        let text = final_assistant_text(&summary).trim().to_string();
        runtime.shutdown_plugins()?;
        Ok(text)
    }

    fn run_internal_prompt_text(
        &self,
        prompt: &str,
        enable_tools: bool,
    ) -> Result<String, Box<dyn std::error::Error>> {
        self.run_internal_prompt_text_with_progress(prompt, enable_tools, None)
    }

    fn run_bughunter(&mut self, scope: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        println!("{}", format_bughunter_report(scope));
        // The header above describes the command; this is the command doing it.
        // Before this, `/bughunter` printed only that description - a spec card
        // for a bug hunt, with no hunt behind it.
        println!();
        let report = self.run_internal_prompt_text(&bughunter_prompt(scope), true)?;
        println!("{report}");
        Ok(())
    }

    fn run_ultraplan(&self, task: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        println!("{}", format_ultraplan_report(task));
        Ok(())
    }

    fn run_teleport(target: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        let Some(target) = target.map(str::trim).filter(|value| !value.is_empty()) else {
            println!("Usage: /teleport <symbol-or-path>");
            return Ok(());
        };

        println!("{}", render_teleport_report(target)?);
        Ok(())
    }

    fn run_debug_tool_call(&self, args: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        validate_no_args("/debug-tool-call", args)?;
        println!("{}", render_last_tool_debug_report(self.runtime.session())?);
        Ok(())
    }

    fn run_commit(&mut self, args: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        validate_no_args("/commit", args)?;
        let status = git_output(&["status", "--short", "--branch"])?;
        let summary = parse_git_workspace_summary(Some(&status));
        let branch = parse_git_status_branch(Some(&status));
        if summary.is_clean() {
            println!("{}", format_commit_skipped_report());
            return Ok(());
        }

        println!("{}", format_commit_preflight_report(branch.as_deref(), summary));
        Ok(())
    }

    fn run_pr(&self, context: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        let branch =
            resolve_git_branch_for(&env::current_dir()?).unwrap_or_else(|| "unknown".to_string());
        println!("{}", format_pr_report(&branch, context));
        Ok(())
    }

    fn run_issue(&self, context: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        println!("{}", format_issue_report(context));
        Ok(())
    }

    fn run_review(&mut self, scope: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        let cwd = env::current_dir()?;
        let review_scope = ReviewScope::parse(scope)?;
        let is_full_repo = matches!(&review_scope, ReviewScope::FullRepo(_));
        // Phase 2-C: full repo preflight handles its own Git/non-Git gate.
        // Non-full scopes still require a Git worktree at cwd.
        if !is_full_repo && !is_git_worktree(&cwd) {
            eprintln!("{}", non_git_review_error(&cwd));
            return Ok(());
        }
        let target = match collect_review_target(&cwd, review_scope) {
            Ok(t) => t,
            Err(e) => {
                // Phase 2-C: preflight block is a structured message, not a crash.
                eprintln!("{e}");
                return Ok(());
            }
        };
        if target.is_empty() {
            print_clean_review_scope(&target);
            return Ok(());
        }

        self.run_review_target(target)
    }

    fn handle_review_command(
        &mut self,
        scope: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match parse_review_history_command(scope)? {
            Some(command) => run_review_history_command(command),
            None => self.run_review(scope),
        }
    }

    fn run_review_target(
        &mut self,
        target: ReviewTarget,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let cwd = env::current_dir()?;
        let is_full_repo = matches!(&target.scope, ReviewScope::FullRepo(_));
        // Phase 2-C: full repo preflight handles its own Git/non-Git gate.
        // Non-full scopes still require a Git worktree at cwd.
        if !is_full_repo && !is_git_worktree(&cwd) {
            eprintln!("{}", non_git_review_error(&cwd));
            return Ok(()); // REPL: print friendly message and continue, do not exit process
        }
        // R1: for FullRepo, persist artifacts into the target repo, not cwd.
        let workspace_root = target.workspace_root.clone().unwrap_or_else(|| cwd.clone());
        let context = ReviewContext::new(target);
        let prompt = build_review_prompt(&context, ReviewPromptOptions::default());
        let review_text = self.run_turn_capture_text(&prompt, false)?;
        let report = ReviewReport::from_model_output(review_text);
        // C20.6-B UX-D: run deterministic evidence gate before persisting.
        let findings =
            runtime::code_review::evaluate_evidence_gate(report.findings, &context.target);
        let report = ReviewReport { findings, ..report };
        let artifact = persist_review_artifact(&workspace_root, &context.target, &report)?;
        println!("{}", format_review_completion_summary(&report, &artifact));
        Ok(())
    }
}

fn display_path_for_user(path: &std::path::Path) -> String {
    let value = path.display().to_string();
    if cfg!(windows) {
        if let Some(rest) = value.strip_prefix("\\\\?\\UNC\\") {
            return format!("\\\\{rest}");
        }
        if let Some(rest) = value.strip_prefix("\\\\?\\") {
            return rest.to_string();
        }
    }
    value
}

fn sessions_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let path = cwd.join(".claw").join("sessions");
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn create_managed_session_handle(
    session_id: &str,
) -> Result<SessionHandle, Box<dyn std::error::Error>> {
    let id = session_id.to_string();
    let path = sessions_dir()?.join(format!("{id}.{PRIMARY_SESSION_EXTENSION}"));
    Ok(SessionHandle { id, path })
}

// ---------------------------------------------------------------------------
// Crash recovery CLI 集成（A2）
//
// 设计依据：sego-c9-a2-zcode-handoff-2026-06-15.md + Codex 架构审查
// 两层架构：
//   1. 启动提示层 maybe_print_recovery_notice —— parse_args 后只读 recovery JSON
//   2. session 状态写入层 persist_recovery_for_cli —— session handle 已知后写
// 不在 parse_args 后立即写 active（session id/path 那时不可用，Codex 审查问题③）
// 不用 Drop guard 写 graceful（fallible IO 不清晰，Codex 审查 §4.3）
// ---------------------------------------------------------------------------

/// `启动提示层：parse_args` 后调用，只读 recovery JSON，不写、不扫描、不调模型。
///
/// 仅对会创建/恢复可持久化 session 且可能执行模型/工具的动作触发提示
/// （Repl / Prompt / `CodeReview` / ResumeSession）。对 Version/Help 等纯查询动作不触发。
fn maybe_print_recovery_notice(should_check: bool) {
    if !should_check {
        return;
    }
    let Ok(workspace_root) = env::current_dir() else {
        return;
    };
    let assessment = runtime::recovery::assess_recovery_state(&workspace_root);
    if assessment.availability == runtime::recovery::RecoveryAvailability::Recoverable
        || assessment.availability == runtime::recovery::RecoveryAvailability::MissingSession
    {
        println!("{}", assessment.message);
        println!("hint: run `sego --resume latest` to restore the previous session.");
    }
}

/// 台账层：在 session handle 已知后打开本次运行的 ledger 条目。
///
/// 粒度 = **一次 `sego` 运行**（Founder 裁定，`SEG-DEV-001` §1.33）：一次运行恰好创建一个
/// session，所以 session id 就是 task id。goal 是"这次运行被启动来做什么"——带 `--prompt`
/// 时就是那段文本，裸 REPL 先写动作名，用户真正说出的第一句由 `note_run_goal` 替换。
///
/// **这一层为什么非要补上**：`track_process` 走的是 `update_task`，而 `update_task` 需要条目
/// 已经存在（否则 `NoTask`）。`mcp_stdio` 与 bash 工具早就在调 `record_spawn` /
/// `record_exit`，但那些调用是 fire-and-forget，错误被丢弃——所以在打开条目之前，
/// **整套进程记录一直是静默失败的**，而不是"记了但没人看"。
fn begin_run_task(session_id: &str, goal: &str) {
    let store = runtime::process_tree::workspace_task_store();
    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let repo = find_git_root_in(&cwd).ok().map(|path| path.display().to_string());
    let branch = resolve_git_branch_for(&cwd);
    let commit = run_git_capture_in(&cwd, &["rev-parse", "HEAD"]).map(|out| out.trim().to_string());

    // 两次都是尽力而为：台账是恢复的辅助物，打不开条目不应让用户真正要跑的那次运行失败。
    if store
        .start_task(
            session_id,
            goal,
            repo.as_deref(),
            branch.as_deref(),
            commit.as_deref(),
            Vec::new(),
        )
        .is_ok()
    {
        // `start_task` 只写 task 记录；恢复提示是**后来那次运行真正读的东西**，
        // 所以必须在这里生成，而不是等第一次 `update_task` 顺带写出来。
        let _ = store.write_recovery_prompt_public();
    }
}

/// 用用户自己那句话替换本次运行的 goal，并重生成恢复提示。
///
/// `update_task` 会顺带重写 recovery prompt，所以后来那次运行读到的"在做什么"是用户的原话，
/// 而不是启动标签。
fn note_run_goal(goal: &str) {
    let store = runtime::process_tree::workspace_task_store();
    let _ = store.update_task(|task| task.current_goal = goal.to_string());
}

/// 干净退出时收口本次运行的 ledger 条目。
///
/// **没有这一步，台账就会说谎**：task 文件是"当前活跃任务"的单一记录，只开不关会让下一次
/// 启动读到一个早已结束、却仍标为 running 的任务（还带着陈旧的进程列表），把"可恢复"
/// 变成误报。收口与 `persist_recovery_for_cli(Graceful, …)` 成对出现。
fn complete_run_task() {
    let store = runtime::process_tree::workspace_task_store();
    let _ = store.complete_task();
}

/// session 状态写入层：在 session handle 已知后调用，写入完整 recovery record。
///
/// 错误路径（进程崩溃 / Ctrl+C / 窗口关闭）不会调用本函数写 graceful，
/// 因此 exit-state 保留上次的 active，下次启动提示可恢复。
fn persist_recovery_for_cli(
    state: runtime::recovery::RecoveryExitState,
    session_id: &str,
    session_path: &Path,
    model: Option<&str>,
    last_user_goal: Option<&str>,
) {
    let Ok(workspace_root) = env::current_dir() else {
        return;
    };
    let Ok(cwd) = env::current_dir() else {
        return;
    };
    let mut update =
        runtime::recovery::RecoveryStateUpdate::new(session_id, session_path, cwd, state);
    if let Some(model) = model {
        update = update.with_model(model.to_string());
    }
    if let Some(goal) = last_user_goal {
        update = update.with_last_user_goal(goal.to_string());
    }
    // recovery 写入失败不应影响主流程，只记录到 stderr。
    if let Err(error) = runtime::recovery::persist_recovery_state(&workspace_root, update) {
        eprintln!("warning: failed to persist recovery state: {error}");
    }
}

/// `/recovery-export` 的共享实现：读取当前 recovery 状态，渲染 summary，写文件。
///
/// resume 模式和 REPL 模式共用此 helper（Codex 审查 §2.3 建议），
/// 避免"registered but not yet implemented"的断裂体验。
///
/// 路径语义：
/// - 用户传 path → 尊重用户路径，不强制 .txt（区别于 /export 的 `resolve_export_path`）
/// - 用户不传 path → 默认写到 .sego/recovery/recovery-summary.md
fn write_recovery_export(
    requested_path: Option<&str>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let workspace_root = env::current_dir()?;
    let assessment = runtime::recovery::assess_recovery_state(&workspace_root);
    let summary = runtime::recovery::render_recovery_summary(&assessment);

    let final_path = match requested_path {
        Some(path) if !path.trim().is_empty() => PathBuf::from(path),
        _ => runtime::recovery::recovery_summary_path(&workspace_root),
    };
    if let Some(parent) = final_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(&final_path, summary)?;
    Ok(final_path)
}

fn resolve_session_reference(reference: &str) -> Result<SessionHandle, Box<dyn std::error::Error>> {
    if SESSION_REFERENCE_ALIASES.iter().any(|alias| reference.eq_ignore_ascii_case(alias)) {
        let latest = latest_managed_session()?;
        return Ok(SessionHandle { id: latest.id, path: latest.path });
    }

    let direct = PathBuf::from(reference);
    let looks_like_path = direct.extension().is_some() || direct.components().count() > 1;
    let path = if direct.exists() {
        direct
    } else if looks_like_path {
        return Err(format_missing_session_reference(reference).into());
    } else {
        resolve_managed_session_path(reference)?
    };
    let id = path
        .file_name()
        .and_then(|value| value.to_str())
        .and_then(|name| {
            name.strip_suffix(&format!(".{PRIMARY_SESSION_EXTENSION}"))
                .or_else(|| name.strip_suffix(&format!(".{LEGACY_SESSION_EXTENSION}")))
        })
        .unwrap_or(reference)
        .to_string();
    Ok(SessionHandle { id, path })
}

fn resolve_managed_session_path(session_id: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let directory = sessions_dir()?;
    for extension in [PRIMARY_SESSION_EXTENSION, LEGACY_SESSION_EXTENSION] {
        let path = directory.join(format!("{session_id}.{extension}"));
        if path.exists() {
            return Ok(path);
        }
    }
    Err(format_missing_session_reference(session_id).into())
}

fn is_managed_session_file(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()).is_some_and(|extension| {
        extension == PRIMARY_SESSION_EXTENSION || extension == LEGACY_SESSION_EXTENSION
    })
}

fn list_managed_sessions() -> Result<Vec<ManagedSessionSummary>, Box<dyn std::error::Error>> {
    let mut sessions = Vec::new();
    for entry in fs::read_dir(sessions_dir()?)? {
        let entry = entry?;
        let path = entry.path();
        if !is_managed_session_file(&path) {
            continue;
        }
        let metadata = entry.metadata()?;
        let modified_epoch_millis = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis())
            .unwrap_or_default();
        let (id, message_count, parent_session_id, branch_name) =
            match Session::load_from_path(&path) {
                Ok(session) => {
                    let parent_session_id =
                        session.fork.as_ref().map(|fork| fork.parent_session_id.clone());
                    let branch_name =
                        session.fork.as_ref().and_then(|fork| fork.branch_name.clone());
                    (session.session_id, session.messages.len(), parent_session_id, branch_name)
                }
                Err(_) => (
                    path.file_stem()
                        .and_then(|value| value.to_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    0,
                    None,
                    None,
                ),
            };
        sessions.push(ManagedSessionSummary {
            id,
            path,
            modified_epoch_millis,
            message_count,
            parent_session_id,
            branch_name,
        });
    }
    sessions.sort_by(|left, right| {
        right
            .modified_epoch_millis
            .cmp(&left.modified_epoch_millis)
            .then_with(|| right.id.cmp(&left.id))
    });
    Ok(sessions)
}

fn latest_managed_session() -> Result<ManagedSessionSummary, Box<dyn std::error::Error>> {
    list_managed_sessions()?.into_iter().next().ok_or_else(|| format_no_managed_sessions().into())
}

fn render_session_list(active_session_id: &str) -> Result<String, Box<dyn std::error::Error>> {
    let sessions = list_managed_sessions()?;
    let mut lines =
        vec!["Sessions".to_string(), format!("  Directory         {}", sessions_dir()?.display())];
    if sessions.is_empty() {
        lines.push("  No managed sessions saved yet.".to_string());
        return Ok(lines.join("\n"));
    }
    for session in sessions {
        let marker = if session.id == active_session_id { "● current" } else { "○ saved" };
        let lineage = match (session.branch_name.as_deref(), session.parent_session_id.as_deref()) {
            (Some(branch_name), Some(parent_session_id)) => {
                format!(" branch={branch_name} from={parent_session_id}")
            }
            (None, Some(parent_session_id)) => format!(" from={parent_session_id}"),
            (Some(branch_name), None) => format!(" branch={branch_name}"),
            (None, None) => String::new(),
        };
        lines.push(format!(
            "  {id:<20} {marker:<10} msgs={msgs:<4} modified={modified}{lineage} path={path}",
            id = session.id,
            msgs = session.message_count,
            modified = format_session_modified_age(session.modified_epoch_millis),
            lineage = lineage,
            path = session.path.display(),
        ));
    }
    Ok(lines.join("\n"))
}

fn write_session_clear_backup(
    session: &Session,
    session_path: &Path,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let backup_path = session_clear_backup_path(session_path);
    session.save_to_path(&backup_path)?;
    Ok(backup_path)
}

fn session_clear_backup_path(session_path: &Path) -> PathBuf {
    let timestamp = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map_or(0, |duration| duration.as_millis());
    let file_name =
        session_path.file_name().and_then(|value| value.to_str()).unwrap_or("session.jsonl");
    session_path.with_file_name(format!("{file_name}.before-clear-{timestamp}.bak"))
}

fn render_nl_intent_miss(miss: &NlIntentMiss) -> String {
    match miss {
        NlIntentMiss::NeedsMoreDetail { action, example } => format!(
            "Sego 没能确定要执行的本地动作。\n  疑似动作        {action}\n  可以这样说      {example}\n  查看更多        /dir"
        ),
    }
}

fn print_status_snapshot(
    model: &str,
    permission_mode: PermissionMode,
    output_format: CliOutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let ctx = status_context(None)?;
    match output_format {
        CliOutputFormat::Json => {
            println!(
                "{}",
                serde_json::json!({
                    "model": model,
                    "permission_mode": permission_mode.as_str(),
                    "cwd": ctx.cwd.display().to_string(),
                    "git_branch": ctx.git_branch,
                    "git_summary": ctx.git_summary.headline(),
                    "config_files_loaded": ctx.loaded_config_files,
                    "memory_files": ctx.memory_file_count,
                    "sandbox_supported": ctx.sandbox_status.supported,
                    "sandbox_active": ctx.sandbox_status.active,
                })
            );
        }
        CliOutputFormat::Text => {
            println!(
                "{}",
                format_status_report(
                    model,
                    StatusUsage {
                        message_count: 0,
                        turns: 0,
                        latest: TokenUsage::default(),
                        cumulative: TokenUsage::default(),
                        estimated_tokens: 0,
                    },
                    permission_mode.as_str(),
                    &ctx,
                )
            );
        }
    }
    Ok(())
}

fn print_workspace_snapshot(
    output_format: CliOutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let context = workspace_context()?;
    match output_format {
        CliOutputFormat::Json => {
            println!(
                "{}",
                serde_json::json!({
                    "cwd": context.cwd.display().to_string(),
                    "project_root": context
                        .project_root
                        .as_ref()
                        .map(|path| path.display().to_string()),
                    "session_dir": context.session_dir.display().to_string(),
                    "recovery_dir": context.recovery_dir.display().to_string(),
                    "filesystem_mode": context.sandbox_status.filesystem_mode.as_str(),
                    "allowed_mounts": context.sandbox_status.allowed_mounts,
                })
            );
        }
        CliOutputFormat::Text => println!("{}", format_workspace_report(&context)),
    }
    Ok(())
}

fn print_workflow_review(
    last_n: Option<usize>,
    output_format: CliOutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let store = WorkflowStore::new(&cwd);

    match output_format {
        CliOutputFormat::Json => {
            let sessions = store.load_recent_sessions(last_n.unwrap_or(1))?;
            println!("{}", serde_json::to_string_pretty(&sessions)?);
        }
        CliOutputFormat::Text => {
            let count = last_n.unwrap_or(1);
            let sessions = store.load_recent_sessions(count)?;

            if sessions.is_empty() {
                println!("No workflow sessions found in {}.", cwd.display());
                println!("Sessions are recorded automatically when you run sego.");
                return Ok(());
            }

            println!("🤖 Sego Agent — Workflow Review — Last {} session(s)\n", sessions.len());

            for session in &sessions {
                let session_id = session["session_id"].as_str().unwrap_or("unknown");
                let efficiency = session["efficiency_score"].as_f64().unwrap_or(0.0);
                let failures = session["failure_count"].as_u64().unwrap_or(0);
                let recoveries = session["recovery_successes"].as_u64().unwrap_or(0);
                let green = session["green_level"].as_str().unwrap_or("not set");
                let task = session["task_description"].as_str().unwrap_or("untitled");

                println!("  {session_id}  {efficiency:>5.0}%  {task}");
                println!("    failures={failures}  recoveries={recoveries}  green={green}");
                println!();
            }

            // Show trends if available
            if let Ok(trends) = store.load_trends() {
                if trends.total_sessions > 1 {
                    println!("📈 Historical Trends");
                    println!("  Total sessions:     {}", trends.total_sessions);
                    println!("  Average efficiency: {:.0}%", trends.average_efficiency);
                    println!("  Improvement rate:   {:.0}%", trends.improvement_rate * 100.0);
                    println!("  Total recoveries:   {}", trends.total_recoveries);
                    if let Some(ref failure_type) = trends.most_common_failure {
                        println!("  Most common failure: {failure_type}");
                    }
                }
            }

            if sessions.len() >= 2 {
                println!();
                println!("💡 Run `sego learn` for optimization suggestions.");
            }
        }
    }
    Ok(())
}

fn print_workflow_learn(output_format: CliOutputFormat) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let store = WorkflowStore::new(&cwd);
    let trends = store.load_trends()?;

    if trends.total_sessions == 0 {
        println!("No workflow data yet. Run sego to start recording sessions.");
        return Ok(());
    }

    match output_format {
        CliOutputFormat::Json => {
            let mut output = serde_json::json!({
                "total_sessions": trends.total_sessions,
                "average_efficiency": trends.average_efficiency,
                "improvement_rate": trends.improvement_rate,
                "total_failures": trends.total_failures,
                "total_recoveries": trends.total_recoveries,
                "suggestions": Vec::<String>::new(),
            });

            let recent = store.load_recent_sessions(5)?;
            output["recent_sessions"] = serde_json::json!(recent);

            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        CliOutputFormat::Text => {
            println!("🦞 claw Learning Report\n");
            println!("Based on {} recorded session(s):\n", trends.total_sessions);

            println!("  Average efficiency:  {:.0}%", trends.average_efficiency);
            println!("  Average duration:    {}s", trends.average_duration_seconds);
            println!(
                "  Recovery rate:       {:.0}%",
                if trends.total_failures > 0 {
                    f64::from(trends.total_recoveries) / f64::from(trends.total_failures) * 100.0
                } else {
                    100.0
                }
            );
            println!();

            // Generate suggestions
            let mut suggestions = Vec::new();

            if trends.improvement_rate > 0.05 {
                suggestions.push(
                    "↑ Your efficiency is trending upward. Current practices are working well."
                        .to_string(),
                );
            } else if trends.improvement_rate < -0.05 {
                suggestions.push("↓ Efficiency has been declining. Review recent sessions for new failure patterns.".to_string());
            }

            if trends.average_efficiency < 70.0 {
                suggestions.push(format!(
                    "Your average efficiency ({:.0}%) is below 70%. Consider:\n\
                     • Enabling Green Contract (TargetedTests minimum)\n\
                     • Reviewing most common failure types\n\
                     • Running smaller, more focused tasks",
                    trends.average_efficiency
                ));
            }

            if trends.total_recoveries > 0 {
                let recovery_rate = if trends.total_failures > 0 {
                    f64::from(trends.total_recoveries) / f64::from(trends.total_failures) * 100.0
                } else {
                    100.0
                };
                let recoveries = trends.total_recoveries;
                let total = trends.total_failures;
                suggestions.push(format!(
                    "Recovery system: {recoveries} recoveries from {total} failures ({recovery_rate:.0}% success rate)."
                ));
            }

            if trends.total_sessions < 5 {
                suggestions.push(String::from(
                    "Not enough data for deep analysis. Continue using claw — more sessions = better insights.",
                ));
            }

            if suggestions.is_empty() {
                suggestions.push(String::from(
                    "Keep using claw consistently. More data enables deeper insights.",
                ));
            }

            println!("💡 Suggestions:");
            for (i, suggestion) in suggestions.iter().enumerate() {
                println!("  {}. {suggestion}", i + 1);
            }

            // Show recent sessions
            if let Ok(recent) = store.load_recent_sessions(3) {
                if !recent.is_empty() {
                    println!();
                    println!("📋 Recent sessions:");
                    for session in &recent {
                        let id = session["session_id"].as_str().unwrap_or("unknown");
                        let efficiency = session["efficiency_score"].as_f64().unwrap_or(0.0);
                        let task = session["task_description"].as_str().unwrap_or("untitled");
                        println!("  {id}  {efficiency:.0}%  {task}");
                    }
                }
            }
        }
    }
    Ok(())
}

fn print_doctor(output_format: CliOutputFormat) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let loader = ConfigLoader::default_for(&cwd);
    let runtime_config = loader.load().unwrap_or_else(|_| runtime::RuntimeConfig::empty());
    let sandbox = resolve_sandbox_status(runtime_config.sandbox(), &cwd);

    match output_format {
        CliOutputFormat::Json => {
            println!(
                "{}",
                serde_json::json!({
                    "cwd": cwd.display().to_string(),
                    "date": default_date(),
                    "sandbox_supported": sandbox.supported,
                    "sandbox_active": sandbox.active,
                    "in_container": sandbox.in_container,
                    "credential_file_privacy": match runtime::file_privacy() {
                        runtime::FilePrivacy::OwnerOnly => "owner_only",
                        runtime::FilePrivacy::DirectoryAclsOnly => "directory_acls_only",
                    },
                    "config_files_loaded": runtime_config.loaded_entries().len(),
                    "sego_version": VERSION,
                    "model": default_model(),
                })
            );
        }
        CliOutputFormat::Text => {
            println!("Sego Agent — System Diagnostics");
            println!();
            println!("  Version:          {VERSION}");
            println!("  Date:             {}", default_date());
            println!("  Working dir:      {}", cwd.display());
            println!("  Model:            {}", default_model());
            println!();
            println!("  Sandbox:");
            println!("    Supported:      {}", sandbox.supported);
            println!("    Active:         {}", sandbox.active);
            println!("    In container:   {}", sandbox.in_container);
            println!();
            println!("  File privacy:");
            println!(
                "    Credentials:    {}",
                match runtime::file_privacy() {
                    runtime::FilePrivacy::OwnerOnly =>
                        "owner-only; the mode is applied when the file is created",
                    runtime::FilePrivacy::DirectoryAclsOnly =>
                        "directory ACLs only; this process applies no restriction",
                }
            );
            println!();
            println!("  Config files:     {} loaded", runtime_config.loaded_entries().len());
            if let Ok(status) = status_context(None) {
                println!(
                    "  Git branch:       {}",
                    status.git_branch.as_deref().unwrap_or("unknown")
                );
                println!("  Git state:        {}", status.git_summary.headline());
            }
            println!();
            println!("  Run `sego workflow-review` for session analysis.");
            println!("  Run `sego learn` for optimization suggestions.");
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    assets: Vec<GithubReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubReleaseAsset {
    name: String,
    browser_download_url: String,
}

fn maybe_print_update_notice(enabled: bool) {
    if !enabled || env::var_os(UPDATE_CHECK_ENV).is_some() {
        return;
    }

    let Ok(Some(release)) = fetch_latest_release() else {
        return;
    };
    if is_newer_version(&release.tag_name, VERSION) {
        eprintln!(
            "Sego update available: current v{VERSION}, latest {}. Run `sego update` to install.",
            release.tag_name
        );
    }
}

fn run_update(check_only: bool) -> Result<(), Box<dyn std::error::Error>> {
    let Some(release) = fetch_latest_release()? else {
        println!("Could not check for updates. Please try again later.");
        return Ok(());
    };

    println!("Current version: v{VERSION}");
    println!("Latest version:  {}", release.tag_name);

    if !is_newer_version(&release.tag_name, VERSION) {
        println!("Sego is already up to date.");
        return Ok(());
    }

    if check_only {
        println!("Update available: {0} -> {1}", VERSION, release.tag_name);
        println!("To install:  sego update");
        println!("Or download: {0}", release.html_url);
        return Ok(());
    }

    let Some(asset) = release.assets.iter().find(|asset| asset.name == UPDATE_WINDOWS_ASSET) else {
        println!(
            "Latest release does not include {UPDATE_WINDOWS_ASSET}. Download manually: {}",
            release.html_url
        );
        return Ok(());
    };

    if !cfg!(windows) {
        println!(
            "Automatic update is currently Windows-only. Download manually: {}",
            release.html_url
        );
        return Ok(());
    }

    let current_exe = env::current_exe()?;
    let install_dir = current_exe.parent().ok_or("could not resolve Sego install directory")?;
    let temp_exe = install_dir.join("sego.update.exe");

    println!("Downloading {} ...", asset.browser_download_url);
    download_file(&asset.browser_download_url, &temp_exe)?;

    // Verify against the release's published checksums *before* the binary is
    // allowed to replace anything. A failed verification removes the download
    // and leaves the installed version untouched.
    verify_release_checksum(&release, &asset.name, &temp_exe)?;

    let script_path = install_dir.join("sego-update.cmd");
    let backup_exe = install_dir.join("sego.previous.exe");
    let script = build_update_script(
        &release.tag_name,
        &current_exe,
        &backup_exe,
        &temp_exe,
        &release.html_url,
    );
    fs::write(&script_path, script)?;

    println!("Updater prepared. Sego will close and finish the replacement in a new window.");
    Command::new("cmd")
        .args(["/C", "start", "Sego Update", &script_path.display().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

/// Build the batch script that finishes an update after this process exits.
///
/// Every step checks its exit code and has somewhere to go. The previous
/// version moved the current binary aside and then ran `move` for the new one
/// with `>nul` and no error branch: had that move failed, the install
/// directory would have held no executable at all while the working binary sat
/// in the backup, recoverable only by hand. `restore` puts the backup back on
/// any failure between the swap and a successful `--version` smoke test.
fn build_update_script(
    tag: &str,
    current: &Path,
    backup: &Path,
    temp: &Path,
    release_url: &str,
) -> String {
    format!(
        "@echo off\r\n\
         setlocal EnableExtensions\r\n\
         echo Updating Sego to {tag}...\r\n\
         timeout /t 1 /nobreak >nul\r\n\
         if exist \"{backup}\" del /f /q \"{backup}\"\r\n\
         if exist \"{current}\" move /y \"{current}\" \"{backup}\" >nul\r\n\
         if errorlevel 1 goto :backup_failed\r\n\
         move /y \"{temp}\" \"{current}\" >nul\r\n\
         if errorlevel 1 goto :replace_failed\r\n\
         \"{current}\" --version\r\n\
         if errorlevel 1 goto :launch_failed\r\n\
         echo Sego updated to {tag}.\r\n\
         goto :done\r\n\
         \r\n\
         :replace_failed\r\n\
         echo Update failed: the new binary could not be put in place.\r\n\
         goto :restore\r\n\
         \r\n\
         :launch_failed\r\n\
         echo Update failed: the new binary did not start.\r\n\
         goto :restore\r\n\
         \r\n\
         :restore\r\n\
         if not exist \"{backup}\" goto :failed\r\n\
         del /f /q \"{current}\" >nul 2>&1\r\n\
         move /y \"{backup}\" \"{current}\" >nul\r\n\
         if errorlevel 1 goto :failed\r\n\
         echo Previous version restored.\r\n\
         goto :failed\r\n\
         \r\n\
         :backup_failed\r\n\
         echo Update failed: the installed binary could not be moved aside. Nothing was changed.\r\n\
         goto :failed\r\n\
         \r\n\
         :failed\r\n\
         echo Sego was NOT updated.\r\n\
         echo Download manually: {release_url}\r\n\
         pause\r\n\
         exit /b 1\r\n\
         \r\n\
         :done\r\n\
         pause\r\n\
         exit /b 0\r\n",
        tag = tag,
        backup = backup.display(),
        current = current.display(),
        temp = temp.display(),
        release_url = release_url,
    )
}

/// Compare a downloaded release asset against the `checksums.txt` published
/// with the same release.
///
/// A missing checksums file, a missing entry, or a mismatch removes the
/// download and returns an error, so an unverified binary never reaches the
/// replacement step. `SEGO_UPDATE_ALLOW_UNVERIFIED=1` is an explicit opt-out.
fn verify_release_checksum(
    release: &GithubRelease,
    asset_name: &str,
    downloaded: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if env::var_os(UPDATE_ALLOW_UNVERIFIED_ENV).is_some() {
        println!("WARNING: {UPDATE_ALLOW_UNVERIFIED_ENV} is set - skipping checksum verification.");
        return Ok(());
    }

    let Some(checksums) = release.assets.iter().find(|asset| asset.name == UPDATE_CHECKSUMS_ASSET)
    else {
        let _ = fs::remove_file(downloaded);
        return Err(format!(
            "release {} does not publish {UPDATE_CHECKSUMS_ASSET}; refusing to install an \
             unverified binary (set {UPDATE_ALLOW_UNVERIFIED_ENV}=1 to override)",
            release.tag_name
        )
        .into());
    };

    let body = match fetch_text(&checksums.browser_download_url) {
        Ok(body) => body,
        Err(error) => {
            let _ = fs::remove_file(downloaded);
            return Err(format!("could not read {UPDATE_CHECKSUMS_ASSET}: {error}").into());
        }
    };

    let Some(expected) = expected_hash_for(&body, asset_name) else {
        let _ = fs::remove_file(downloaded);
        return Err(format!(
            "{UPDATE_CHECKSUMS_ASSET} for {} does not list {asset_name}; refusing to install an \
             unverified binary",
            release.tag_name
        )
        .into());
    };

    let actual = sha256_of_file(downloaded)?;
    if actual != expected {
        let _ = fs::remove_file(downloaded);
        return Err(format!(
            "checksum mismatch for {asset_name}: expected {expected}, got {actual}. The download \
             was discarded and nothing was replaced."
        )
        .into());
    }
    println!("Checksum verified ({expected}).");
    Ok(())
}

/// Extract the hash for `asset_name` from a `sha256sum`-style listing, whose
/// lines are `<hash>  <name>` (`*` prefix optional, as produced by `sha256sum -b`).
fn expected_hash_for(checksums: &str, asset_name: &str) -> Option<String> {
    checksums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (name == asset_name).then(|| hash.to_ascii_lowercase())
    })
}

fn sha256_of_file(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    use sha2::{Digest, Sha256};
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn fetch_text(url: &str) -> Result<String, Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;
        let response = client
            .get(url)
            .header("user-agent", concat!("sego/", env!("CARGO_PKG_VERSION")))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(format!("request failed with {}", response.status()).into());
        }
        Ok::<_, Box<dyn std::error::Error>>(response.text().await?)
    })
}

fn fetch_latest_release() -> Result<Option<GithubRelease>, Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        let client = reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?;
        let response = client
            .get(UPDATE_LATEST_URL)
            .header("user-agent", concat!("sego/", env!("CARGO_PKG_VERSION")))
            .send()
            .await?;
        if !response.status().is_success() {
            return Ok(None);
        }
        let release = response.json::<GithubRelease>().await?;
        Ok(Some(release))
    })
}

fn download_file(url: &str, destination: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Runtime::new()?;
    let bytes = runtime.block_on(async {
        let client = reqwest::Client::builder().timeout(Duration::from_secs(120)).build()?;
        let response = client
            .get(url)
            .header("user-agent", concat!("sego/", env!("CARGO_PKG_VERSION")))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(format!("download failed with {}", response.status()).into());
        }
        Ok::<_, Box<dyn std::error::Error>>(response.bytes().await?)
    })?;
    fs::write(destination, bytes)?;
    Ok(())
}

fn is_newer_version(latest_tag: &str, current_version: &str) -> bool {
    parse_version_triplet(latest_tag) > parse_version_triplet(current_version)
}

fn parse_version_triplet(value: &str) -> (u64, u64, u64) {
    let normalized = value.trim().trim_start_matches('v');
    let mut parts = normalized.split('.').map(|part| part.parse::<u64>().unwrap_or(0));
    (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0))
}

fn print_telemetry(
    action: Option<&str>,
    output_format: CliOutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let mut community = CommunityLearning::new(&cwd);

    match action.unwrap_or("status") {
        "on" | "enable" => {
            community.enable();
            match output_format {
                CliOutputFormat::Text => println!("Community learning telemetry: ENABLED\n\nAnonymous stats (failure types, efficiency scores, recovery rates) will be reported to help improve Sego for everyone.\nNo conversation content, code, API keys, or personal data is ever collected.\n\nRun 'sego telemetry off' to disable."),
                CliOutputFormat::Json => println!("{}", serde_json::json!({"telemetry": "enabled"})),
            }
        }
        "off" | "disable" => {
            community.disable();
            match output_format {
                CliOutputFormat::Text => println!("Community learning telemetry: DISABLED"),
                CliOutputFormat::Json => {
                    println!("{}", serde_json::json!({"telemetry": "disabled"}));
                }
            }
        }
        "export" => {
            let pending = community.export_pending();
            if output_format == CliOutputFormat::Json {
                println!("{pending}");
            } else if pending == "[]" {
                println!("No pending telemetry reports.");
            } else {
                println!("Pending reports:\n{pending}");
            }
        }
        _ => {
            let status = if community.is_enabled() { "enabled" } else { "disabled" };
            match output_format {
                CliOutputFormat::Text => {
                    println!("Community learning telemetry: {status}");
                    println!();
                    println!("  sego telemetry on   — Enable anonymous stats sharing");
                    println!("  sego telemetry off  — Disable");
                    println!("  sego telemetry export — View pending reports");
                    println!();
                    println!("When enabled, Sego shares anonymous statistics (failure types,");
                    println!("efficiency scores, recovery rates) to improve the community model.");
                    println!("No conversation content, code, or personal data is ever collected.");
                }
                CliOutputFormat::Json => {
                    println!("{}", serde_json::json!({"telemetry": status}));
                }
            }
        }
    }
    Ok(())
}

fn print_sandbox_status_snapshot(
    output_format: CliOutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let loader = ConfigLoader::default_for(&cwd);
    let runtime_config = loader.load().unwrap_or_else(|_| runtime::RuntimeConfig::empty());
    let sandbox = resolve_sandbox_status(runtime_config.sandbox(), &cwd);
    match output_format {
        CliOutputFormat::Json => {
            println!(
                "{}",
                serde_json::json!({
                    "supported": sandbox.supported,
                    "active": sandbox.active,
                    "in_container": sandbox.in_container,
                    "filesystem_mode": format!("{:?}", sandbox.filesystem_mode),
                })
            );
        }
        CliOutputFormat::Text => {
            println!("{}", format_sandbox_report(&sandbox));
        }
    }
    Ok(())
}

fn render_config_report(section: Option<&str>) -> Result<String, Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let loader = ConfigLoader::default_for(&cwd);
    let discovered = loader.discover();
    let runtime_config = loader.load()?;

    let mut lines = vec![
        format!(
            "Config
  Working directory {}
  Loaded files      {}
  Merged keys       {}",
            cwd.display(),
            runtime_config.loaded_entries().len(),
            runtime_config.merged().len()
        ),
        "Discovered files".to_string(),
    ];
    for entry in discovered {
        let source = match entry.source {
            ConfigSource::User => "user",
            ConfigSource::Project => "project",
            ConfigSource::Local => "local",
        };
        let status = if runtime_config
            .loaded_entries()
            .iter()
            .any(|loaded_entry| loaded_entry.path == entry.path)
        {
            "loaded"
        } else {
            "missing"
        };
        lines.push(format!("  {source:<7} {status:<7} {}", entry.path.display()));
    }

    if let Some(section) = section {
        lines.push(format!("Merged section: {section}"));
        let value = match section {
            "env" => runtime_config.get("env"),
            "hooks" => runtime_config.get("hooks"),
            "model" => runtime_config.get("model"),
            "plugins" => {
                runtime_config.get("plugins").or_else(|| runtime_config.get("enabledPlugins"))
            }
            other => {
                lines.push(format!(
                    "  Unsupported config section '{other}'. Use env, hooks, model, or plugins."
                ));
                return Ok(lines.join(
                    "
",
                ));
            }
        };
        lines.push(format!(
            "  {}",
            match value {
                Some(value) => value.render(),
                None => "<unset>".to_string(),
            }
        ));
        return Ok(lines.join(
            "
",
        ));
    }

    lines.push("Merged JSON".to_string());
    lines.push(format!("  {}", runtime_config.as_json().render()));
    Ok(lines.join(
        "
",
    ))
}

fn render_memory_report() -> Result<String, Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let project_context = ProjectContext::discover(&cwd, default_date())?;
    let mut lines = vec![format!(
        "Memory
  Working directory {}
  Instruction files {}",
        cwd.display(),
        project_context.instruction_files.len()
    )];
    if project_context.instruction_files.is_empty() {
        lines.push("Discovered files".to_string());
        lines.push(
            "  No CLAUDE instruction files discovered in the current directory ancestry."
                .to_string(),
        );
    } else {
        lines.push("Discovered files".to_string());
        for (index, file) in project_context.instruction_files.iter().enumerate() {
            let preview = file.content.lines().next().unwrap_or("").trim();
            let preview = if preview.is_empty() { "<empty>" } else { preview };
            lines.push(format!("  {}. {}", index + 1, file.path.display()));
            lines.push(format!("     lines={} preview={}", file.content.lines().count(), preview));
        }
    }
    Ok(lines.join(
        "
",
    ))
}

fn init_claude_md() -> Result<String, Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    Ok(initialize_repo(&cwd)?.render())
}

fn run_init() -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", init_claude_md()?);
    Ok(())
}

fn render_diff_report() -> Result<String, Box<dyn std::error::Error>> {
    render_diff_report_for(&env::current_dir()?)
}

fn render_diff_report_for(cwd: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let staged = run_git_diff_command_in(cwd, &["diff", "--cached"])?;
    let unstaged = run_git_diff_command_in(cwd, &["diff"])?;
    if staged.trim().is_empty() && unstaged.trim().is_empty() {
        return Ok(
            "Diff\n  Result           clean working tree\n  Detail           no current changes"
                .to_string(),
        );
    }

    let mut sections = Vec::new();
    if !staged.trim().is_empty() {
        sections.push(format!("Staged changes:\n{}", staged.trim_end()));
    }
    if !unstaged.trim().is_empty() {
        sections.push(format!("Unstaged changes:\n{}", unstaged.trim_end()));
    }

    Ok(format!("Diff\n\n{}", sections.join("\n\n")))
}

fn collect_review_target(
    cwd: &Path,
    scope: ReviewScope,
) -> Result<ReviewTarget, Box<dyn std::error::Error>> {
    // C20: Full repository audit - walk the tree instead of running git diff.
    if let ReviewScope::FullRepo(ref audit_path) = scope {
        // Phase 2-C: run allow/block preflight before any snapshot collection.
        let preflight = full_scope_preflight::run_full_review_preflight(cwd, audit_path);
        if preflight.is_block() {
            return Err(format_preflight_block_error(&preflight).into());
        }
        // Preflight allowed: use the resolved target from preflight evidence.
        let repo_root =
            preflight.resolved_target.clone().ok_or("preflight allow but no resolved target")?;
        let full_tree = collect_full_repo_snapshot(&repo_root)?;
        // R2: git_status from the target repo, not cwd.
        let git_status = if is_git_worktree(&repo_root) {
            run_git_diff_command_in(&repo_root, &["status", "--short", "--branch"])
                .unwrap_or_default()
        } else {
            String::new()
        };
        return Ok(ReviewTarget {
            scope,
            git_status,
            staged_diff: String::new(),
            unstaged_diff: String::new(),
            full_tree,
            workspace_root: Some(repo_root),
        });
    }

    let git_status = run_git_diff_command_in(cwd, &["status", "--short", "--branch"])?;
    let (staged_diff, unstaged_diff) = match &scope {
        ReviewScope::Workspace => (
            run_git_diff_command_in(cwd, &["diff", "--cached"])?,
            run_git_diff_command_in(cwd, &["diff"])?,
        ),
        ReviewScope::Staged => {
            (run_git_diff_command_in(cwd, &["diff", "--cached"])?, String::new())
        }
        ReviewScope::Unstaged => (String::new(), run_git_diff_command_in(cwd, &["diff"])?),
        ReviewScope::Path(path) => {
            // Phase 2-C: Git-path preflight - path must exist and be inside Git root.
            let preflight = full_scope_preflight::run_git_path_preflight(cwd, path);
            if preflight.is_block() {
                return Err(format_preflight_block_error(&preflight).into());
            }
            let path = path.to_string_lossy();
            (
                run_git_diff_command_in(cwd, &["diff", "--cached", "--", path.as_ref()])?,
                run_git_diff_command_in(cwd, &["diff", "--", path.as_ref()])?,
            )
        }
        ReviewScope::FullRepo(_) => unreachable!("FullRepo handled above"),
    };

    Ok(ReviewTarget {
        scope,
        git_status,
        staged_diff,
        unstaged_diff,
        full_tree: String::new(),
        workspace_root: None,
    })
}

fn collect_staged_review_paths(cwd: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let output =
        run_git_diff_command_in(cwd, &["diff", "--cached", "--name-only", "--diff-filter=ACMR"])?;
    Ok(output.lines().map(str::trim).filter(|line| !line.is_empty()).map(PathBuf::from).collect())
}
/// Files above this size are listed but not read into the snapshot.
const MAX_SINGLE_FILE_BYTES: u64 = 1_000_000;

/// C20+R1: Walk a repository directory, collect file tree, manifest contents,
/// and bounded key source file sampling.
/// Skips `.git`, `node_modules`, virtual envs, build artifacts, and cache dirs.
// One pass that builds the whole snapshot document: tree, manifests, sampled
// key files and the limits that were hit.
#[allow(clippy::too_many_lines)]
fn collect_full_repo_snapshot(repo_root: &Path) -> Result<String, Box<dyn std::error::Error>> {
    // The `as _` matters: the module already imports `std::io::Write` for the
    // terminal writers, and bringing `std::fmt::Write` in by name would make
    // the `writeln!` calls below ambiguous.
    use std::fmt::Write as _;

    const SKIP_DIRS: &[&str] = &[
        ".git",
        "node_modules",
        "__pycache__",
        ".venv",
        "venv",
        ".tox",
        ".mypy_cache",
        ".pytest_cache",
        ".ruff_cache",
        "target",
        "build",
        "dist",
        ".next",
        ".nuxt",
        "coverage",
        ".cache",
        ".idea",
        ".vscode",
        ".vs",
        ".cargo",
    ];

    // R3: lock files excluded from key content (noisy, large). They still appear in file tree.
    const KEY_MANIFEST_FILES: &[&str] = &[
        "Cargo.toml",
        "pyproject.toml",
        "requirements.txt",
        "setup.py",
        "setup.cfg",
        "package.json",
        "go.mod",
        "go.sum",
        "Makefile",
        "CMakeLists.txt",
        "Dockerfile",
        "docker-compose.yml",
        "docker-compose.yaml",
        ".github/workflows",
    ];

    // R1: entry-point files.
    const ENTRY_FILES: &[&str] = &[
        "main.py",
        "app.py",
        "webapp.py",
        "lib.rs",
        "main.rs",
        "index.ts",
        "index.js",
        "main.go",
        "manage.py",
        "cli.py",
        "run.py",
    ];

    // R1: README-like files.
    const README_FILES: &[&str] =
        &["README.md", "README_zh.md", "README.txt", "README.rst", "README"];

    // R1: key source dirs to sample.
    const KEY_SOURCE_DIRS: &[&str] = &["src", "crates", "packages", "lib"];
    const MAX_SOURCE_FILES_PER_DIR: usize = 8;
    const MAX_SOURCE_FILE_CHARS: usize = 6_000;

    const MAX_DEPTH: usize = 64;
    // R3: aggregate byte cap on key_contents to avoid huge prompts.
    const MAX_SNAPSHOT_BYTES: usize = 256 * 1024;

    let mut file_list: Vec<String> = Vec::new();
    let mut key_contents: Vec<String> = Vec::new();
    let mut total_key_bytes: usize = 0;
    let mut skipped_large: usize = 0;
    let mut unreadable: usize = 0;
    // R2: key for sampling cap is the first matching key-source ancestor component.
    let mut dir_source_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();

    // R2: stable ordering — collect top-level entries, sort, then walk.
    if let Ok(entries) = std::fs::read_dir(repo_root) {
        let mut top_entries: Vec<std::fs::DirEntry> = entries.flatten().collect();
        top_entries
            .sort_by(|a, b| a.file_name().to_string_lossy().cmp(&b.file_name().to_string_lossy()));
        for entry in top_entries {
            let path = entry.path();
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if SKIP_DIRS.contains(&file_name) {
                continue;
            }
            walk_repo_dir_r2(
                &path,
                repo_root,
                0,
                MAX_DEPTH,
                &mut file_list,
                &mut key_contents,
                &mut total_key_bytes,
                MAX_SNAPSHOT_BYTES,
                &mut skipped_large,
                &mut unreadable,
                SKIP_DIRS,
                KEY_MANIFEST_FILES,
                ENTRY_FILES,
                README_FILES,
                KEY_SOURCE_DIRS,
                &mut dir_source_counts,
                MAX_SOURCE_FILES_PER_DIR,
                MAX_SOURCE_FILE_CHARS,
            );
        }
    }

    file_list.sort();
    let mut output = String::new();
    output.push_str("## File tree\n");
    for f in &file_list {
        output.push_str(f);
        output.push('\n');
    }
    output.push('\n');

    // R3: snapshot summary with limits info.
    output.push_str("## Snapshot limits\n");
    let cap_reached = if total_key_bytes >= MAX_SNAPSHOT_BYTES { "yes" } else { "no" };
    let _ = writeln!(output, "Key content cap reached: {cap_reached}");
    let _ = writeln!(output, "Skipped large files: {skipped_large}");
    let _ = writeln!(output, "Unreadable files: {unreadable}");
    output.push('\n');

    if !key_contents.is_empty() {
        output.push_str("## Key files\n\n");
        for block in &key_contents {
            output.push_str(block);
            output.push('\n');
        }
    }

    Ok(output)
}

/// R2+R3: recursive directory walker with depth limit, symlink detection, stable ordering,
/// aggregate byte cap, and unreadable-file tracking.
#[allow(clippy::too_many_arguments)]
// Recursive walk with a depth limit, an aggregate byte cap, symlink detection
// and unreadable-file tracking, threading five accumulators through the
// recursion; splitting it would mean passing the same bundle around twice.
#[allow(clippy::too_many_lines)]
fn walk_repo_dir_r2(
    current: &Path,
    repo_root: &Path,
    depth: usize,
    max_depth: usize,
    file_list: &mut Vec<String>,
    key_contents: &mut Vec<String>,
    total_key_bytes: &mut usize,
    max_snapshot_bytes: usize,
    skipped_large: &mut usize,
    unreadable: &mut usize,
    skip_dirs: &[&str],
    key_manifest_files: &[&str],
    entry_files: &[&str],
    readme_files: &[&str],
    key_source_dirs: &[&str],
    dir_source_counts: &mut std::collections::HashMap<String, usize>,
    max_source_per_dir: usize,
    max_source_chars: usize,
) {
    if depth > max_depth || !current.exists() {
        return;
    }
    // R2: detect and skip symlinks to avoid loops.
    if let Ok(meta) = current.symlink_metadata() {
        if meta.file_type().is_symlink() {
            return;
        }
    }
    if current.is_dir() {
        let dir_name = current.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if skip_dirs.contains(&dir_name) {
            return;
        }
        // R2: stable ordering — collect entries, sort by name, then recurse.
        if let Ok(entries) = std::fs::read_dir(current) {
            let mut children: Vec<std::fs::DirEntry> = entries.flatten().collect();
            children.sort_by(|a, b| {
                a.file_name().to_string_lossy().cmp(&b.file_name().to_string_lossy())
            });
            for entry in children {
                walk_repo_dir_r2(
                    &entry.path(),
                    repo_root,
                    depth + 1,
                    max_depth,
                    file_list,
                    key_contents,
                    total_key_bytes,
                    max_snapshot_bytes,
                    skipped_large,
                    unreadable,
                    skip_dirs,
                    key_manifest_files,
                    entry_files,
                    readme_files,
                    key_source_dirs,
                    dir_source_counts,
                    max_source_per_dir,
                    max_source_chars,
                );
            }
        }
        return;
    }

    let Ok(relative) = current.strip_prefix(repo_root) else {
        return;
    };
    let rel_str = relative.to_string_lossy().replace('\\', "/");
    file_list.push(rel_str.clone());

    let file_name = current.file_name().and_then(|n| n.to_str()).unwrap_or("");

    let is_manifest = key_manifest_files.contains(&file_name)
        || rel_str.ends_with("/Cargo.toml")
        || rel_str.ends_with("/pyproject.toml")
        || rel_str.ends_with("/package.json")
        || rel_str.ends_with("/requirements.txt")
        || rel_str.ends_with("/go.mod")
        || rel_str.ends_with("/Makefile")
        || rel_str.ends_with("/Dockerfile");

    // R2: also match .yaml CI workflow files.
    // The extension is matched case-insensitively: a workflow file named `.YML`
    // is still a workflow file on a case-insensitive filesystem, and the cost of
    // not recognising one is that it goes unreviewed.
    let is_ci_workflow = rel_str.contains(".github/workflows")
        && (full_scope_preflight::has_extension(&rel_str, "yml")
            || full_scope_preflight::has_extension(&rel_str, "yaml"));
    let is_readme = readme_files.contains(&file_name);
    let is_entry = entry_files.contains(&file_name);

    // R2: find the first matching key-source ancestor for the counter key.
    let counter_key = relative
        .components()
        .find_map(|c| {
            let s = c.as_os_str().to_string_lossy();
            key_source_dirs.contains(&s.as_ref()).then(|| s.to_string())
        })
        .unwrap_or_else(|| {
            relative
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string()
        });

    let is_in_key_dir = !counter_key.is_empty() && key_source_dirs.contains(&counter_key.as_str());

    let mut should_sample_source = false;
    if is_in_key_dir && !is_manifest && !is_ci_workflow && !is_readme && !is_entry {
        let count = dir_source_counts.get(&counter_key).copied().unwrap_or(0);
        if count < max_source_per_dir {
            dir_source_counts.insert(counter_key.clone(), count + 1);
            should_sample_source = true;
        }
    }

    if is_manifest || is_ci_workflow || is_readme || is_entry || should_sample_source {
        // R3: check aggregate cap before reading.
        if *total_key_bytes >= max_snapshot_bytes {
            return;
        }
        // R3: check file size before reading; skip very large files.
        let file_size = current.metadata().map_or(0, |m| m.len());
        if file_size > MAX_SINGLE_FILE_BYTES {
            *skipped_large += 1;
            key_contents.push(format!("### {rel_str} [skipped: {file_size} bytes]\n"));
            return;
        }
        match std::fs::read_to_string(current) {
            Ok(content) => {
                let max_chars = if should_sample_source { max_source_chars } else { 4000 };
                let truncated = truncate_file_content(&content, max_chars);
                let status =
                    if content.chars().count() > max_chars { " [truncated]" } else { " [full]" };
                let block = format!("### {rel_str}{status}\n```\n{truncated}\n```");
                *total_key_bytes += block.len();
                key_contents.push(block);
            }
            Err(e) => {
                *unreadable += 1;
                key_contents.push(format!("### {} [unreadable: {}]\n", rel_str, e.kind()));
            }
        }
    }
}

fn truncate_file_content(content: &str, max_chars: usize) -> String {
    if content.chars().count() <= max_chars {
        return content.to_string();
    }
    let truncated: String = content.chars().take(max_chars).collect();
    format!("{truncated}\n\n[truncated: file exceeded {max_chars} characters]")
}

fn run_code_review_cli(
    model: String,
    allowed_tools: Option<AllowedToolSet>,
    permission_mode: PermissionMode,
    scope: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let review_scope = ReviewScope::parse(scope)?;
    let is_full_repo = matches!(&review_scope, ReviewScope::FullRepo(_));
    // Phase 2-C: full repo preflight handles its own Git/non-Git gate.
    // Non-full scopes still require a Git worktree at cwd.
    if !is_full_repo && !is_git_worktree(&cwd) {
        eprintln!("{}", non_git_review_error(&cwd));
        std::process::exit(1);
    }
    let target = match collect_review_target(&cwd, review_scope) {
        Ok(t) => t,
        Err(e) => {
            // Phase 2-C: preflight block is a structured message, not a crash.
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    if target.is_empty() {
        print_clean_review_scope(&target);
        return Ok(());
    }

    // A2 session 状态写入：active 在 LiveCli 创建后写，graceful 在成功返回后写。
    let mut cli = LiveCli::new(model, true, allowed_tools, permission_mode)?;
    persist_recovery_for_cli(
        runtime::recovery::RecoveryExitState::Active,
        cli.session_id(),
        cli.session_path(),
        Some(cli.model_name()),
        Some(scope.unwrap_or("code-review")),
    );
    begin_run_task(cli.session_id(), &format!("code review: {}", scope.unwrap_or("workspace")));
    cli.run_review_target(target)?;
    persist_recovery_for_cli(
        runtime::recovery::RecoveryExitState::Graceful,
        cli.session_id(),
        cli.session_path(),
        Some(cli.model_name()),
        Some(scope.unwrap_or("code-review")),
    );
    complete_run_task();
    Ok(())
}

fn run_review_history_command(
    command: ReviewHistoryCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        ReviewHistoryCommand::List => print_code_review_history(),
        ReviewHistoryCommand::Show { id, json } => {
            if json {
                print_code_review_summary_json(&id)
            } else {
                print_code_review_report(&id)
            }
        }
        ReviewHistoryCommand::Card { id } => print_code_review_card(&id),
        ReviewHistoryCommand::Status { id } => print_code_review_finding_status(&id),
        ReviewHistoryCommand::Mark { id, finding_id, status, note } => {
            mark_code_review_finding(&id, &finding_id, status, note)
        }
        ReviewHistoryCommand::Ready => print_code_review_readiness(),
        ReviewHistoryCommand::Summary => print_code_review_summary(),
        ReviewHistoryCommand::Tools => print_code_review_tools(),
        ReviewHistoryCommand::Safety { scope } => print_code_review_safety(scope),
    }
}

fn print_code_review_readiness() -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", render_code_review_readiness_for(&env::current_dir()?)?);
    Ok(())
}

fn render_code_review_readiness_for(cwd: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let report = build_code_review_readiness_report(cwd)?;
    Ok(report.render())
}

fn print_code_review_summary() -> Result<(), Box<dyn std::error::Error>> {
    println!("{}", render_code_review_summary_for(&env::current_dir()?)?);
    Ok(())
}

fn render_code_review_summary_for(cwd: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let report = build_code_review_summary_report(cwd)?;
    Ok(report.render())
}

fn build_code_review_summary_report(
    cwd: &Path,
) -> Result<CodeReviewSummaryReport, Box<dyn std::error::Error>> {
    let git_status = run_git_diff_command_in(cwd, &["status", "--short", "--branch"])?;
    let staged_diff = run_git_diff_command_in(cwd, &["diff", "--cached"])?;
    let unstaged_diff = run_git_diff_command_in(cwd, &["diff"])?;
    let staged_paths = collect_staged_review_paths(cwd)?;
    let safety_report = if staged_paths.is_empty() {
        None
    } else {
        Some(runtime::build_safety_lock_report_for_paths(
            cwd,
            staged_paths.iter(),
            runtime::SafetyScanMode::Staged,
        )?)
    };
    let verification_plan = build_verification_plan(cwd, VerificationScope::Fast);
    let mut review_entries = load_review_index(cwd)?;
    review_entries.sort_by_key(|entry| entry.created_at_epoch_seconds);
    let latest_review = review_entries.pop();
    let latest_status_counts = latest_review
        .as_ref()
        .map(|entry| latest_review_finding_statuses(cwd, &entry.id))
        .transpose()?
        .map(|statuses| ReviewFindingStatusCounts::from_entries(&statuses));

    Ok(CodeReviewSummaryReport {
        root: cwd.display().to_string(),
        git_status,
        staged_diff,
        unstaged_diff,
        staged_paths,
        safety_report,
        verification_plan,
        latest_review,
        latest_status_counts,
    })
}

fn build_code_review_readiness_report(
    cwd: &Path,
) -> Result<CodeReviewReadinessReport, Box<dyn std::error::Error>> {
    let staged_paths = collect_staged_review_paths(cwd)?;
    let safety_report = runtime::build_safety_lock_report_for_paths(
        cwd,
        staged_paths.iter(),
        runtime::SafetyScanMode::Staged,
    )?;
    let verification_plan = build_verification_plan(cwd, VerificationScope::Fast);

    Ok(CodeReviewReadinessReport {
        root: cwd.display().to_string(),
        staged_paths,
        safety_report,
        verification_plan,
    })
}

fn print_code_review_tools() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let report = runtime::build_tool_probe_report(&cwd)?;

    println!("Review Tool Plan");
    println!("  Root             {}", report.root);
    println!("  Mode             suggest-only");
    println!("  Safety           no tools were executed; no dependencies were installed");

    if report.signals.is_empty() {
        println!("  Projects         none detected");
    } else {
        println!("  Projects");
        for signal in &report.signals {
            println!("    {} (confidence {}%)", signal.language.label(), signal.confidence);
            if !signal.evidence_paths.is_empty() {
                println!("      Evidence      {}", signal.evidence_paths.join(", "));
            }
        }
    }

    if report.checks.is_empty() {
        println!("  Checks           no checks planned");
    } else {
        println!("  Suggested checks");
        for check in &report.checks {
            println!(
                "    [{}] {}\n      Command       {}\n      Purpose       {}\n      Risk          {}\n      Execution     {}",
                check.language.label(),
                check.tool,
                check.command,
                check.purpose,
                check.risk.label(),
                check.execution_mode
            );
        }
    }

    if !report.warnings.is_empty() {
        println!("  Warnings");
        for warning in &report.warnings {
            println!("    {warning}");
        }
    }

    println!("  Next step        run the relevant commands manually or ask Sego to verify after approval");
    Ok(())
}

fn print_code_review_safety(scope: SafetyReviewScope) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let report = match scope {
        SafetyReviewScope::Workspace => runtime::build_safety_lock_report(&cwd)?,
        SafetyReviewScope::Staged => {
            let paths = collect_staged_review_paths(&cwd)?;
            if paths.is_empty() {
                println!("Review Safety");
                println!("  Root             {}", cwd.display());
                println!("  Mode             read-only");
                println!("  Scope            staged");
                println!("  Safety           no files were modified; no tools were executed");
                println!("  Result           no staged files to scan");
                println!("  Next step        stage changes, then rerun /review safety staged");
                return Ok(());
            }
            runtime::build_safety_lock_report_for_paths(
                &cwd,
                paths,
                runtime::SafetyScanMode::Staged,
            )?
        }
    };

    println!("Review Safety");
    println!("  Root             {}", report.root);
    println!("  Mode             read-only");
    println!("  Scope            {}", report.mode.label());
    println!("  Safety           no files were modified; no tools were executed");

    if report.findings.is_empty() {
        println!("  Result           no obvious beginner-safety risks found");
    } else {
        println!("  Findings         {}", report.findings.len());
        for finding in &report.findings {
            let location = finding
                .line
                .map_or_else(|| finding.file.clone(), |line| format!("{}:{line}", finding.file));
            println!(
                "\n  [{}] {}\n    Category       {}\n    File           {}\n    Evidence       {}\n    Risk           {}\n    Suggestion     {}",
                finding.severity.label(),
                finding.title,
                finding.category.label(),
                location,
                finding.evidence,
                finding.risk,
                finding.suggestion
            );
        }
    }

    if !report.warnings.is_empty() {
        println!("  Warnings");
        for warning in &report.warnings {
            println!("    {warning}");
        }
    }

    if report.findings.iter().any(|finding| finding.severity == runtime::SafetySeverity::High) {
        println!("  Next step        fix high-risk items before asking Sego to verify or commit");
    } else {
        println!("  Next step        run /review tools and relevant verification commands");
    }
    Ok(())
}

fn print_code_review_history() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let mut entries = load_review_index(&cwd)?;
    entries.sort_by_key(|entry| entry.created_at_epoch_seconds);

    println!("Review History");
    if entries.is_empty() {
        println!("  Result           no review reports found");
        println!(
            "  Index            {}",
            cwd.join(".sego").join("reviews").join("index.jsonl").display()
        );
        return Ok(());
    }

    for entry in entries.iter().rev() {
        print_review_index_entry(entry);
    }

    Ok(())
}

fn print_review_index_entry(entry: &ReviewIndexEntry) {
    let highest_severity = entry.highest_severity.map_or("none", runtime::ReviewSeverity::label);
    println!(
        "  {}\n    Scope            {}\n    Findings         {}\n    Highest severity {}\n    Parse status     {}\n    Created          {}\n    Markdown         {}",
        entry.id,
        entry.scope,
        entry.finding_count,
        highest_severity,
        entry.parse_status.label(),
        entry.created_at_epoch_seconds,
        entry.markdown_path
    );
}

fn print_code_review_report(id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let entries = load_review_index(&cwd)?;
    if id == "latest" && entries.is_empty() {
        print_no_review_reports_found(&cwd);
        return Ok(());
    }
    let entry = resolve_review_entry(&entries, id)?;
    let markdown_path = resolve_review_markdown_path(&cwd, entry);
    let markdown = fs::read_to_string(&markdown_path)?;
    println!("{markdown}");
    Ok(())
}

fn print_code_review_summary_json(id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let summary = build_code_review_summary_json(&cwd, id)?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GeneratedReviewCard {
    review_id: String,
    card_path: PathBuf,
    latest_card_path: PathBuf,
    terminal_summary: String,
}

fn print_code_review_card(id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let generated = generate_review_card_for(&cwd, id)?;
    open_browser(&local_file_url(&generated.card_path))?;
    println!("{}", generated.terminal_summary);
    println!("  Auto-open: Windows default browser launch requested");
    Ok(())
}

fn generate_review_card_for(
    workspace_root: &Path,
    id: &str,
) -> Result<GeneratedReviewCard, Box<dyn std::error::Error>> {
    let entries = load_review_index(workspace_root)?;
    if entries.is_empty() {
        return Err("no review artifact found; run `sego review` first".into());
    }

    let entry = resolve_review_entry(&entries, id)?;
    if !entry.id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')) {
        return Err(format!("refusing unsafe review id for card output: {}", entry.id).into());
    }

    let report = load_review_report_from_index_entry(workspace_root, entry)?;
    let json_path = resolve_review_json_path(workspace_root, entry);
    let markdown_path = resolve_review_markdown_path(workspace_root, entry);
    let card_data = ReviewCardData {
        id: &entry.id,
        scope: &entry.scope,
        finding_count: entry.finding_count,
        highest_severity: entry.highest_severity.or_else(|| report.highest_severity()),
        parse_status: report.parse_status,
        findings: &report.findings,
        json_path: &json_path,
        markdown_path: &markdown_path,
    };
    let reviews_dir = workspace_root.join(".sego").join("reviews");
    fs::create_dir_all(&reviews_dir)?;
    let card_path = reviews_dir.join(format!("{}-card.html", entry.id));
    let latest_card_path = reviews_dir.join("latest-card.html");
    let html = render_review_card_html(&card_data);
    fs::write(&card_path, &html)?;
    fs::write(&latest_card_path, html)?;

    Ok(GeneratedReviewCard {
        review_id: entry.id.clone(),
        terminal_summary: render_review_card_terminal_summary(&card_data, &card_path),
        card_path,
        latest_card_path,
    })
}

fn build_code_review_summary_json(
    cwd: &Path,
    id: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let entries = load_review_index(cwd)?;

    if id == "latest" && entries.is_empty() {
        let index_path = cwd.join(".sego").join("reviews").join("index.jsonl");
        return Ok(json!({
            "schema_version": 1,
            "kind": "sego_latest_review_summary",
            "found": false,
            "index_path": index_path.display().to_string(),
            "next_step": "run /review staged, /review workspace, or sego review --full <path>",
        }));
    }

    let entry = resolve_review_entry(&entries, id)?;
    Ok(build_review_summary_json_value(
        id,
        entry,
        &review_finding_status_counts_for_summary(cwd, entry)?,
    ))
}

fn print_no_review_reports_found(cwd: &Path) {
    let index_path = cwd.join(".sego").join("reviews").join("index.jsonl");
    println!("Review");
    println!("  Result           no review reports found");
    println!("  Index            {}", index_path.display());
    println!(
        "  Next step        run /review staged, /review workspace, or sego review --full <path>"
    );
}

fn review_finding_status_counts_for_summary(
    cwd: &Path,
    entry: &ReviewIndexEntry,
) -> Result<ReviewFindingStatusCounts, Box<dyn std::error::Error>> {
    let statuses = latest_review_finding_statuses(cwd, &entry.id)?;
    let mut counts = ReviewFindingStatusCounts { open: entry.finding_count, ..Default::default() };
    for status in statuses.values().map(|entry| entry.status) {
        match status {
            ReviewFindingStatus::Open => {}
            ReviewFindingStatus::Acknowledged => {
                counts.open = counts.open.saturating_sub(1);
                counts.acknowledged += 1;
            }
            ReviewFindingStatus::Fixed => {
                counts.open = counts.open.saturating_sub(1);
                counts.fixed += 1;
            }
            ReviewFindingStatus::AcceptedRisk => {
                counts.open = counts.open.saturating_sub(1);
                counts.accepted_risk += 1;
            }
            ReviewFindingStatus::FalsePositive => {
                counts.open = counts.open.saturating_sub(1);
                counts.false_positive += 1;
            }
            ReviewFindingStatus::Ignored => {
                counts.open = counts.open.saturating_sub(1);
                counts.ignored += 1;
            }
        }
    }
    Ok(counts)
}

fn print_code_review_finding_status(id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let entries = load_review_index(&cwd)?;
    let entry = resolve_review_entry(&entries, id)?;
    let report = load_review_report_from_index_entry(&cwd, entry)?;
    let latest_statuses = latest_review_finding_statuses(&cwd, &entry.id)?;

    println!("Review Finding Status");
    println!("  Review           {}", entry.id);
    println!("  Findings         {}", report.findings.len());
    if report.findings.is_empty() {
        println!("  Result           no structured findings");
        return Ok(());
    }

    for finding in &report.findings {
        let status = latest_statuses
            .get(&finding.id)
            .map_or(ReviewFindingStatus::Open, |entry| entry.status);
        let line = finding.line.map_or_else(|| "-".to_string(), |line| line.to_string());
        println!(
            "  {}\n    Status           {}\n    Severity         {}\n    Location         {}:{}\n    Title            {}",
            finding.id,
            status.label(),
            finding.severity.label(),
            finding.file,
            line,
            finding.title
        );
        if let Some(note) = latest_statuses.get(&finding.id).and_then(|entry| entry.note.as_ref()) {
            println!("    Note             {note}");
        }
    }

    Ok(())
}

fn mark_code_review_finding(
    id: &str,
    finding_id: &str,
    status: ReviewFindingStatus,
    note: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let entries = load_review_index(&cwd)?;
    let entry = resolve_review_entry(&entries, id)?;
    let report = load_review_report_from_index_entry(&cwd, entry)?;
    if !report.findings.iter().any(|finding| finding.id == finding_id) {
        return Err(format!("finding not found in {}: {finding_id}", entry.id).into());
    }

    let status_entry = record_review_finding_status(&cwd, &entry.id, finding_id, status, note)?;

    println!("Review Finding Marked");
    println!("  Review           {}", status_entry.report_id);
    println!("  Finding          {}", status_entry.finding_id);
    println!("  Status           {}", status_entry.status.label());
    println!("  Updated          {}", status_entry.updated_at_epoch_seconds);
    if let Some(note) = status_entry.note {
        println!("  Note             {note}");
    }

    Ok(())
}

fn resolve_review_entry<'a>(
    entries: &'a [ReviewIndexEntry],
    id: &str,
) -> Result<&'a ReviewIndexEntry, Box<dyn std::error::Error>> {
    if id == "latest" {
        return latest_review_index_entry(entries)
            .ok_or_else(|| "review report not found: latest".into());
    }

    let matches = entries.iter().filter(|entry| entry.id == id).collect::<Vec<_>>();
    if let [entry] = matches.as_slice() {
        return Ok(*entry);
    }

    let prefix_matches =
        entries.iter().filter(|entry| entry.id.starts_with(id)).collect::<Vec<_>>();
    match prefix_matches.as_slice() {
        [entry] => Ok(*entry),
        [] => Err(format!("review report not found: {id}").into()),
        _ => Err(format!("review id prefix is ambiguous: {id}").into()),
    }
}

fn latest_review_index_entry(entries: &[ReviewIndexEntry]) -> Option<&ReviewIndexEntry> {
    entries.iter().max_by(|left, right| {
        left.created_at_epoch_seconds
            .cmp(&right.created_at_epoch_seconds)
            .then_with(|| left.id.cmp(&right.id))
    })
}

fn resolve_review_markdown_path(workspace_root: &Path, entry: &ReviewIndexEntry) -> PathBuf {
    let stored = PathBuf::from(entry.markdown_path.replace('/', std::path::MAIN_SEPARATOR_STR));
    if stored.exists() {
        return stored;
    }
    workspace_root.join(".sego").join("reviews").join(format!("{}.md", entry.id))
}

fn load_review_report_from_index_entry(
    workspace_root: &Path,
    entry: &ReviewIndexEntry,
) -> Result<ReviewReport, Box<dyn std::error::Error>> {
    let json_path = resolve_review_json_path(workspace_root, entry);
    let json = fs::read_to_string(json_path)?;
    let artifact: serde_json::Value = serde_json::from_str(json.trim_start_matches('\u{feff}'))?;
    let findings = artifact
        .get("findings")
        .cloned()
        .map(serde_json::from_value)
        .transpose()?
        .unwrap_or_default();
    let raw_text = artifact
        .get("raw_text")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let parse_status = artifact
        .get("parse_status")
        .cloned()
        .map(serde_json::from_value)
        .transpose()?
        .unwrap_or(runtime::ReviewParseStatus::FallbackRawText);

    // C20.6-A R2: restore parse diagnostics from persisted artifact JSON.
    let parse_error = artifact
        .get("parse_error")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let parse_repair =
        artifact.get("parse_repair").and_then(serde_json::Value::as_str).map(String::from);

    Ok(ReviewReport { findings, raw_text, parse_status, parse_error, parse_repair })
}

fn resolve_review_json_path(workspace_root: &Path, entry: &ReviewIndexEntry) -> PathBuf {
    let stored = PathBuf::from(entry.json_path.replace('/', std::path::MAIN_SEPARATOR_STR));
    if stored.exists() {
        return stored;
    }
    workspace_root.join(".sego").join("reviews").join(format!("{}.json", entry.id))
}

fn print_clean_review_scope(target: &ReviewTarget) {
    println!(
        "Review\n  Result           clean review scope\n  Scope            {}\n  Detail           no current changes",
        target.scope.label()
    );
}

fn run_code_verify_cli(scope: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let scope = VerificationScope::parse(scope)?;
    let cwd = env::current_dir()?;
    let plan = build_verification_plan(&cwd, scope);

    println!("Verify");
    println!("  Scope            {}", scope.label());
    match &plan.status {
        VerificationPlanStatus::NoPlan { reason } => {
            println!("  Result           no verification plan");
            println!("  Reason           {reason}");
            return Ok(());
        }
        VerificationPlanStatus::Ready => {
            println!("  Plan             {} command(s)", plan.commands.len());
        }
    }

    let mut failed = false;
    for command in &plan.commands {
        let result = run_verification_command(&cwd, command)?;
        if !result.success {
            failed = true;
        }
        print_verification_result(&result);
    }

    println!("  Result           {}", if failed { "failed" } else { "passed" });

    if failed {
        return Err("verification failed".into());
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct VerificationCommandResult {
    label: String,
    command: String,
    working_dir: String,
    exit_code: Option<i32>,
    success: bool,
    duration: Duration,
    stdout: String,
    stderr: String,
}

fn run_verification_command(
    cwd: &Path,
    command: &VerificationCommand,
) -> Result<VerificationCommandResult, Box<dyn std::error::Error>> {
    let started_at = Instant::now();
    let working_dir =
        if command.working_dir == "." { cwd.to_path_buf() } else { cwd.join(&command.working_dir) };
    let output = std::process::Command::new(&command.program)
        .args(&command.args)
        .current_dir(&working_dir)
        .output()?;

    Ok(VerificationCommandResult {
        label: command.label.clone(),
        command: command.display_command(),
        working_dir: command.working_dir.clone(),
        exit_code: output.status.code(),
        success: output.status.success(),
        duration: started_at.elapsed(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn print_verification_result(result: &VerificationCommandResult) {
    println!();
    println!("  Command          {}", result.label);
    println!("  Run              {}", result.command);
    println!("  Working dir      {}", result.working_dir);
    println!(
        "  Exit             {}",
        result.exit_code.map_or_else(|| "signal".to_string(), |code| code.to_string())
    );
    println!("  Duration         {}ms", result.duration.as_millis());
    println!("  Status           {}", if result.success { "passed" } else { "failed" });
    let stdout = summarize_command_output(&result.stdout, result.success);
    if !stdout.is_empty() {
        println!("  Stdout           {stdout}");
    }
    let stderr = summarize_command_output(&result.stderr, result.success);
    if !stderr.is_empty() {
        println!("  Stderr           {stderr}");
    }
}

fn summarize_command_output(value: &str, success: bool) -> String {
    let lines = value.lines().map(str::trim).filter(|line| !line.is_empty()).collect::<Vec<_>>();

    let compact = if success {
        lines.iter().take(6).copied().collect::<Vec<_>>().join(" | ")
    } else {
        let mut selected = Vec::new();
        for line in lines.iter().filter(|line| is_diagnostic_output_line(line)).take(12) {
            selected.push(*line);
        }
        let tail_start = lines.len().saturating_sub(12);
        for line in &lines[tail_start..] {
            if !selected.contains(line) {
                selected.push(*line);
            }
        }
        selected.join(" | ")
    };

    let max_chars = if success { 500 } else { 1_200 };
    if compact.chars().count() <= max_chars {
        compact
    } else {
        format!("{}...", compact.chars().take(max_chars).collect::<String>())
    }
}

fn is_diagnostic_output_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains("failures:")
        || lower.contains("failed")
        || lower.contains("panicked")
        || lower.contains("assertion failed")
        || lower.contains("error:")
        || lower.contains("test result: failed")
        || lower.starts_with("---- ")
        || lower.starts_with("thread '")
}

fn run_git_diff_command_in(
    cwd: &Path,
    args: &[&str],
) -> Result<String, Box<dyn std::error::Error>> {
    let output = std::process::Command::new("git").args(args).current_dir(cwd).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("git {} failed: {stderr}", args.join(" ")).into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn is_git_worktree(cwd: &Path) -> bool {
    std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(cwd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn non_git_review_error(cwd: &Path) -> String {
    // C20.5-B: structured recovery hint.
    format!(
        "Review\n  Result           failed\n  Reason           no Git repository found\n  Workspace        {}\n  Next step        Run `sego review --full <path>` for non-Git directories, or use /dir.",
        cwd.display()
    )
}

fn render_teleport_report(target: &str) -> Result<String, Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;

    let file_list = Command::new("rg").args(["--files"]).current_dir(&cwd).output()?;
    let file_matches = if file_list.status.success() {
        String::from_utf8(file_list.stdout)?
            .lines()
            .filter(|line| line.contains(target))
            .take(10)
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let content_output = Command::new("rg")
        .args(["-n", "-S", "--color", "never", target, "."])
        .current_dir(&cwd)
        .output()?;

    let mut lines = vec![
        "Teleport".to_string(),
        format!("  Target           {target}"),
        "  Action           search workspace files and content for the target".to_string(),
    ];
    if !file_matches.is_empty() {
        lines.push(String::new());
        lines.push("File matches".to_string());
        lines.extend(file_matches.into_iter().map(|path| format!("  {path}")));
    }

    if content_output.status.success() {
        let matches = String::from_utf8(content_output.stdout)?;
        if !matches.trim().is_empty() {
            lines.push(String::new());
            lines.push("Content matches".to_string());
            lines.push(truncate_for_prompt(&matches, 4_000));
        }
    }

    if lines.len() == 1 {
        lines.push("  Result           no matches found".to_string());
    }

    Ok(lines.join("\n"))
}

fn render_last_tool_debug_report(session: &Session) -> Result<String, Box<dyn std::error::Error>> {
    let last_tool_use = session
        .messages
        .iter()
        .rev()
        .find_map(|message| {
            message.blocks.iter().rev().find_map(|block| match block {
                ContentBlock::ToolUse { id, name, input } => {
                    Some((id.clone(), name.clone(), input.clone()))
                }
                _ => None,
            })
        })
        .ok_or_else(|| "no prior tool call found in session".to_string())?;

    let tool_result = session.messages.iter().rev().find_map(|message| {
        message.blocks.iter().rev().find_map(|block| match block {
            ContentBlock::ToolResult { tool_use_id, tool_name, output, is_error }
                if tool_use_id == &last_tool_use.0 =>
            {
                Some((tool_name.clone(), output.clone(), *is_error))
            }
            _ => None,
        })
    });

    let mut lines = vec![
        "Debug tool call".to_string(),
        "  Action           inspect the last recorded tool call and its result".to_string(),
        format!("  Tool id          {}", last_tool_use.0),
        format!("  Tool name        {}", last_tool_use.1),
        "  Input".to_string(),
        indent_block(&last_tool_use.2, 4),
    ];

    match tool_result {
        Some((tool_name, output, is_error)) => {
            lines.push("  Result".to_string());
            lines.push(format!("    name           {tool_name}"));
            lines.push(format!("    status         {}", if is_error { "error" } else { "ok" }));
            lines.push(indent_block(&output, 4));
        }
        None => lines.push("  Result           missing tool result".to_string()),
    }

    Ok(lines.join("\n"))
}

fn indent_block(value: &str, spaces: usize) -> String {
    let indent = " ".repeat(spaces);
    value.lines().map(|line| format!("{indent}{line}")).collect::<Vec<_>>().join("\n")
}

fn validate_no_args(
    command_name: &str,
    args: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(args) = args.map(str::trim).filter(|value| !value.is_empty()) {
        return Err(format!(
            "{command_name} does not accept arguments. Received: {args}\nUsage: {command_name}"
        )
        .into());
    }
    Ok(())
}

fn format_bughunter_report(scope: Option<&str>) -> String {
    format!(
        "Bughunter
  Scope            {}
  Action           inspect the selected code for likely bugs and correctness issues
  Output           findings should include file paths, severity, and suggested fixes",
        scope.unwrap_or("the current repository")
    )
}

/// What `/bughunter` asks the model to do.
///
/// Separate from the report header on purpose: the header describes the command
/// to the user, this is the instruction sent to the model, and keeping them apart
/// means the instruction can be asserted without a model in the loop.
///
/// Two sentences here are load-bearing, and both came out of running the command
/// against a real model rather than a fixture:
///
/// - **Use the file-reading tools rather than shell commands.** The first live
///   run was in read-only mode, where the shell is refused: the model reached for
///   `cat` through bash, was denied before it ran anything, and correctly replied
///   that it would not guess at a file it had not read. The wiring was right and
///   the command still could not do its job. Reading is exactly what read-only
///   mode permits, so the instruction now points at the tool that works there.
/// - **If you find none, say so plainly.** A model told to hunt bugs will produce
///   some; allowing "nothing found" as an answer is what keeps a clean result
///   readable as "nothing found" rather than as a report that ran out of room -
///   the same distinction the review path draws when it refuses to render a failed
///   parse as `Findings 0`.
fn bughunter_prompt(scope: Option<&str>) -> String {
    format!(
        "Inspect {} for likely bugs and correctness issues. Use the file-reading tools to read \
         what you need rather than shell commands, because the shell is not available in every \
         permission mode. For each finding give the file path, the severity, what goes wrong, \
         and a suggested fix. Report only defects you can point at in the code; if you find \
         none, say so plainly instead of listing speculative concerns.",
        scope.unwrap_or("the current repository")
    )
}

fn format_ultraplan_report(task: Option<&str>) -> String {
    format!(
        "Ultraplan
  Task             {}
  Action           break work into a multi-step execution plan
  Output           plan should cover goals, risks, sequencing, verification, and rollback",
        task.unwrap_or("the current repo work")
    )
}

fn format_pr_report(branch: &str, context: Option<&str>) -> String {
    format!(
        "PR
  Branch           {branch}
  Context          {}
  Action           draft or create a pull request for the current branch
  Output           title and markdown body suitable for GitHub",
        context.unwrap_or("none")
    )
}

fn format_issue_report(context: Option<&str>) -> String {
    format!(
        "Issue
  Context          {}
  Action           draft or create a GitHub issue from the current context
  Output           title and markdown body suitable for GitHub",
        context.unwrap_or("none")
    )
}

fn git_output(args: &[&str]) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("git").args(args).current_dir(env::current_dir()?).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("git {} failed: {stderr}", args.join(" ")).into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn git_status_ok(args: &[&str]) -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("git").args(args).current_dir(env::current_dir()?).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("git {} failed: {stderr}", args.join(" ")).into());
    }
    Ok(())
}

fn command_exists(name: &str) -> bool {
    Command::new("which").arg(name).output().is_ok_and(|output| output.status.success())
}

fn write_temp_text_file(
    filename: &str,
    contents: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = env::temp_dir().join(filename);
    fs::write(&path, contents)?;
    Ok(path)
}

fn recent_user_context(session: &Session, limit: usize) -> String {
    let requests = session
        .messages
        .iter()
        .filter(|message| message.role == MessageRole::User)
        .filter_map(|message| {
            message.blocks.iter().find_map(|block| match block {
                ContentBlock::Text { text } => Some(text.trim().to_string()),
                _ => None,
            })
        })
        .rev()
        .take(limit)
        .collect::<Vec<_>>();

    if requests.is_empty() {
        "<no prior user messages>".to_string()
    } else {
        requests
            .into_iter()
            .rev()
            .enumerate()
            .map(|(index, text)| format!("{}. {}", index + 1, text))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn truncate_for_prompt(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        value.trim().to_string()
    } else {
        let truncated = value.chars().take(limit).collect::<String>();
        format!("{}\n…[truncated]", truncated.trim_end())
    }
}

fn sanitize_generated_message(value: &str) -> String {
    value.trim().trim_matches('`').trim().replace("\r\n", "\n")
}

fn parse_titled_body(value: &str) -> Option<(String, String)> {
    let normalized = sanitize_generated_message(value);
    let title = normalized.lines().find_map(|line| line.strip_prefix("TITLE:").map(str::trim))?;
    let body_start = normalized.find("BODY:")?;
    let body = normalized[body_start + "BODY:".len()..].trim();
    Some((title.to_string(), body.to_string()))
}

fn render_version_report() -> String {
    let git_sha = GIT_SHA.unwrap_or("unknown");
    let target = BUILD_TARGET.unwrap_or("unknown");
    let date = default_date();
    format!(
        "Sego Agent\n  Version          {VERSION}\n  Git SHA          {git_sha}\n  Target           {target}\n  Build date       {date}"
    )
}

fn render_export_text(session: &Session) -> String {
    let mut lines = vec!["# Conversation Export".to_string(), String::new()];
    for (index, message) in session.messages.iter().enumerate() {
        let role = match message.role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool => "tool",
        };
        lines.push(format!("## {}. {role}", index + 1));
        for block in &message.blocks {
            match block {
                ContentBlock::Text { text } => lines.push(text.clone()),
                ContentBlock::ToolUse { id, name, input } => {
                    lines.push(format!("[tool_use id={id} name={name}] {input}"));
                }
                ContentBlock::ToolResult { tool_use_id, tool_name, output, is_error } => {
                    lines.push(format!(
                        "[tool_result id={tool_use_id} name={tool_name} error={is_error}] {output}"
                    ));
                }
                ContentBlock::Thinking { .. } => {}
            }
        }
        lines.push(String::new());
    }
    lines.join("\n")
}

fn latest_assistant_text(session: &Session) -> Option<String> {
    session.messages.iter().rev().find_map(|message| {
        if message.role != MessageRole::Assistant {
            return None;
        }

        let text = message
            .blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.trim()),
                _ => None,
            })
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        (!text.is_empty()).then_some(text)
    })
}

fn default_export_filename(session: &Session) -> String {
    let stem = session
        .messages
        .iter()
        .find_map(|message| match message.role {
            MessageRole::User => message.blocks.iter().find_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            }),
            _ => None,
        })
        .map_or("conversation", |text| text.lines().next().unwrap_or("conversation"))
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .join("-");
    let fallback = if stem.is_empty() { "conversation" } else { &stem };
    format!("{fallback}.txt")
}

fn resolve_export_path(
    requested_path: Option<&str>,
    session: &Session,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let file_name =
        requested_path.map_or_else(|| default_export_filename(session), ToOwned::to_owned);
    let final_name =
        if Path::new(&file_name).extension().is_some_and(|ext| ext.eq_ignore_ascii_case("txt")) {
            file_name
        } else {
            format!("{file_name}.txt")
        };
    Ok(cwd.join(final_name))
}

fn resolve_direct_export_path(
    requested_path: Option<&str>,
    default_name: impl FnOnce() -> String,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let file_name = requested_path.map_or_else(default_name, |path| path.trim().to_string());
    let candidate = PathBuf::from(file_name);
    let candidate = if candidate.is_absolute() { candidate } else { cwd.join(candidate) };
    if candidate.extension().is_some() {
        Ok(candidate)
    } else {
        Ok(candidate.with_extension("md"))
    }
}

fn build_system_prompt() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    Ok(load_system_prompt(env::current_dir()?, default_date(), env::consts::OS, "unknown")?)
}

fn build_runtime_plugin_state() -> Result<RuntimePluginState, Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let loader = ConfigLoader::default_for(&cwd);
    let runtime_config = loader.load()?;
    build_runtime_plugin_state_with_loader(&cwd, &loader, &runtime_config)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InternalPromptProgressState {
    command_label: &'static str,
    task_label: String,
    step: usize,
    phase: String,
    detail: Option<String>,
    saw_final_text: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InternalPromptProgressEvent {
    Started,
    Update,
    Heartbeat,
    Complete,
    Failed,
}

#[derive(Debug)]
struct InternalPromptProgressShared {
    state: Mutex<InternalPromptProgressState>,
    output_lock: Mutex<()>,
    started_at: Instant,
}

#[derive(Debug, Clone)]
struct InternalPromptProgressReporter {
    shared: Arc<InternalPromptProgressShared>,
}

#[derive(Debug)]
struct InternalPromptProgressRun {
    reporter: InternalPromptProgressReporter,
    heartbeat_stop: Option<mpsc::Sender<()>>,
    heartbeat_handle: Option<thread::JoinHandle<()>>,
}

impl InternalPromptProgressReporter {
    fn ultraplan(task: &str) -> Self {
        Self {
            shared: Arc::new(InternalPromptProgressShared {
                state: Mutex::new(InternalPromptProgressState {
                    command_label: "Ultraplan",
                    task_label: task.to_string(),
                    step: 0,
                    phase: "planning started".to_string(),
                    detail: Some(format!("task: {task}")),
                    saw_final_text: false,
                }),
                output_lock: Mutex::new(()),
                started_at: Instant::now(),
            }),
        }
    }

    fn emit(&self, event: InternalPromptProgressEvent, error: Option<&str>) {
        let snapshot = self.snapshot();
        let line = format_internal_prompt_progress_line(event, &snapshot, self.elapsed(), error);
        self.write_line(&line);
    }

    fn mark_model_phase(&self) {
        let snapshot = {
            let mut state =
                self.shared.state.lock().expect("internal prompt progress state poisoned");
            state.step += 1;
            state.phase = if state.step == 1 {
                "analyzing request".to_string()
            } else {
                "reviewing findings".to_string()
            };
            state.detail = Some(format!("task: {}", state.task_label));
            state.clone()
        };
        self.write_line(&format_internal_prompt_progress_line(
            InternalPromptProgressEvent::Update,
            &snapshot,
            self.elapsed(),
            None,
        ));
    }

    fn mark_tool_phase(&self, name: &str, input: &str) {
        let detail = describe_tool_progress(name, input);
        let snapshot = {
            let mut state =
                self.shared.state.lock().expect("internal prompt progress state poisoned");
            state.step += 1;
            state.phase = format!("running {name}");
            state.detail = Some(detail);
            state.clone()
        };
        self.write_line(&format_internal_prompt_progress_line(
            InternalPromptProgressEvent::Update,
            &snapshot,
            self.elapsed(),
            None,
        ));
    }

    fn mark_text_phase(&self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let detail = truncate_for_summary(first_visible_line(trimmed), 120);
        let snapshot = {
            let mut state =
                self.shared.state.lock().expect("internal prompt progress state poisoned");
            if state.saw_final_text {
                return;
            }
            state.saw_final_text = true;
            state.step += 1;
            state.phase = "drafting final plan".to_string();
            state.detail = (!detail.is_empty()).then_some(detail);
            state.clone()
        };
        self.write_line(&format_internal_prompt_progress_line(
            InternalPromptProgressEvent::Update,
            &snapshot,
            self.elapsed(),
            None,
        ));
    }

    fn emit_heartbeat(&self) {
        let snapshot = self.snapshot();
        self.write_line(&format_internal_prompt_progress_line(
            InternalPromptProgressEvent::Heartbeat,
            &snapshot,
            self.elapsed(),
            None,
        ));
    }

    fn snapshot(&self) -> InternalPromptProgressState {
        self.shared.state.lock().expect("internal prompt progress state poisoned").clone()
    }

    fn elapsed(&self) -> Duration {
        self.shared.started_at.elapsed()
    }

    fn write_line(&self, line: &str) {
        let _guard =
            self.shared.output_lock.lock().expect("internal prompt progress output lock poisoned");
        let mut stdout = io::stdout();
        let _ = writeln!(stdout, "{line}");
        let _ = stdout.flush();
    }
}

impl InternalPromptProgressRun {
    fn start_ultraplan(task: &str) -> Self {
        let reporter = InternalPromptProgressReporter::ultraplan(task);
        reporter.emit(InternalPromptProgressEvent::Started, None);

        let (heartbeat_stop, heartbeat_rx) = mpsc::channel();
        let heartbeat_reporter = reporter.clone();
        let heartbeat_handle = thread::spawn(move || loop {
            match heartbeat_rx.recv_timeout(INTERNAL_PROGRESS_HEARTBEAT_INTERVAL) {
                Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => heartbeat_reporter.emit_heartbeat(),
            }
        });

        Self {
            reporter,
            heartbeat_stop: Some(heartbeat_stop),
            heartbeat_handle: Some(heartbeat_handle),
        }
    }

    fn reporter(&self) -> InternalPromptProgressReporter {
        self.reporter.clone()
    }

    fn finish_success(&mut self) {
        self.stop_heartbeat();
        self.reporter.emit(InternalPromptProgressEvent::Complete, None);
    }

    fn finish_failure(&mut self, error: &str) {
        self.stop_heartbeat();
        self.reporter.emit(InternalPromptProgressEvent::Failed, Some(error));
    }

    fn stop_heartbeat(&mut self) {
        if let Some(sender) = self.heartbeat_stop.take() {
            let _ = sender.send(());
        }
        if let Some(handle) = self.heartbeat_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for InternalPromptProgressRun {
    fn drop(&mut self) {
        self.stop_heartbeat();
    }
}

fn format_internal_prompt_progress_line(
    event: InternalPromptProgressEvent,
    snapshot: &InternalPromptProgressState,
    elapsed: Duration,
    error: Option<&str>,
) -> String {
    let elapsed_seconds = elapsed.as_secs();
    let step_label = if snapshot.step == 0 {
        "current step pending".to_string()
    } else {
        format!("current step {}", snapshot.step)
    };
    let mut status_bits = vec![step_label, format!("phase {}", snapshot.phase)];
    if let Some(detail) = snapshot.detail.as_deref().filter(|detail| !detail.is_empty()) {
        status_bits.push(detail.to_string());
    }
    let status = status_bits.join(" · ");
    match event {
        InternalPromptProgressEvent::Started => {
            format!("🧭 {} status · planning started · {status}", snapshot.command_label)
        }
        InternalPromptProgressEvent::Update => {
            format!("… {} status · {status}", snapshot.command_label)
        }
        InternalPromptProgressEvent::Heartbeat => format!(
            "… {} heartbeat · {elapsed_seconds}s elapsed · {status}",
            snapshot.command_label
        ),
        InternalPromptProgressEvent::Complete => format!(
            "✔ {} status · completed · {elapsed_seconds}s elapsed · {} steps total",
            snapshot.command_label, snapshot.step
        ),
        InternalPromptProgressEvent::Failed => format!(
            "✘ {} status · failed · {elapsed_seconds}s elapsed · {}",
            snapshot.command_label,
            error.unwrap_or("unknown error")
        ),
    }
}

fn describe_tool_progress(name: &str, input: &str) -> String {
    let parsed: serde_json::Value =
        serde_json::from_str(input).unwrap_or(serde_json::Value::String(input.to_string()));
    match name {
        "bash" | "Bash" => {
            let command =
                parsed.get("command").and_then(|value| value.as_str()).unwrap_or_default();
            if command.is_empty() {
                "running shell command".to_string()
            } else {
                format!("command {}", truncate_for_summary(command.trim(), 100))
            }
        }
        "read_file" | "Read" => format!("reading {}", extract_tool_path(&parsed)),
        "write_file" | "Write" => format!("writing {}", extract_tool_path(&parsed)),
        "edit_file" | "Edit" => format!("editing {}", extract_tool_path(&parsed)),
        "glob_search" | "Glob" => {
            let pattern = parsed.get("pattern").and_then(|value| value.as_str()).unwrap_or("?");
            let scope = parsed.get("path").and_then(|value| value.as_str()).unwrap_or(".");
            format!("glob `{pattern}` in {scope}")
        }
        "grep_search" | "Grep" => {
            let pattern = parsed.get("pattern").and_then(|value| value.as_str()).unwrap_or("?");
            let scope = parsed.get("path").and_then(|value| value.as_str()).unwrap_or(".");
            format!("grep `{pattern}` in {scope}")
        }
        "web_search" | "WebSearch" => {
            parsed.get("query").and_then(|value| value.as_str()).map_or_else(
                || "running web search".to_string(),
                |query| format!("query {}", truncate_for_summary(query, 100)),
            )
        }
        _ => {
            let summary = summarize_tool_payload(input);
            if summary.is_empty() {
                format!("running {name}")
            } else {
                format!("{name}: {summary}")
            }
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
#[allow(clippy::too_many_arguments)]
fn build_runtime(
    session: Session,
    session_id: &str,
    model: String,
    system_prompt: Vec<String>,
    enable_tools: bool,
    emit_output: bool,
    allowed_tools: Option<AllowedToolSet>,
    permission_mode: PermissionMode,
    progress_reporter: Option<InternalPromptProgressReporter>,
) -> Result<BuiltRuntime, Box<dyn std::error::Error>> {
    let runtime_plugin_state = build_runtime_plugin_state()?;
    build_runtime_with_plugin_state(
        session,
        session_id,
        model,
        system_prompt,
        enable_tools,
        emit_output,
        allowed_tools,
        permission_mode,
        progress_reporter,
        runtime_plugin_state,
    )
}

#[allow(clippy::needless_pass_by_value)]
#[allow(clippy::too_many_arguments)]
fn build_runtime_with_plugin_state(
    session: Session,
    session_id: &str,
    model: String,
    system_prompt: Vec<String>,
    enable_tools: bool,
    emit_output: bool,
    allowed_tools: Option<AllowedToolSet>,
    permission_mode: PermissionMode,
    progress_reporter: Option<InternalPromptProgressReporter>,
    runtime_plugin_state: RuntimePluginState,
) -> Result<BuiltRuntime, Box<dyn std::error::Error>> {
    let RuntimePluginState { feature_config, tool_registry, plugin_registry, mcp_state } =
        runtime_plugin_state;
    plugin_registry.initialize()?;
    let policy = permission_policy(permission_mode, &feature_config, &tool_registry)
        .map_err(std::io::Error::other)?;
    let mut runtime = ConversationRuntime::new_with_features(
        session,
        SegoRuntimeClient::new(
            session_id,
            model,
            enable_tools,
            emit_output,
            allowed_tools.clone(),
            tool_registry.clone(),
            progress_reporter,
        )?,
        CliToolExecutor::new(
            allowed_tools.clone(),
            emit_output,
            tool_registry.clone(),
            mcp_state.clone(),
        ),
        policy,
        system_prompt,
        &feature_config,
    );
    if emit_output {
        runtime = runtime.with_hook_progress_reporter(Box::new(CliHookProgressReporter));
    }
    Ok(BuiltRuntime::new(runtime, plugin_registry, mcp_state))
}

struct CliHookProgressReporter;

impl runtime::HookProgressReporter for CliHookProgressReporter {
    fn on_event(&mut self, event: &runtime::HookProgressEvent) {
        match event {
            runtime::HookProgressEvent::Started { event, tool_name, command } => {
                eprintln!(
                    "[hook {event_name}] {tool_name}: {command}",
                    event_name = event.as_str()
                );
            }
            runtime::HookProgressEvent::Completed { event, tool_name, command } => eprintln!(
                "[hook done {event_name}] {tool_name}: {command}",
                event_name = event.as_str()
            ),
            runtime::HookProgressEvent::Cancelled { event, tool_name, command } => eprintln!(
                "[hook cancelled {event_name}] {tool_name}: {command}",
                event_name = event.as_str()
            ),
        }
    }
}

/// Fail-closed permission prompter for machine/sidecar mode (D-IDE-1).
/// Never writes to stdout, never reads from stdin. Always denies tool calls
/// that require interactive approval. Used when `machine_output == true`.
struct MachinePermissionPrompter;

impl runtime::PermissionPrompter for MachinePermissionPrompter {
    fn decide(
        &mut self,
        _request: &runtime::PermissionRequest,
    ) -> runtime::PermissionPromptDecision {
        runtime::PermissionPromptDecision::Deny {
            reason: "machine mode: interactive approval not available".to_string(),
        }
    }
}

struct CliPermissionPrompter {
    current_mode: PermissionMode,
}

impl CliPermissionPrompter {
    fn new(current_mode: PermissionMode) -> Self {
        Self { current_mode }
    }
}

impl runtime::PermissionPrompter for CliPermissionPrompter {
    fn decide(
        &mut self,
        request: &runtime::PermissionRequest,
    ) -> runtime::PermissionPromptDecision {
        println!();
        println!("Permission approval required");
        println!("  Tool             {}", request.tool_name);
        println!("  Current mode     {}", self.current_mode.as_str());
        println!("  Required mode    {}", request.required_mode.as_str());
        if let Some(reason) = &request.reason {
            println!("  Reason           {reason}");
        }
        println!("  Input            {}", request.input);
        print!("Approve this tool call? [y/N]: ");
        let _ = io::stdout().flush();

        let mut response = String::new();
        match io::stdin().read_line(&mut response) {
            Ok(_) => {
                let normalized = response.trim().to_ascii_lowercase();
                if matches!(normalized.as_str(), "y" | "yes") {
                    runtime::PermissionPromptDecision::Allow
                } else {
                    runtime::PermissionPromptDecision::Deny {
                        reason: format!(
                            "tool '{}' denied by user approval prompt",
                            request.tool_name
                        ),
                    }
                }
            }
            Err(error) => runtime::PermissionPromptDecision::Deny {
                reason: format!("permission approval failed: {error}"),
            },
        }
    }
}

struct SegoRuntimeClient {
    runtime: tokio::runtime::Runtime,
    client: ProviderClient,
    model: String,
    enable_tools: bool,
    emit_output: bool,
    allowed_tools: Option<AllowedToolSet>,
    tool_registry: GlobalToolRegistry,
    progress_reporter: Option<InternalPromptProgressReporter>,
}

impl SegoRuntimeClient {
    fn new(
        session_id: &str,
        model: String,
        enable_tools: bool,
        emit_output: bool,
        allowed_tools: Option<AllowedToolSet>,
        tool_registry: GlobalToolRegistry,
        progress_reporter: Option<InternalPromptProgressReporter>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let client = ProviderClient::from_model(&model)
            .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?;
        Ok(Self {
            runtime: tokio::runtime::Runtime::new()?,
            client: client.with_prompt_cache(api::PromptCache::new(session_id)),
            model,
            enable_tools,
            emit_output,
            allowed_tools,
            tool_registry,
            progress_reporter,
        })
    }
}

impl ApiClient for SegoRuntimeClient {
    #[allow(clippy::too_many_lines)]
    fn stream(&mut self, request: ApiRequest) -> Result<Vec<AssistantEvent>, RuntimeError> {
        if let Some(progress_reporter) = &self.progress_reporter {
            progress_reporter.mark_model_phase();
        }
        // Context window preflight check with automatic max_tokens reduction
        let estimated_chars: usize = request
            .messages
            .iter()
            .flat_map(|m| m.blocks.iter())
            .map(|b| match b {
                ContentBlock::Text { text } => text.len(),
                ContentBlock::ToolUse { input, .. } => input.len() + 50,
                ContentBlock::ToolResult { output, .. } => output.len() + 50,
                ContentBlock::Thinking { thinking, .. } => thinking.len(),
            })
            .sum();
        let estimated_tokens = u32::try_from(estimated_chars / 4).unwrap_or(u32::MAX);
        let context_limit = context_window_limit(&self.model);
        let requested_max = max_tokens_for_model(&self.model);
        // Auto-reduce max_tokens if estimated input + requested output exceeds context window
        let safe_max_tokens = if estimated_tokens + requested_max > context_limit {
            let reduced = context_limit.saturating_sub(estimated_tokens).max(1024);
            if self.emit_output {
                eprintln!(
                    "⚠ Context: estimated {estimated_tokens} input tokens + {requested_max} output exceeds {context_limit} limit. Reducing max_tokens to ~{reduced}."
                );
            }
            reduced
        } else if estimated_tokens + requested_max > context_limit * 90 / 100 {
            if self.emit_output {
                eprintln!(
                    "⚠ Approaching context limit: ~{} / {} tokens. Consider /compact soon.",
                    estimated_tokens + requested_max,
                    context_limit
                );
            }
            requested_max
        } else {
            requested_max
        };
        let message_request = MessageRequest {
            model: self.model.clone(),
            max_tokens: safe_max_tokens,
            messages: convert_messages(&request.messages),
            system: (!request.system_prompt.is_empty()).then(|| request.system_prompt.join("\n\n")),
            tools: self
                .enable_tools
                .then(|| filter_tool_specs(&self.tool_registry, self.allowed_tools.as_ref())),
            tool_choice: self.enable_tools.then_some(ToolChoice::Auto),
            stream: true,
        };

        self.runtime.block_on(async {
            let mut stream = self
                .client
                .stream_message(&message_request)
                .await
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            let mut stdout = io::stdout();
            let mut sink = io::sink();
            let out: &mut dyn Write = if self.emit_output { &mut stdout } else { &mut sink };
            let renderer = TerminalRenderer::new();
            let mut markdown_stream = MarkdownStreamState::default();
            let mut events = Vec::new();
            let mut pending_tool: Option<(String, String, String)> = None;
            let mut pending_thinking = String::new();
            let mut pending_thinking_signature: Option<String> = None;
            let mut saw_stop = false;

            while let Some(event) =
                stream.next_event().await.map_err(|error| RuntimeError::new(error.to_string()))?
            {
                match event {
                    ApiStreamEvent::MessageStart(start) => {
                        for block in start.message.content {
                            push_output_block(block, out, &mut events, &mut pending_tool, true)?;
                        }
                    }
                    ApiStreamEvent::ContentBlockStart(start) => {
                        push_output_block(
                            start.content_block,
                            out,
                            &mut events,
                            &mut pending_tool,
                            true,
                        )?;
                    }
                    ApiStreamEvent::ContentBlockDelta(delta) => match delta.delta {
                        ContentBlockDelta::TextDelta { text } => {
                            if !text.is_empty() {
                                if let Some(progress_reporter) = &self.progress_reporter {
                                    progress_reporter.mark_text_phase(&text);
                                }
                                if let Some(rendered) = markdown_stream.push(&renderer, &text) {
                                    write!(out, "{rendered}")
                                        .and_then(|()| out.flush())
                                        .map_err(|error| RuntimeError::new(error.to_string()))?;
                                }
                                events.push(AssistantEvent::TextDelta(text));
                            }
                        }
                        ContentBlockDelta::InputJsonDelta { partial_json } => {
                            if let Some((_, _, input)) = &mut pending_tool {
                                input.push_str(&partial_json);
                            }
                        }
                        ContentBlockDelta::ThinkingDelta { thinking } => {
                            pending_thinking.push_str(&thinking);
                        }
                        ContentBlockDelta::SignatureDelta { signature } => {
                            pending_thinking_signature = Some(signature);
                        }
                    },
                    ApiStreamEvent::ContentBlockStop(_) => {
                        if let Some(rendered) = markdown_stream.flush(&renderer) {
                            write!(out, "{rendered}")
                                .and_then(|()| out.flush())
                                .map_err(|error| RuntimeError::new(error.to_string()))?;
                        }
                        if let Some((id, name, input)) = pending_tool.take() {
                            if let Some(progress_reporter) = &self.progress_reporter {
                                progress_reporter.mark_tool_phase(&name, &input);
                            }
                            // Display tool call now that input is fully accumulated
                            writeln!(out, "\n{}", format_tool_call_start(&name, &input))
                                .and_then(|()| out.flush())
                                .map_err(|error| RuntimeError::new(error.to_string()))?;
                            events.push(AssistantEvent::ToolUse { id, name, input });
                        }
                    }
                    ApiStreamEvent::MessageDelta(delta) => {
                        events.push(AssistantEvent::Usage(delta.usage.token_usage()));
                    }
                    ApiStreamEvent::MessageStop(_) => {
                        saw_stop = true;
                        if let Some(rendered) = markdown_stream.flush(&renderer) {
                            write!(out, "{rendered}")
                                .and_then(|()| out.flush())
                                .map_err(|error| RuntimeError::new(error.to_string()))?;
                        }
                        if !pending_thinking.is_empty() {
                            events.push(AssistantEvent::Thinking {
                                thinking: std::mem::take(&mut pending_thinking),
                                signature: pending_thinking_signature.take(),
                            });
                        }
                        events.push(AssistantEvent::MessageStop);
                    }
                }
            }

            push_prompt_cache_record(&self.client, &mut events);

            if !pending_thinking.is_empty() {
                events.push(AssistantEvent::Thinking {
                    thinking: std::mem::take(&mut pending_thinking),
                    signature: pending_thinking_signature.take(),
                });
            }

            if !saw_stop
                && events.iter().any(|event| {
                    matches!(event, AssistantEvent::TextDelta(text) if !text.is_empty())
                        || matches!(event, AssistantEvent::ToolUse { .. })
                })
            {
                events.push(AssistantEvent::MessageStop);
            }

            if events.iter().any(|event| matches!(event, AssistantEvent::MessageStop)) {
                return Ok(events);
            }

            let response = self
                .client
                .send_message(&MessageRequest { stream: false, ..message_request.clone() })
                .await
                .map_err(|error| RuntimeError::new(error.to_string()))?;
            let mut events = response_to_events(response, out)?;
            push_prompt_cache_record(&self.client, &mut events);
            Ok(events)
        })
    }
}

fn final_assistant_text(summary: &runtime::TurnSummary) -> String {
    summary
        .assistant_messages
        .last()
        .map(|message| {
            message
                .blocks
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

/// What a completed turn observed about itself, for the review artifact's egress
/// record (`SEG-ADR-004`).
///
/// Every value is taken from the run rather than from configuration: the request
/// count is the loop's own iteration count, the tool names are the tools that
/// actually returned, and the token count is what the provider reported. A review
/// that ran against a locally served model while the configuration still named a
/// remote one must not be recorded as having sent nothing anywhere, and the only
/// way to be sure of that is to not look at the configuration at all.
fn observed_turn(summary: &runtime::TurnSummary) -> runtime::code_review::ReviewEgressObservation {
    let usage = summary.usage;
    // An all-zero usage sample means the provider reported nothing, not that the
    // review consumed nothing: the prompt alone is never zero tokens. Writing `0`
    // would turn a gap into a claim, so the gap is carried as one.
    let reported_nothing = usage.input_tokens == 0
        && usage.output_tokens == 0
        && usage.cache_creation_input_tokens == 0
        && usage.cache_read_input_tokens == 0;
    runtime::code_review::ReviewEgressObservation {
        provider_calls: summary.iterations as u64,
        input_tokens: if reported_nothing { None } else { Some(u64::from(usage.input_tokens)) },
        tool_names: summary
            .tool_results
            .iter()
            .flat_map(|message| message.blocks.iter())
            .filter_map(|block| match block {
                ContentBlock::ToolResult { tool_name, .. } => Some(tool_name.clone()),
                _ => None,
            })
            .collect(),
    }
}

fn collect_tool_uses(summary: &runtime::TurnSummary) -> Vec<serde_json::Value> {
    summary
        .assistant_messages
        .iter()
        .flat_map(|message| message.blocks.iter())
        .filter_map(|block| match block {
            ContentBlock::ToolUse { id, name, input } => Some(json!({
                "id": id,
                "name": name,
                "input": input,
            })),
            _ => None,
        })
        .collect()
}

fn collect_tool_results(summary: &runtime::TurnSummary) -> Vec<serde_json::Value> {
    summary
        .tool_results
        .iter()
        .flat_map(|message| message.blocks.iter())
        .filter_map(|block| match block {
            ContentBlock::ToolResult { tool_use_id, tool_name, output, is_error } => Some(json!({
                "tool_use_id": tool_use_id,
                "tool_name": tool_name,
                "output": output,
                "is_error": is_error,
            })),
            _ => None,
        })
        .collect()
}

fn collect_prompt_cache_events(summary: &runtime::TurnSummary) -> Vec<serde_json::Value> {
    summary
        .prompt_cache_events
        .iter()
        .map(|event| {
            json!({
                "unexpected": event.unexpected,
                "reason": event.reason,
                "previous_cache_read_input_tokens": event.previous_cache_read_input_tokens,
                "current_cache_read_input_tokens": event.current_cache_read_input_tokens,
                "token_drop": event.token_drop,
            })
        })
        .collect()
}

fn slash_command_completion_candidates_with_sessions(
    model: &str,
    active_session_id: Option<&str>,
    recent_session_ids: Vec<String>,
) -> Vec<String> {
    let mut completions = BTreeSet::new();

    for spec in slash_command_specs() {
        completions.insert(format!("/{}", spec.name));
        for alias in spec.aliases {
            completions.insert(format!("/{alias}"));
        }
    }

    for candidate in [
        "/bughunter ",
        "/clear --confirm",
        "/config ",
        "/config env",
        "/config hooks",
        "/config model",
        "/config plugins",
        "/mcp ",
        "/mcp list",
        "/mcp show ",
        "/export ",
        "/issue ",
        "/model ",
        "/model opus",
        "/model sonnet",
        "/model haiku",
        "/permissions ",
        "/permissions read-only",
        "/permissions workspace-write",
        "/permissions danger-full-access",
        "/plugin list",
        "/plugin install ",
        "/plugin enable ",
        "/plugin disable ",
        "/plugin uninstall ",
        "/plugin update ",
        "/plugins list",
        "/pr ",
        "/resume ",
        "/session list",
        "/session switch ",
        "/session fork ",
        "/teleport ",
        "/ultraplan ",
        "/agents help",
        "/mcp help",
        "/skills help",
    ] {
        completions.insert(candidate.to_string());
    }

    if !model.trim().is_empty() {
        completions.insert(format!("/model {}", resolve_model_alias(model)));
        completions.insert(format!("/model {model}"));
    }

    if let Some(active_session_id) = active_session_id.filter(|value| !value.trim().is_empty()) {
        completions.insert(format!("/resume {active_session_id}"));
        completions.insert(format!("/session switch {active_session_id}"));
    }

    for session_id in
        recent_session_ids.into_iter().filter(|value| !value.trim().is_empty()).take(10)
    {
        completions.insert(format!("/resume {session_id}"));
        completions.insert(format!("/session switch {session_id}"));
    }

    completions.into_iter().collect()
}

fn format_tool_call_start(name: &str, input: &str) -> String {
    let parsed: serde_json::Value =
        serde_json::from_str(input).unwrap_or(serde_json::Value::String(input.to_string()));

    let detail = match name {
        "bash" | "Bash" => format_bash_call(&parsed),
        "read_file" | "Read" => {
            let path = extract_tool_path(&parsed);
            format!("\x1b[2m📄 Reading {path}…\x1b[0m")
        }
        "write_file" | "Write" => {
            let path = extract_tool_path(&parsed);
            let lines = parsed
                .get("content")
                .and_then(|value| value.as_str())
                .map_or(0, |content| content.lines().count());
            format!("\x1b[1;32m✏️ Writing {path}\x1b[0m \x1b[2m({lines} lines)\x1b[0m")
        }
        "edit_file" | "Edit" => {
            let path = extract_tool_path(&parsed);
            let old_value = parsed
                .get("old_string")
                .or_else(|| parsed.get("oldString"))
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let new_value = parsed
                .get("new_string")
                .or_else(|| parsed.get("newString"))
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            format!(
                "\x1b[1;33m📝 Editing {path}\x1b[0m{}",
                format_patch_preview(old_value, new_value)
                    .map(|preview| format!("\n{preview}"))
                    .unwrap_or_default()
            )
        }
        "glob_search" | "Glob" => format_search_start("🔎 Glob", &parsed),
        "grep_search" | "Grep" => format_search_start("🔎 Grep", &parsed),
        "web_search" | "WebSearch" => {
            parsed.get("query").and_then(|value| value.as_str()).unwrap_or("?").to_string()
        }
        _ => summarize_tool_payload(input),
    };

    let border = "─".repeat(name.len() + 8);
    format!(
        "\x1b[38;5;245m╭─ \x1b[1;36m{name}\x1b[0;38;5;245m ─╮\x1b[0m\n\x1b[38;5;245m│\x1b[0m {detail}\n\x1b[38;5;245m╰{border}╯\x1b[0m"
    )
}

fn format_tool_result(name: &str, output: &str, is_error: bool) -> String {
    let icon = if is_error { "\x1b[1;31m✗\x1b[0m" } else { "\x1b[1;32m✓\x1b[0m" };
    if is_error {
        let summary = truncate_for_summary(output.trim(), 160);
        return if summary.is_empty() {
            format!("{icon} \x1b[38;5;245m{name}\x1b[0m")
        } else {
            format!("{icon} \x1b[38;5;245m{name}\x1b[0m\n\x1b[38;5;203m{summary}\x1b[0m")
        };
    }

    let parsed: serde_json::Value =
        serde_json::from_str(output).unwrap_or(serde_json::Value::String(output.to_string()));
    match name {
        "bash" | "Bash" => format_bash_result(icon, &parsed),
        "read_file" | "Read" => format_read_result(icon, &parsed),
        "write_file" | "Write" => format_write_result(icon, &parsed),
        "edit_file" | "Edit" => format_edit_result(icon, &parsed),
        "glob_search" | "Glob" => format_glob_result(icon, &parsed),
        "grep_search" | "Grep" => format_grep_result(icon, &parsed),
        _ => format_generic_tool_result(icon, name, &parsed),
    }
}

const DISPLAY_TRUNCATION_NOTICE: &str =
    "\x1b[2m… output truncated for display; full result preserved in session.\x1b[0m";
const READ_DISPLAY_MAX_LINES: usize = 40;
const READ_DISPLAY_MAX_CHARS: usize = 3_000;
const TOOL_OUTPUT_DISPLAY_MAX_LINES: usize = 15;
const TOOL_OUTPUT_DISPLAY_MAX_CHARS: usize = 1_500;

/// Default token-saving tool set - the most-used tools, minus the long tail.
///
/// These must be the **registry's** names. They are matched by exact string in
/// `GlobalToolRegistry::definitions`, and the short aliases (`read`, `write`, …)
/// that users may type in `--allowedTools` are resolved only inside
/// `normalize_allowed_tools` - never here. The set used to be written in those
/// short aliases, so fifteen of its sixteen entries matched nothing and were
/// silently filtered out: **the effective default tool set was `bash` alone**.
/// With the default read-only permission mode refusing `bash`, that left the
/// model unable to read a file at all. `every_tool_in_the_default_set_actually_exists`
/// now asserts this list against the registry so a dead entry cannot hide again.
///
/// `task` mapped to `TaskCreate`. The registry has since split that tool into a
/// family (`TaskCreate`, `TaskGet`, `TaskList`, `TaskStop`, `TaskUpdate`,
/// `TaskOutput`, `RunTaskPacket`); `TaskCreate` is the entry point, and which of
/// the rest belong in a *lite* set is a product choice rather than something to
/// guess at here.
static LITE_TOOLS: std::sync::LazyLock<AllowedToolSet> = std::sync::LazyLock::new(|| {
    [
        "bash",
        "read_file",
        "write_file",
        "edit_file",
        "grep_search",
        "glob_search",
        "WebSearch",
        "WebFetch",
        "Agent",
        "TodoWrite",
        "TaskCreate",
        "AskUserQuestion",
        "Skill",
        "NotebookEdit",
        "EnterPlanMode",
        "ExitPlanMode",
    ]
    .iter()
    .map(std::string::ToString::to_string)
    .collect()
});

fn extract_tool_path(parsed: &serde_json::Value) -> String {
    parsed
        .get("file_path")
        .or_else(|| parsed.get("filePath"))
        .or_else(|| parsed.get("path"))
        .and_then(|value| value.as_str())
        .unwrap_or("?")
        .to_string()
}

fn format_search_start(label: &str, parsed: &serde_json::Value) -> String {
    let pattern = parsed.get("pattern").and_then(|value| value.as_str()).unwrap_or("?");
    let scope = parsed.get("path").and_then(|value| value.as_str()).unwrap_or(".");
    format!("{label} {pattern}\n\x1b[2min {scope}\x1b[0m")
}

fn format_patch_preview(old_value: &str, new_value: &str) -> Option<String> {
    if old_value.is_empty() && new_value.is_empty() {
        return None;
    }
    Some(format!(
        "\x1b[38;5;203m- {}\x1b[0m\n\x1b[38;5;70m+ {}\x1b[0m",
        truncate_for_summary(first_visible_line(old_value), 72),
        truncate_for_summary(first_visible_line(new_value), 72)
    ))
}

fn format_bash_call(parsed: &serde_json::Value) -> String {
    let command = parsed.get("command").and_then(|value| value.as_str()).unwrap_or_default();
    if command.is_empty() {
        String::new()
    } else {
        format!("\x1b[48;5;236;38;5;255m $ {} \x1b[0m", truncate_for_summary(command, 160))
    }
}

fn first_visible_line(text: &str) -> &str {
    text.lines().find(|line| !line.trim().is_empty()).unwrap_or(text)
}

fn format_bash_result(icon: &str, parsed: &serde_json::Value) -> String {
    use std::fmt::Write as _;

    let exit_code = parsed.get("exitCode").and_then(serde_json::Value::as_i64).unwrap_or(0);
    let status = if exit_code == 0 { "" } else { " \x1b[38;5;203merror\x1b[0m" };
    let mut header = format!("{icon} \x1b[38;5;245mbash\x1b[0m{status}");

    if let Some(task_id) = parsed.get("backgroundTaskId").and_then(|value| value.as_str()) {
        write!(&mut header, " backgrounded ({task_id})").expect("write");
    }

    let stdout = parsed.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
    let stderr = parsed.get("stderr").and_then(|v| v.as_str()).unwrap_or("");

    // Clean one-liner: show first non-empty line of output as preview
    let preview_line =
        stdout.lines().chain(stderr.lines()).find(|l| !l.trim().is_empty()).unwrap_or("");
    let preview = truncate_for_summary(preview_line.trim(), 100);

    let mut lines = vec![header];

    if !stdout.trim().is_empty() {
        let trimmed = stdout.trim();
        let line_count = stdout.lines().count();
        if line_count <= 3 && trimmed.len() < 200 {
            // Short output: show inline
            lines.push(format!("\x1b[2m{trimmed}\x1b[0m"));
        } else if !preview.is_empty() {
            lines.push(format!("\x1b[2m{preview}\x1b[0m"));
        }
    }

    if !stderr.trim().is_empty() {
        let stderr_preview = truncate_for_summary(stderr.trim(), 80);
        lines.push(format!("\x1b[38;5;203m{stderr_preview}\x1b[0m"));
    }

    lines.join("\n")
}

fn format_read_result(icon: &str, parsed: &serde_json::Value) -> String {
    let file = parsed.get("file").unwrap_or(parsed);
    let path = extract_tool_path(file);
    let start_line = file.get("startLine").and_then(serde_json::Value::as_u64).unwrap_or(1);
    let num_lines = file.get("numLines").and_then(serde_json::Value::as_u64).unwrap_or(0);
    let total_lines =
        file.get("totalLines").and_then(serde_json::Value::as_u64).unwrap_or(num_lines);
    let content = file.get("content").and_then(|value| value.as_str()).unwrap_or_default();
    let end_line = start_line.saturating_add(num_lines.saturating_sub(1));

    format!(
        "{icon} \x1b[2m📄 Read {path} (lines {}-{} of {})\x1b[0m\n{}",
        start_line,
        end_line.max(start_line),
        total_lines,
        truncate_output_for_display(content, READ_DISPLAY_MAX_LINES, READ_DISPLAY_MAX_CHARS)
    )
}

fn format_write_result(icon: &str, parsed: &serde_json::Value) -> String {
    let path = extract_tool_path(parsed);
    let kind = parsed.get("type").and_then(|value| value.as_str()).unwrap_or("write");
    let line_count = parsed
        .get("content")
        .and_then(|value| value.as_str())
        .map_or(0, |content| content.lines().count());
    format!(
        "{icon} \x1b[1;32m✏️ {} {path}\x1b[0m \x1b[2m({line_count} lines)\x1b[0m",
        if kind == "create" { "Wrote" } else { "Updated" },
    )
}

fn format_structured_patch_preview(parsed: &serde_json::Value) -> Option<String> {
    let hunks = parsed.get("structuredPatch")?.as_array()?;
    let mut preview = Vec::new();
    for hunk in hunks.iter().take(2) {
        let lines = hunk.get("lines")?.as_array()?;
        for line in lines.iter().filter_map(|value| value.as_str()).take(6) {
            match line.chars().next() {
                Some('+') => preview.push(format!("\x1b[38;5;70m{line}\x1b[0m")),
                Some('-') => preview.push(format!("\x1b[38;5;203m{line}\x1b[0m")),
                _ => preview.push(line.to_string()),
            }
        }
    }
    if preview.is_empty() {
        None
    } else {
        Some(preview.join("\n"))
    }
}

fn format_edit_result(icon: &str, parsed: &serde_json::Value) -> String {
    let path = extract_tool_path(parsed);
    let suffix = if parsed.get("replaceAll").and_then(serde_json::Value::as_bool).unwrap_or(false) {
        " (replace all)"
    } else {
        ""
    };
    let preview = format_structured_patch_preview(parsed).or_else(|| {
        let old_value =
            parsed.get("oldString").and_then(|value| value.as_str()).unwrap_or_default();
        let new_value =
            parsed.get("newString").and_then(|value| value.as_str()).unwrap_or_default();
        format_patch_preview(old_value, new_value)
    });

    match preview {
        Some(preview) => format!("{icon} \x1b[1;33m📝 Edited {path}{suffix}\x1b[0m\n{preview}"),
        None => format!("{icon} \x1b[1;33m📝 Edited {path}{suffix}\x1b[0m"),
    }
}

fn format_glob_result(icon: &str, parsed: &serde_json::Value) -> String {
    let num_files = parsed.get("numFiles").and_then(serde_json::Value::as_u64).unwrap_or(0);
    let filenames = parsed
        .get("filenames")
        .and_then(|value| value.as_array())
        .map(|files| {
            files.iter().filter_map(|value| value.as_str()).take(8).collect::<Vec<_>>().join("\n")
        })
        .unwrap_or_default();
    if filenames.is_empty() {
        format!("{icon} \x1b[38;5;245mglob_search\x1b[0m matched {num_files} files")
    } else {
        format!("{icon} \x1b[38;5;245mglob_search\x1b[0m matched {num_files} files\n{filenames}")
    }
}

fn format_grep_result(icon: &str, parsed: &serde_json::Value) -> String {
    let num_matches = parsed.get("numMatches").and_then(serde_json::Value::as_u64).unwrap_or(0);
    let num_files = parsed.get("numFiles").and_then(serde_json::Value::as_u64).unwrap_or(0);
    let content = parsed.get("content").and_then(|value| value.as_str()).unwrap_or_default();
    let filenames = parsed
        .get("filenames")
        .and_then(|value| value.as_array())
        .map(|files| {
            files.iter().filter_map(|value| value.as_str()).take(8).collect::<Vec<_>>().join("\n")
        })
        .unwrap_or_default();
    let summary = format!(
        "{icon} \x1b[38;5;245mgrep_search\x1b[0m {num_matches} matches across {num_files} files"
    );
    if !content.trim().is_empty() {
        format!(
            "{summary}\n{}",
            truncate_output_for_display(
                content,
                TOOL_OUTPUT_DISPLAY_MAX_LINES,
                TOOL_OUTPUT_DISPLAY_MAX_CHARS,
            )
        )
    } else if !filenames.is_empty() {
        format!("{summary}\n{filenames}")
    } else {
        summary
    }
}

fn format_generic_tool_result(icon: &str, name: &str, parsed: &serde_json::Value) -> String {
    let rendered_output = match parsed {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Null => String::new(),
        serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
            serde_json::to_string_pretty(parsed).unwrap_or_else(|_| parsed.to_string())
        }
        _ => parsed.to_string(),
    };
    let preview = truncate_output_for_display(
        &rendered_output,
        TOOL_OUTPUT_DISPLAY_MAX_LINES,
        TOOL_OUTPUT_DISPLAY_MAX_CHARS,
    );

    if preview.is_empty() {
        format!("{icon} \x1b[38;5;245m{name}\x1b[0m")
    } else if preview.contains('\n') {
        format!("{icon} \x1b[38;5;245m{name}\x1b[0m\n{preview}")
    } else {
        format!("{icon} \x1b[38;5;245m{name}:\x1b[0m {preview}")
    }
}

fn summarize_tool_payload(payload: &str) -> String {
    let compact = match serde_json::from_str::<serde_json::Value>(payload) {
        Ok(value) => value.to_string(),
        Err(_) => payload.trim().to_string(),
    };
    truncate_for_summary(&compact, 96)
}

fn truncate_for_summary(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn truncate_output_for_display(content: &str, max_lines: usize, max_chars: usize) -> String {
    let original = content.trim_end_matches('\n');
    if original.is_empty() {
        return String::new();
    }

    let mut preview_lines = Vec::new();
    let mut used_chars = 0usize;
    let mut truncated = false;

    for (index, line) in original.lines().enumerate() {
        if index >= max_lines {
            truncated = true;
            break;
        }

        let newline_cost = usize::from(!preview_lines.is_empty());
        let available = max_chars.saturating_sub(used_chars + newline_cost);
        if available == 0 {
            truncated = true;
            break;
        }

        let line_chars = line.chars().count();
        if line_chars > available {
            preview_lines.push(line.chars().take(available).collect::<String>());
            truncated = true;
            break;
        }

        preview_lines.push(line.to_string());
        used_chars += newline_cost + line_chars;
    }

    let mut preview = preview_lines.join("\n");
    if truncated {
        if !preview.is_empty() {
            preview.push('\n');
        }
        preview.push_str(DISPLAY_TRUNCATION_NOTICE);
    }
    preview
}

fn push_output_block(
    block: OutputContentBlock,
    out: &mut (impl Write + ?Sized),
    events: &mut Vec<AssistantEvent>,
    pending_tool: &mut Option<(String, String, String)>,
    streaming_tool_input: bool,
) -> Result<(), RuntimeError> {
    match block {
        OutputContentBlock::Text { text } => {
            if !text.is_empty() {
                let rendered = TerminalRenderer::new().markdown_to_ansi(&text);
                write!(out, "{rendered}")
                    .and_then(|()| out.flush())
                    .map_err(|error| RuntimeError::new(error.to_string()))?;
                events.push(AssistantEvent::TextDelta(text));
            }
        }
        OutputContentBlock::ToolUse { id, name, input } => {
            // During streaming, the initial content_block_start has an empty input ({}).
            // The real input arrives via input_json_delta events. In
            // non-streaming responses, preserve a legitimate empty object.
            let initial_input = if streaming_tool_input
                && input.is_object()
                && input.as_object().is_some_and(serde_json::Map::is_empty)
            {
                String::new()
            } else {
                input.to_string()
            };
            *pending_tool = Some((id, name, initial_input));
        }
        OutputContentBlock::Thinking { thinking, signature } => {
            events.push(AssistantEvent::Thinking { thinking, signature });
        }
        OutputContentBlock::RedactedThinking { .. } => {
            // Redacted thinking cannot be passed back; treated as empty.
        }
    }
    Ok(())
}

fn response_to_events(
    response: MessageResponse,
    out: &mut (impl Write + ?Sized),
) -> Result<Vec<AssistantEvent>, RuntimeError> {
    let mut events = Vec::new();
    let mut pending_tool = None;

    for block in response.content {
        push_output_block(block, out, &mut events, &mut pending_tool, false)?;
        if let Some((id, name, input)) = pending_tool.take() {
            events.push(AssistantEvent::ToolUse { id, name, input });
        }
    }

    events.push(AssistantEvent::Usage(response.usage.token_usage()));
    events.push(AssistantEvent::MessageStop);
    Ok(events)
}

fn push_prompt_cache_record(client: &ProviderClient, events: &mut Vec<AssistantEvent>) {
    if let Some(record) = client.take_last_prompt_cache_record() {
        if let Some(event) = prompt_cache_record_to_runtime_event(record) {
            events.push(AssistantEvent::PromptCache(event));
        }
    }
}

fn prompt_cache_record_to_runtime_event(
    record: api::PromptCacheRecord,
) -> Option<PromptCacheEvent> {
    let cache_break = record.cache_break?;
    Some(PromptCacheEvent {
        unexpected: cache_break.unexpected,
        reason: cache_break.reason,
        previous_cache_read_input_tokens: cache_break.previous_cache_read_input_tokens,
        current_cache_read_input_tokens: cache_break.current_cache_read_input_tokens,
        token_drop: cache_break.token_drop,
    })
}

struct CliToolExecutor {
    renderer: TerminalRenderer,
    emit_output: bool,
    allowed_tools: Option<AllowedToolSet>,
    tool_registry: GlobalToolRegistry,
    mcp_state: Option<Arc<Mutex<RuntimeMcpState>>>,
}

impl CliToolExecutor {
    fn new(
        allowed_tools: Option<AllowedToolSet>,
        emit_output: bool,
        tool_registry: GlobalToolRegistry,
        mcp_state: Option<Arc<Mutex<RuntimeMcpState>>>,
    ) -> Self {
        Self {
            renderer: TerminalRenderer::new(),
            emit_output,
            allowed_tools,
            tool_registry,
            mcp_state,
        }
    }

    fn execute_search_tool(&self, value: serde_json::Value) -> Result<String, ToolError> {
        let input: ToolSearchRequest = serde_json::from_value(value)
            .map_err(|error| ToolError::new(format!("invalid tool input JSON: {error}")))?;
        let (pending_mcp_servers, mcp_degraded) =
            self.mcp_state.as_ref().map_or((None, None), |state| {
                let state = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                (state.pending_servers(), state.degraded_report())
            });
        serde_json::to_string_pretty(&self.tool_registry.search(
            &input.query,
            input.max_results.unwrap_or(5),
            pending_mcp_servers,
            mcp_degraded,
        ))
        .map_err(|error| ToolError::new(error.to_string()))
    }

    fn execute_runtime_tool(
        &self,
        tool_name: &str,
        value: serde_json::Value,
    ) -> Result<String, ToolError> {
        let Some(mcp_state) = &self.mcp_state else {
            return Err(ToolError::new(format!(
                "runtime tool `{tool_name}` is unavailable without configured MCP servers"
            )));
        };
        let mut mcp_state = mcp_state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

        match tool_name {
            "MCPTool" => {
                let input: McpToolRequest = serde_json::from_value(value)
                    .map_err(|error| ToolError::new(format!("invalid tool input JSON: {error}")))?;
                let qualified_name = input
                    .qualified_name
                    .or(input.tool)
                    .ok_or_else(|| ToolError::new("missing required field `qualifiedName`"))?;
                mcp_state.call_tool(&qualified_name, input.arguments)
            }
            "ListMcpResourcesTool" => {
                let input: ListMcpResourcesRequest = serde_json::from_value(value)
                    .map_err(|error| ToolError::new(format!("invalid tool input JSON: {error}")))?;
                match input.server {
                    Some(server_name) => mcp_state.list_resources_for_server(&server_name),
                    None => mcp_state.list_resources_for_all_servers(),
                }
            }
            "ReadMcpResourceTool" => {
                let input: ReadMcpResourceRequest = serde_json::from_value(value)
                    .map_err(|error| ToolError::new(format!("invalid tool input JSON: {error}")))?;
                mcp_state.read_resource(&input.server, &input.uri)
            }
            _ => mcp_state.call_tool(tool_name, Some(value)),
        }
    }
}

impl ToolExecutor for CliToolExecutor {
    fn execute(&mut self, tool_name: &str, input: &str) -> Result<String, ToolError> {
        if self.allowed_tools.as_ref().is_some_and(|allowed| !allowed.contains(tool_name)) {
            return Err(ToolError::new(format!(
                "tool `{tool_name}` is not enabled by the current --allowedTools setting"
            )));
        }
        let value = serde_json::from_str(input)
            .map_err(|error| ToolError::new(format!("invalid tool input JSON: {error}")))?;
        let result = if tool_name == "ToolSearch" {
            self.execute_search_tool(value)
        } else if self.tool_registry.has_runtime_tool(tool_name) {
            self.execute_runtime_tool(tool_name, value)
        } else {
            self.tool_registry.execute(tool_name, &value).map_err(ToolError::new)
        };
        match result {
            Ok(output) => {
                if self.emit_output {
                    let markdown = format_tool_result(tool_name, &output, false);
                    self.renderer
                        .stream_markdown(&markdown, &mut io::stdout())
                        .map_err(|error| ToolError::new(error.to_string()))?;
                }
                Ok(output)
            }
            Err(error) => {
                if self.emit_output {
                    let markdown = format_tool_result(tool_name, &error.to_string(), true);
                    self.renderer
                        .stream_markdown(&markdown, &mut io::stdout())
                        .map_err(|stream_error| ToolError::new(stream_error.to_string()))?;
                }
                Err(error)
            }
        }
    }
}

fn permission_policy(
    mode: PermissionMode,
    feature_config: &runtime::RuntimeFeatureConfig,
    tool_registry: &GlobalToolRegistry,
) -> Result<PermissionPolicy, String> {
    let base = PermissionPolicy::new(mode).with_permission_rules(feature_config.permission_rules());
    // c10/b: enable review-trust bash classifier when --permission-profile review-trust was passed.
    let policy = if env::var("SEGO_REVIEW_TRUST").as_deref() == Ok("1") {
        base.with_review_trust()
    } else {
        base
    };
    Ok(tool_registry.permission_specs(None)?.into_iter().fold(
        policy,
        |policy, (name, required_permission)| {
            policy.with_tool_requirement(name, required_permission)
        },
    ))
}

fn convert_messages(messages: &[ConversationMessage]) -> Vec<InputMessage> {
    let mut result = Vec::new();
    let mut pending_tool_use_ids: HashSet<String> = HashSet::new();
    let mut i = 0;

    while i < messages.len() {
        let message = &messages[i];
        let role = match message.role {
            MessageRole::System | MessageRole::User | MessageRole::Tool => "user",
            MessageRole::Assistant => "assistant",
        };

        // Build content, filtering orphan tool_results in non-assistant messages
        let content: Vec<InputContentBlock> = message
            .blocks
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(InputContentBlock::Text { text: text.clone() }),
                ContentBlock::ToolUse { id, name, input } => Some(InputContentBlock::ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: serde_json::from_str(input)
                        .unwrap_or_else(|_| serde_json::json!({ "raw": input })),
                }),
                ContentBlock::ToolResult { tool_use_id, output, is_error, .. } => {
                    // DeepSeek: drop tool_results without matching tool_use in previous assistant
                    if role != "assistant" && !pending_tool_use_ids.contains(tool_use_id) {
                        return None;
                    }
                    Some(InputContentBlock::ToolResult {
                        tool_use_id: tool_use_id.clone(),
                        content: vec![ToolResultContentBlock::Text { text: output.clone() }],
                        is_error: *is_error,
                    })
                }
                ContentBlock::Thinking { thinking, signature } => {
                    Some(InputContentBlock::Thinking {
                        thinking: thinking.clone(),
                        signature: signature.clone(),
                    })
                }
            })
            .collect();

        if role == "assistant" {
            // Track tool_use IDs from this assistant message
            pending_tool_use_ids = message
                .blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::ToolUse { id, .. } => Some(id.clone()),
                    _ => None,
                })
                .collect();

            if !pending_tool_use_ids.is_empty() {
                // Merge consecutive tool_result messages into one user message
                let mut tool_results = Vec::new();
                let mut j = i + 1;
                while j < messages.len() {
                    let next = &messages[j];
                    if next.blocks.iter().any(|b| matches!(b, ContentBlock::ToolResult { .. })) {
                        for block in &next.blocks {
                            if let ContentBlock::ToolResult {
                                tool_use_id, output, is_error, ..
                            } = block
                            {
                                if pending_tool_use_ids.contains(tool_use_id) {
                                    tool_results.push(InputContentBlock::ToolResult {
                                        tool_use_id: tool_use_id.clone(),
                                        content: vec![ToolResultContentBlock::Text {
                                            text: output.clone(),
                                        }],
                                        is_error: *is_error,
                                    });
                                }
                            }
                        }
                        j += 1;
                    } else {
                        break;
                    }
                }

                if !content.is_empty() {
                    result.push(InputMessage { role: "assistant".to_string(), content });
                }
                if !tool_results.is_empty() {
                    result.push(InputMessage { role: "user".to_string(), content: tool_results });
                }
                i = j;
                continue;
            }
        }

        if !content.is_empty() {
            result.push(InputMessage { role: role.to_string(), content });
        }
        i += 1;
    }
    result
}

#[allow(clippy::too_many_lines)]
fn print_help_to(out: &mut impl Write) -> io::Result<()> {
    writeln!(out, "sego v{VERSION}")?;
    writeln!(out)?;
    writeln!(out, "Usage:")?;
    writeln!(out, "  sego [--model MODEL] [--allowedTools TOOL[,TOOL...]]")?;
    writeln!(out, "      Start the interactive REPL")?;
    writeln!(out, "  sego [--model MODEL] [--output-format text|json] prompt TEXT")?;
    writeln!(out, "      Send one prompt and exit")?;
    writeln!(out, "  sego [--model MODEL] [--output-format text|json] TEXT")?;
    writeln!(out, "      Shorthand non-interactive prompt mode")?;
    writeln!(out, "  sego --resume [SESSION.jsonl|session-id|latest] [/status] [/compact] [...]")?;
    writeln!(out, "      Inspect or maintain a saved session without entering the REPL")?;
    writeln!(out, "  sego help")?;
    writeln!(out, "      Alias for --help")?;
    writeln!(out, "  sego version")?;
    writeln!(out, "      Alias for --version")?;
    writeln!(out, "  sego update")?;
    writeln!(out, "      Check GitHub Releases and install the latest Sego on Windows")?;
    writeln!(out, "  sego status")?;
    writeln!(out, "      Show the current local workspace status snapshot")?;
    writeln!(out, "  sego sandbox")?;
    writeln!(out, "      Show the current sandbox isolation snapshot")?;
    writeln!(out, "  sego dump-manifests")?;
    writeln!(out, "  sego bootstrap-plan")?;
    writeln!(out, "  sego agents")?;
    writeln!(out, "  sego mcp")?;
    writeln!(out, "  sego skills")?;
    writeln!(out, "  sego system-prompt [--cwd PATH] [--date YYYY-MM-DD]")?;
    writeln!(out, "  sego login")?;
    writeln!(out, "  sego logout")?;
    writeln!(out, "  sego init")?;
    writeln!(out, "  sego review")?;
    writeln!(out, "      Run code review for the current workspace and save review artifacts")?;
    writeln!(out, "  sego review [staged|unstaged|workspace|PATH]")?;
    writeln!(out, "      Run code review for a specific diff scope or file path")?;
    writeln!(out, "  sego workflow-review [--last N]")?;
    writeln!(out, "      Show workflow analysis for recent sessions")?;
    writeln!(out, "  sego session-review [--last N]")?;
    writeln!(out, "      Alias for workflow-review")?;
    writeln!(out, "  sego learn")?;
    writeln!(out, "      Show learning suggestions based on workflow history")?;
    writeln!(out, "  sego doctor")?;
    writeln!(out, "      Show system diagnostics and health check")?;
    writeln!(out)?;
    writeln!(out, "Flags:")?;
    writeln!(out, "  --model MODEL              Override the active model")?;
    writeln!(out, "  --output-format FORMAT     Non-interactive output format: text or json")?;
    writeln!(
        out,
        "  --permission-mode MODE     Set read-only, workspace-write, or danger-full-access"
    )?;
    writeln!(out, "  --dangerously-skip-permissions  Skip all permission checks")?;
    writeln!(out, "  --allowedTools TOOLS       Restrict enabled tools (repeatable; comma-separated aliases supported)")?;
    writeln!(out, "  --version, -V              Print version and build information locally")?;
    writeln!(out)?;
    writeln!(out, "Interactive slash commands:")?;
    writeln!(out, "{}", render_slash_command_help())?;
    writeln!(out)?;
    let resume_commands = resume_supported_slash_commands()
        .into_iter()
        .map(|spec| match spec.argument_hint {
            Some(argument_hint) => format!("/{} {}", spec.name, argument_hint),
            None => format!("/{}", spec.name),
        })
        .collect::<Vec<_>>()
        .join(", ");
    writeln!(out, "Resume-safe commands: {resume_commands}")?;
    writeln!(out)?;
    writeln!(out, "Session shortcuts:")?;
    writeln!(
        out,
        "  REPL turns auto-save to .claw/sessions/<session-id>.{PRIMARY_SESSION_EXTENSION}"
    )?;
    writeln!(
        out,
        "  Use `{LATEST_SESSION_REFERENCE}` with --resume, /resume, or /session switch to target the newest saved session"
    )?;
    writeln!(out, "  Use /session list in the REPL to browse managed sessions")?;
    writeln!(out, "Examples:")?;
    writeln!(out, "  sego --model claude-opus \"summarize this repo\"")?;
    writeln!(out, "  sego --output-format json prompt \"explain src/main.rs\"")?;
    writeln!(out, "  sego --allowedTools read,glob \"summarize Cargo.toml\"")?;
    writeln!(out, "  sego --resume {LATEST_SESSION_REFERENCE}")?;
    writeln!(out, "  sego --resume {LATEST_SESSION_REFERENCE} /status /diff /export notes.txt")?;
    writeln!(out, "  sego agents")?;
    writeln!(out, "  sego mcp show my-server")?;
    writeln!(out, "  sego /skills")?;
    writeln!(out, "  sego login")?;
    writeln!(out, "  sego init")?;
    Ok(())
}

fn print_help() {
    let _ = print_help_to(&mut io::stdout());
}

mod review_reports;

pub(crate) use review_reports::{
    build_review_summary_json_value, format_preflight_block_error,
    format_review_completion_summary, push_review_summary_field, CodeReviewReadinessReport,
    CodeReviewSummaryReport, ReviewFindingStatusCounts,
};

#[cfg(test)]
mod tests;

fn write_mcp_server_fixture(script_path: &Path) {
    let script = [
            "#!/usr/bin/env python3",
            "import json, sys",
            "",
            "def read_message():",
            "    header = b''",
            r"    while not header.endswith(b'\r\n\r\n'):",
            "        chunk = sys.stdin.buffer.read(1)",
            "        if not chunk:",
            "            return None",
            "        header += chunk",
            "    length = 0",
            r"    for line in header.decode().split('\r\n'):",
            r"        if line.lower().startswith('content-length:'):",
            "            length = int(line.split(':', 1)[1].strip())",
            "    payload = sys.stdin.buffer.read(length)",
            "    return json.loads(payload.decode())",
            "",
            "def send_message(message):",
            "    payload = json.dumps(message).encode()",
            r"    sys.stdout.buffer.write(f'Content-Length: {len(payload)}\r\n\r\n'.encode() + payload)",
            "    sys.stdout.buffer.flush()",
            "",
            "while True:",
            "    request = read_message()",
            "    if request is None:",
            "        break",
            "    method = request['method']",
            "    if method == 'initialize':",
            "        send_message({",
            "            'jsonrpc': '2.0',",
            "            'id': request['id'],",
            "            'result': {",
            "                'protocolVersion': request['params']['protocolVersion'],",
            "                'capabilities': {'tools': {}, 'resources': {}},",
            "                'serverInfo': {'name': 'fixture', 'version': '1.0.0'}",
            "            }",
            "        })",
            "    elif method == 'tools/list':",
            "        send_message({",
            "            'jsonrpc': '2.0',",
            "            'id': request['id'],",
            "            'result': {",
            "                'tools': [",
            "                    {",
            "                        'name': 'echo',",
            "                        'description': 'Echo from MCP fixture',",
            "                        'inputSchema': {",
            "                            'type': 'object',",
            "                            'properties': {'text': {'type': 'string'}},",
            "                            'required': ['text'],",
            "                            'additionalProperties': False",
            "                        },",
            "                        'annotations': {'readOnlyHint': True}",
            "                    }",
            "                ]",
            "            }",
            "        })",
            "    elif method == 'tools/call':",
            "        args = request['params'].get('arguments') or {}",
            "        send_message({",
            "            'jsonrpc': '2.0',",
            "            'id': request['id'],",
            "            'result': {",
            "                'content': [{'type': 'text', 'text': f\"echo:{args.get('text', '')}\"}],",
            "                'structuredContent': {'echoed': args.get('text', '')},",
            "                'isError': False",
            "            }",
            "        })",
            "    elif method == 'resources/list':",
            "        send_message({",
            "            'jsonrpc': '2.0',",
            "            'id': request['id'],",
            "            'result': {",
            "                'resources': [{'uri': 'file://guide.txt', 'name': 'guide', 'mimeType': 'text/plain'}]",
            "            }",
            "        })",
            "    elif method == 'resources/read':",
            "        uri = request['params']['uri']",
            "        send_message({",
            "            'jsonrpc': '2.0',",
            "            'id': request['id'],",
            "            'result': {",
            "                'contents': [{'uri': uri, 'mimeType': 'text/plain', 'text': f'contents for {uri}'}]",
            "            }",
            "        })",
            "    else:",
            "        send_message({",
            "            'jsonrpc': '2.0',",
            "            'id': request['id'],",
            "            'error': {'code': -32601, 'message': method}",
            "        })",
            "",
        ]
        .join("\n");
    fs::write(script_path, script).expect("mcp fixture script should write");
}

#[cfg(test)]
mod sandbox_report_tests;

mod cli_suggestions;

pub(crate) use cli_suggestions::{
    bare_slash_command_guidance, format_unknown_direct_slash_command, format_unknown_option,
    format_unknown_slash_command, levenshtein_distance, looks_like_slash_command_token,
    ranked_suggestions, render_suggestion_line, suggest_closest_term, suggest_slash_commands,
    CLI_OPTION_SUGGESTIONS,
};

mod cli_args;

pub(crate) use cli_args::{
    apply_requested_cwd, build_plugin_manager, build_runtime_mcp_state,
    build_runtime_plugin_state_with_loader, current_tool_registry, default_date, default_model,
    is_leap, join_optional_args, mcp_wrapper_tool_definitions, normalize_allowed_tools,
    normalize_permission_mode, parse_args, parse_code_review_slash_action,
    parse_direct_slash_cli_action, parse_permission_mode_arg, parse_positive_usize,
    parse_resume_args, parse_review_history_command, parse_single_word_command_alias,
    parse_system_prompt_args, parse_workflow_review_args, resolve_cli_cwd, resolve_model_alias,
    resolve_plugin_path, resume_command_can_absorb_token, review_history_command_to_cli_action,
    runtime_hook_config_from_plugin_hooks, CliAction, CliOutputFormat, ReviewHistoryCommand,
    RuntimeMcpState, SafetyReviewScope,
};

mod cli_reports;

pub(crate) use cli_reports::{
    format_auto_compaction_notice, format_commit_preflight_report, format_commit_skipped_report,
    format_compact_report, format_cost_report, format_missing_session_reference,
    format_model_report, format_model_switch_report, format_no_managed_sessions,
    format_permissions_report, format_permissions_switch_report, format_resume_report,
    format_sandbox_report, format_session_modified_age, format_status_report,
    format_workspace_report, format_workspace_switch_report, provider_kind_label,
    render_natural_language_directory, render_repl_help, render_resume_usage, GitWorkspaceSummary,
    StatusContext, StatusUsage, WorkspaceContext,
};

mod cli_context;

pub(crate) use cli_context::{
    find_git_root_in, parse_git_status_branch, parse_git_status_metadata,
    parse_git_status_metadata_for, parse_git_workspace_summary, resolve_git_branch_for,
    run_git_capture_in, status_context, workspace_context,
};
