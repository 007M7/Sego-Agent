#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    clippy::unneeded_struct_pattern,
    clippy::unnecessary_wraps,
    clippy::unused_self
)]
mod init;
mod input;
mod render;
mod sidecar;

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
use plugins::{PluginHooks, PluginManager, PluginManagerConfig, PluginRegistry};
use render::{MarkdownStreamState, Spinner, TerminalRenderer};
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

fn default_model() -> String {
    std::env::var("DEEPSEEK_MODEL")
        .or_else(|_| std::env::var("ANTHROPIC_MODEL"))
        .unwrap_or_else(|_| "deepseek-chat".to_string())
}

fn max_tokens_for_model(model: &str) -> u32 {
    // DeepSeek, MiMo, and GPT all support 64K output
    64_000
}

fn context_window_limit(model: &str) -> u32 {
    if model.contains("deepseek") || model.contains("v4") {
        1_000_000 // DeepSeek V4 supports 1M context
    } else if model.contains("mimo") {
        128_000 // MiMo supports 128K context
    } else if model.starts_with("gpt") {
        128_000 // GPT models 128K context
    } else {
        128_000 // conservative default
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
fn default_date() -> String {
    std::env::var("ANTHROPIC_DATE").unwrap_or_else(|_| {
        let now =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let secs = now.as_secs();
        // Simple date calculation from Unix epoch
        let days_since_epoch = secs / 86400;
        // 1970-01-01 was a Thursday. Calculate year/month/day.
        let mut days = days_since_epoch as i64;
        let mut year = 1970i64;
        loop {
            let year_days = if is_leap(year) { 366 } else { 365 };
            if days < year_days {
                break;
            }
            days -= year_days;
            year += 1;
        }
        let month_days = if is_leap(year) {
            [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        } else {
            [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        };
        let mut month = 0i64;
        while month < 12 && days >= month_days[month as usize] {
            days -= month_days[month as usize];
            month += 1;
        }
        format!("{:04}-{:02}-{:02}", year, month + 1, days + 1)
    })
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
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
const CLI_OPTION_SUGGESTIONS: &[&str] = &[
    "--help",
    "-h",
    "--version",
    "-V",
    "--model",
    "--output-format",
    "--permission-mode",
    "--dangerously-skip-permissions",
    "--allowedTools",
    "--allowed-tools",
    "--resume",
    "--print",
    "-p",
];

type AllowedToolSet = BTreeSet<String>;

fn main() {
    if let Err(error) = run() {
        let message = error.to_string();
        if message.contains("`sego --help`") || message == "verification failed" {
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

fn maybe_pause_after_error() {
    if !cfg!(windows) || env::var_os("SEGO_PAUSE_ON_ERROR").is_none() {
        return;
    }

    eprintln!();
    eprintln!("Press Enter to close this window.");
    let mut buffer = String::new();
    let _ = io::stdin().read_line(&mut buffer);
}

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
        CliAction::Update => run_update()?,
        CliAction::Workspace { output_format } => print_workspace_snapshot(output_format)?,
        CliAction::ResumeSession { session_path, commands } => {
            resume_session(&session_path, &commands);
        }
        CliAction::Status { model, permission_mode, output_format } => {
            print_status_snapshot(&model, permission_mode, output_format)?;
        }
        CliAction::Sandbox { output_format } => print_sandbox_status_snapshot(output_format)?,
        CliAction::Prompt { prompt, model, output_format, allowed_tools, permission_mode } => {
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
            cli.run_turn_with_output(&prompt, output_format)?;
            persist_recovery_for_cli(
                runtime::recovery::RecoveryExitState::Graceful,
                cli.session_id(),
                cli.session_path(),
                Some(cli.model_name()),
                Some(&prompt),
            );
        }
        CliAction::CodeReview { scope, model, allowed_tools, permission_mode } => {
            run_code_review_cli(model, allowed_tools, permission_mode, scope.as_deref())?;
        }
        CliAction::CodeReviewList => print_code_review_history()?,
        CliAction::CodeReviewShow { id } => print_code_review_report(&id)?,
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
        CliAction::Review { last_n, output_format } => {
            print_workflow_review(last_n, output_format)?;
        }
        CliAction::Learn { output_format } => print_workflow_learn(output_format)?,
        CliAction::Doctor { output_format } => print_doctor(output_format)?,
        CliAction::Telemetry { action, output_format } => {
            print_telemetry(action.as_deref(), output_format)?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliAction {
    DumpManifests,
    BootstrapPlan,
    SidecarReview,
    Agents {
        args: Option<String>,
    },
    Mcp {
        args: Option<String>,
    },
    Skills {
        args: Option<String>,
    },
    PrintSystemPrompt {
        cwd: PathBuf,
        date: String,
    },
    Version,
    Update,
    Workspace {
        output_format: CliOutputFormat,
    },
    ResumeSession {
        session_path: PathBuf,
        commands: Vec<String>,
    },
    Status {
        model: String,
        permission_mode: PermissionMode,
        output_format: CliOutputFormat,
    },
    Sandbox {
        output_format: CliOutputFormat,
    },
    Prompt {
        prompt: String,
        model: String,
        output_format: CliOutputFormat,
        allowed_tools: Option<AllowedToolSet>,
        permission_mode: PermissionMode,
    },
    CodeReview {
        scope: Option<String>,
        model: String,
        allowed_tools: Option<AllowedToolSet>,
        permission_mode: PermissionMode,
    },
    CodeReviewList,
    CodeReviewShow {
        id: String,
    },
    CodeReviewStatus {
        id: String,
    },
    CodeReviewMark {
        id: String,
        finding_id: String,
        status: ReviewFindingStatus,
        note: Option<String>,
    },
    CodeReviewReady,
    CodeReviewSummary,
    CodeReviewTools,
    CodeReviewSafety {
        scope: SafetyReviewScope,
    },
    CodeVerify {
        scope: Option<String>,
    },
    Login,
    Logout,
    Init,
    Repl {
        model: String,
        allowed_tools: Option<AllowedToolSet>,
        permission_mode: PermissionMode,
    },
    // prompt-mode formatting is only supported for non-interactive runs
    Help,
    Review {
        last_n: Option<usize>,
        output_format: CliOutputFormat,
    },
    Learn {
        output_format: CliOutputFormat,
    },
    Doctor {
        output_format: CliOutputFormat,
    },
    Telemetry {
        action: Option<String>,
        output_format: CliOutputFormat,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SafetyReviewScope {
    Workspace,
    Staged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliOutputFormat {
    Text,
    Json,
}

impl CliOutputFormat {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "text" => Ok(Self::Text),
            "json" => Ok(Self::Json),
            other => Err(format!(
                "unsupported value for --output-format: {other} (expected text or json)"
            )),
        }
    }
}

#[allow(clippy::too_many_lines)]
fn parse_args(args: &[String]) -> Result<CliAction, String> {
    let mut requested_cwd: Option<PathBuf> = None;
    let mut model = default_model();
    let mut output_format = CliOutputFormat::Text;
    let mut permission_mode_override = None;
    let mut permission_profile_override: Option<String> = None;
    let mut wants_help = false;
    let mut wants_version = false;
    let mut allowed_tool_values = Vec::new();
    let mut rest = Vec::new();
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--help" | "-h" if rest.is_empty() => {
                wants_help = true;
                index += 1;
            }
            "--version" | "-V" => {
                wants_version = true;
                index += 1;
            }
            "--model" => {
                let value =
                    args.get(index + 1).ok_or_else(|| "missing value for --model".to_string())?;
                model = resolve_model_alias(value).to_string();
                index += 2;
            }
            flag if flag.starts_with("--model=") => {
                model = resolve_model_alias(&flag[8..]).to_string();
                index += 1;
            }
            "--cwd" if rest.is_empty() => {
                let value =
                    args.get(index + 1).ok_or_else(|| "missing value for --cwd".to_string())?;
                requested_cwd = Some(resolve_cli_cwd(value)?);
                index += 2;
            }
            flag if rest.is_empty() && flag.starts_with("--cwd=") => {
                requested_cwd = Some(resolve_cli_cwd(&flag[6..])?);
                index += 1;
            }
            "--output-format" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --output-format".to_string())?;
                output_format = CliOutputFormat::parse(value)?;
                index += 2;
            }
            "--permission-mode" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --permission-mode".to_string())?;
                permission_mode_override = Some(parse_permission_mode_arg(value)?);
                index += 2;
            }
            flag if flag.starts_with("--output-format=") => {
                output_format = CliOutputFormat::parse(&flag[16..])?;
                index += 1;
            }
            flag if flag.starts_with("--permission-mode=") => {
                permission_mode_override = Some(parse_permission_mode_arg(&flag[18..])?);
                index += 1;
            }
            "--dangerously-skip-permissions" => {
                permission_mode_override = Some(PermissionMode::DangerFullAccess);
                index += 1;
            }
            "--permission-profile" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --permission-profile".to_string())?;
                permission_profile_override = Some(value.clone());
                index += 2;
            }
            flag if flag.starts_with("--permission-profile=") => {
                permission_profile_override = Some(flag[20..].to_string());
                index += 1;
            }
            "-p" => {
                // Claw Code compat: -p "prompt" = one-shot prompt
                let prompt = args[index + 1..].join(" ");
                if prompt.trim().is_empty() {
                    return Err("-p requires a prompt string".to_string());
                }
                apply_requested_cwd(requested_cwd.as_ref())?;
                return Ok(CliAction::Prompt {
                    prompt,
                    model: resolve_model_alias(&model).to_string(),
                    output_format,
                    allowed_tools: normalize_allowed_tools(&allowed_tool_values)?,
                    permission_mode: permission_mode_override
                        .unwrap_or_else(default_permission_mode),
                });
            }
            "--print" => {
                // Claw Code compat: --print makes output non-interactive
                output_format = CliOutputFormat::Text;
                index += 1;
            }
            "--resume" if rest.is_empty() => {
                rest.push("--resume".to_string());
                index += 1;
            }
            flag if rest.is_empty() && flag.starts_with("--resume=") => {
                rest.push("--resume".to_string());
                rest.push(flag[9..].to_string());
                index += 1;
            }
            "--allowedTools" | "--allowed-tools" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "missing value for --allowedTools".to_string())?;
                allowed_tool_values.push(value.clone());
                index += 2;
            }
            flag if flag.starts_with("--allowedTools=") => {
                allowed_tool_values.push(flag[15..].to_string());
                index += 1;
            }
            flag if flag.starts_with("--allowed-tools=") => {
                allowed_tool_values.push(flag[16..].to_string());
                index += 1;
            }
            other if rest.is_empty() && other.starts_with('-') => {
                return Err(format_unknown_option(other))
            }
            other => {
                rest.push(other.to_string());
                index += 1;
            }
        }
    }

    apply_requested_cwd(requested_cwd.as_ref())?;

    // c10/b: --permission-profile review-trust handling.
    // review-trust maps to workspace-write mode + bash command classifier.
    // Mutually exclusive with --permission-mode (Codex decision §2: fail closed).
    if let Some(profile) = &permission_profile_override {
        if permission_mode_override.is_some() {
            return Err(
                "--permission-profile cannot be combined with --permission-mode; choose one"
                    .to_string(),
            );
        }
        match profile.as_str() {
            "review-trust" => {
                // Signal review-trust via env var (consumed by LiveCli policy construction).
                // This avoids changing every CliAction variant signature.
                env::set_var("SEGO_REVIEW_TRUST", "1");
                permission_mode_override = Some(PermissionMode::WorkspaceWrite);
            }
            other => {
                return Err(format!(
                    "unsupported permission profile '{other}'. Currently supported: review-trust"
                ));
            }
        }
    }

    if wants_help {
        return Ok(CliAction::Help);
    }

    if wants_version {
        return Ok(CliAction::Version);
    }

    let allowed_tools = normalize_allowed_tools(&allowed_tool_values)?;

    if rest.is_empty() {
        let permission_mode = permission_mode_override.unwrap_or_else(default_permission_mode);
        return Ok(CliAction::Repl { model, allowed_tools, permission_mode });
    }
    if rest.first().map(String::as_str) == Some("--resume") {
        return parse_resume_args(&rest[1..]);
    }
    if let Some(action) =
        parse_single_word_command_alias(&rest, &model, permission_mode_override, output_format)
    {
        return action;
    }

    let permission_mode = permission_mode_override.unwrap_or_else(default_permission_mode);

    match rest[0].as_str() {
        "dump-manifests" => Ok(CliAction::DumpManifests),
        "bootstrap-plan" => Ok(CliAction::BootstrapPlan),
        "sidecar" => {
            // sego sidecar review — JSON stdin → JSON stdout (c9/c PoC, P1)
            if rest.len() >= 2 && rest[1] == "review" {
                Ok(CliAction::SidecarReview)
            } else {
                Err("sidecar subcommand requires an action (currently: review)".to_string())
            }
        }
        "agents" => Ok(CliAction::Agents { args: join_optional_args(&rest[1..]) }),
        "mcp" => Ok(CliAction::Mcp { args: join_optional_args(&rest[1..]) }),
        "skills" => Ok(CliAction::Skills { args: join_optional_args(&rest[1..]) }),
        "system-prompt" => parse_system_prompt_args(&rest[1..]),
        "update" => Ok(CliAction::Update),
        "login" => Ok(CliAction::Login),
        "logout" => Ok(CliAction::Logout),
        "init" => Ok(CliAction::Init),
        "prompt" => {
            let prompt = rest[1..].join(" ");
            if prompt.trim().is_empty() {
                return Err("prompt subcommand requires a prompt string".to_string());
            }
            Ok(CliAction::Prompt { prompt, model, output_format, allowed_tools, permission_mode })
        }
        other if other.starts_with('/') => {
            parse_direct_slash_cli_action(&rest, model, allowed_tools, permission_mode)
        }
        _other => Ok(CliAction::Prompt {
            prompt: rest.join(" "),
            model,
            output_format,
            allowed_tools,
            permission_mode,
        }),
    }
}

fn parse_single_word_command_alias(
    rest: &[String],
    model: &str,
    permission_mode_override: Option<PermissionMode>,
    output_format: CliOutputFormat,
) -> Option<Result<CliAction, String>> {
    if rest.len() != 1 {
        return None;
    }

    match rest[0].as_str() {
        "help" => Some(Ok(CliAction::Help)),
        "version" => Some(Ok(CliAction::Version)),
        "update" => Some(Ok(CliAction::Update)),
        "status" => Some(Ok(CliAction::Status {
            model: model.to_string(),
            permission_mode: permission_mode_override.unwrap_or_else(default_permission_mode),
            output_format,
        })),
        "sandbox" => Some(Ok(CliAction::Sandbox { output_format })),
        "workspace" | "workdir" | "pwd" | "cwd" => Some(Ok(CliAction::Workspace { output_format })),
        "review" => {
            Some(Ok(CliAction::Review { last_n: None, output_format: CliOutputFormat::Text }))
        }
        "learn" => Some(Ok(CliAction::Learn { output_format: CliOutputFormat::Text })),
        "doctor" => Some(Ok(CliAction::Doctor { output_format: CliOutputFormat::Text })),
        "telemetry" => {
            let action = rest.get(1).cloned();
            Some(Ok(CliAction::Telemetry { action, output_format: CliOutputFormat::Text }))
        }
        other => bare_slash_command_guidance(other).map(Err),
    }
}

fn bare_slash_command_guidance(command_name: &str) -> Option<String> {
    if matches!(
        command_name,
        "dump-manifests"
            | "bootstrap-plan"
            | "agents"
            | "mcp"
            | "skills"
            | "system-prompt"
            | "update"
            | "login"
            | "logout"
            | "init"
            | "prompt"
    ) {
        return None;
    }
    let slash_command = slash_command_specs().iter().find(|spec| spec.name == command_name)?;
    let guidance = if slash_command.resume_supported {
        format!(
            "`claw {command_name}` is a slash command. Use `claw --resume SESSION.jsonl /{command_name}` or start `sego` and run `/{command_name}`."
        )
    } else {
        format!(
            "`claw {command_name}` is a slash command. Start `claw` and run `/{command_name}` inside the REPL."
        )
    };
    Some(guidance)
}

fn join_optional_args(args: &[String]) -> Option<String> {
    let joined = args.join(" ");
    let trimmed = joined.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn parse_direct_slash_cli_action(
    rest: &[String],
    model: String,
    allowed_tools: Option<AllowedToolSet>,
    permission_mode: PermissionMode,
) -> Result<CliAction, String> {
    let raw = rest.join(" ");
    match SlashCommand::parse(&raw) {
        Ok(Some(SlashCommand::Help)) => Ok(CliAction::Help),
        Ok(Some(SlashCommand::Agents { args })) => Ok(CliAction::Agents { args }),
        Ok(Some(SlashCommand::Mcp { action, target })) => Ok(CliAction::Mcp {
            args: match (action, target) {
                (None, None) => None,
                (Some(action), None) => Some(action),
                (Some(action), Some(target)) => Some(format!("{action} {target}")),
                (None, Some(target)) => Some(target),
            },
        }),
        Ok(Some(SlashCommand::Skills { args })) => Ok(CliAction::Skills { args }),
        Ok(Some(SlashCommand::Pwd | SlashCommand::Workspace { path: None })) => {
            let _ = (model, permission_mode);
            Ok(CliAction::Workspace { output_format: CliOutputFormat::Text })
        }
        Ok(Some(SlashCommand::Cd { .. } | SlashCommand::Workspace { path: Some(_) })) => Err(
            format!(
                "slash command {command_name} changes the live workspace and is REPL-only. Start `sego` and run it there, or launch directly with `sego --cwd <path>`.",
                command_name = rest[0],
            ),
        ),
        Ok(Some(SlashCommand::Review { scope })) => {
            parse_code_review_slash_action(scope.as_deref(), model, allowed_tools)
        }
        Ok(Some(SlashCommand::Verify { scope })) => Ok(CliAction::CodeVerify { scope }),
        Ok(Some(SlashCommand::Unknown(name))) => Err(format_unknown_direct_slash_command(&name)),
        Ok(Some(command)) => Err({
            let _ = command;
            format!(
                "slash command {command_name} is interactive-only. Start `claw` and run it there, or use `claw --resume SESSION.jsonl {command_name}` / `claw --resume {latest} {command_name}` when the command is marked [resume] in /help.",
                command_name = rest[0],
                latest = LATEST_SESSION_REFERENCE,
            )
        }),
        Ok(None) => Err(format!("unknown subcommand: {}", rest[0])),
        Err(error) => Err(error.to_string()),
    }
}

fn parse_code_review_slash_action(
    scope: Option<&str>,
    model: String,
    allowed_tools: Option<AllowedToolSet>,
) -> Result<CliAction, String> {
    if let Some(command) = parse_review_history_command(scope).map_err(|error| error.to_string())? {
        return Ok(review_history_command_to_cli_action(command));
    }

    let Some(scope_value) = scope.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(CliAction::CodeReview {
            scope: None,
            model,
            allowed_tools,
            permission_mode: PermissionMode::ReadOnly,
        });
    };

    Ok(CliAction::CodeReview {
        scope: Some(scope_value.to_string()),
        model,
        allowed_tools,
        permission_mode: PermissionMode::ReadOnly,
    })
}

fn format_unknown_option(option: &str) -> String {
    let mut message = format!("unknown option: {option}");
    if let Some(suggestion) = suggest_closest_term(option, CLI_OPTION_SUGGESTIONS) {
        message.push_str("\nDid you mean ");
        message.push_str(suggestion);
        message.push('?');
    }
    message.push_str("\nRun `sego --help` for usage.");
    message
}

fn format_unknown_direct_slash_command(name: &str) -> String {
    let mut message = format!("unknown slash command outside the REPL: /{name}");
    if let Some(suggestions) = render_suggestion_line("Did you mean", &suggest_slash_commands(name))
    {
        message.push('\n');
        message.push_str(&suggestions);
    }
    message.push_str("\nRun `sego --help` for CLI usage, or start `sego` and use /help.");
    message
}

fn format_unknown_slash_command(name: &str) -> String {
    let mut message = format!("Unknown slash command: /{name}");
    if let Some(suggestions) = render_suggestion_line("Did you mean", &suggest_slash_commands(name))
    {
        message.push('\n');
        message.push_str(&suggestions);
    }
    message.push_str("\n  Help             /help lists available slash commands");
    message
}

fn resolve_cli_cwd(value: &str) -> Result<PathBuf, String> {
    let trimmed = value.trim().trim_matches('"');
    if trimmed.is_empty() {
        return Err("workspace path must not be empty".to_string());
    }
    let raw = PathBuf::from(trimmed);
    let candidate = if raw.is_absolute() {
        raw
    } else {
        env::current_dir().map_err(|error| error.to_string())?.join(raw)
    };
    if !candidate.exists() {
        return Err(format!("workspace path does not exist: {}", candidate.display()));
    }
    if !candidate.is_dir() {
        return Err(format!("workspace path is not a directory: {}", candidate.display()));
    }
    candidate.canonicalize().map_err(|error| {
        format!("failed to normalize workspace path {}: {error}", candidate.display())
    })
}

fn apply_requested_cwd(cwd: Option<&PathBuf>) -> Result<(), String> {
    if let Some(cwd) = cwd {
        env::set_current_dir(cwd)
            .map_err(|error| format!("failed to switch --cwd to {}: {error}", cwd.display()))?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkspaceIntent {
    Show,
    Switch(String),
}

fn parse_workspace_natural_language_intent(input: &str) -> Option<WorkspaceIntent> {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "pwd" | "cwd" | "where am i" | "show workspace" | "current workspace"
    ) || trimmed.contains("当前目录")
        || trimmed.contains("工作目录")
        || trimmed.contains("当前工作区")
    {
        return Some(WorkspaceIntent::Show);
    }

    for prefix in [
        "切换到",
        "切换工作区到",
        "切换工作目录到",
        "打开项目",
        "进入项目",
        "进入目录",
        "把工作目录切到",
        "change directory to",
        "switch workspace to",
        "open project",
        "use workspace",
        "cd ",
    ] {
        if let Some(path) = strip_intent_prefix(trimmed, prefix) {
            return Some(WorkspaceIntent::Switch(path.to_string()));
        }
    }

    None
}

fn strip_intent_prefix<'a>(input: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_ascii() {
        let lower = input.to_ascii_lowercase();
        let lower_prefix = prefix.to_ascii_lowercase();
        if lower.starts_with(&lower_prefix) {
            return Some(input[prefix.len()..].trim().trim_matches('"'));
        }
        return None;
    }
    input.strip_prefix(prefix).map(|path| path.trim().trim_matches('"'))
}

fn render_suggestion_line(label: &str, suggestions: &[String]) -> Option<String> {
    (!suggestions.is_empty()).then(|| format!("  {label:<16} {}", suggestions.join(", ")))
}

fn suggest_slash_commands(input: &str) -> Vec<String> {
    let mut candidates = slash_command_specs()
        .iter()
        .flat_map(|spec| {
            std::iter::once(spec.name)
                .chain(spec.aliases.iter().copied())
                .map(|name| format!("/{name}"))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();
    let candidate_refs = candidates.iter().map(String::as_str).collect::<Vec<_>>();
    ranked_suggestions(input.trim_start_matches('/'), &candidate_refs)
        .into_iter()
        .map(str::to_string)
        .collect()
}

fn suggest_closest_term<'a>(input: &str, candidates: &'a [&'a str]) -> Option<&'a str> {
    ranked_suggestions(input, candidates).into_iter().next()
}

fn ranked_suggestions<'a>(input: &str, candidates: &'a [&'a str]) -> Vec<&'a str> {
    let normalized_input = input.trim_start_matches('/').to_ascii_lowercase();
    let mut ranked = candidates
        .iter()
        .filter_map(|candidate| {
            let normalized_candidate = candidate.trim_start_matches('/').to_ascii_lowercase();
            let distance = levenshtein_distance(&normalized_input, &normalized_candidate);
            let prefix_bonus = usize::from(
                !(normalized_candidate.starts_with(&normalized_input)
                    || normalized_input.starts_with(&normalized_candidate)),
            );
            let score = distance + prefix_bonus;
            (score <= 4).then_some((score, *candidate))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| left.cmp(right).then_with(|| left.1.cmp(right.1)));
    ranked.into_iter().map(|(_, candidate)| candidate).take(3).collect()
}

fn levenshtein_distance(left: &str, right: &str) -> usize {
    if left.is_empty() {
        return right.chars().count();
    }
    if right.is_empty() {
        return left.chars().count();
    }

    let right_chars = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut current = vec![0; right_chars.len() + 1];

    for (left_index, left_char) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right_chars.iter().enumerate() {
            let substitution_cost = usize::from(left_char != *right_char);
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + substitution_cost);
        }
        previous.clone_from(&current);
    }

    previous[right_chars.len()]
}

fn resolve_model_alias(model: &str) -> String {
    api::resolve_model_alias(model)
}

fn normalize_allowed_tools(values: &[String]) -> Result<Option<AllowedToolSet>, String> {
    if values.is_empty() {
        return Ok(None);
    }
    current_tool_registry()?.normalize_allowed_tools(values)
}

fn current_tool_registry() -> Result<GlobalToolRegistry, String> {
    let cwd = env::current_dir().map_err(|error| error.to_string())?;
    let loader = ConfigLoader::default_for(&cwd);
    let runtime_config = loader.load().map_err(|error| error.to_string())?;
    let state = build_runtime_plugin_state_with_loader(&cwd, &loader, &runtime_config)
        .map_err(|error| error.to_string())?;
    let registry = state.tool_registry.clone();
    if let Some(mcp_state) = state.mcp_state {
        mcp_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .shutdown()
            .map_err(|error| error.to_string())?;
    }
    Ok(registry)
}

fn parse_permission_mode_arg(value: &str) -> Result<PermissionMode, String> {
    normalize_permission_mode(value)
        .ok_or_else(|| {
            format!(
                "unsupported permission mode '{value}'. Use read-only, workspace-write, or danger-full-access."
            )
        })
        .map(permission_mode_from_label)
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
        .unwrap_or(PermissionMode::DangerFullAccess)
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
    if let Some(ref set) = allowed_tools {
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

fn parse_system_prompt_args(args: &[String]) -> Result<CliAction, String> {
    let mut cwd = env::current_dir().map_err(|error| error.to_string())?;
    let mut date = default_date();
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--cwd" => {
                let value =
                    args.get(index + 1).ok_or_else(|| "missing value for --cwd".to_string())?;
                cwd = PathBuf::from(value);
                index += 2;
            }
            "--date" => {
                let value =
                    args.get(index + 1).ok_or_else(|| "missing value for --date".to_string())?;
                date.clone_from(value);
                index += 2;
            }
            other => return Err(format!("unknown system-prompt option: {other}")),
        }
    }

    Ok(CliAction::PrintSystemPrompt { cwd, date })
}

fn parse_resume_args(args: &[String]) -> Result<CliAction, String> {
    let (session_path, command_tokens): (PathBuf, &[String]) = match args.first() {
        None => (PathBuf::from(LATEST_SESSION_REFERENCE), &[]),
        Some(first) if looks_like_slash_command_token(first) => {
            (PathBuf::from(LATEST_SESSION_REFERENCE), args)
        }
        Some(first) => (PathBuf::from(first), &args[1..]),
    };
    let mut commands = Vec::new();
    let mut current_command = String::new();

    for token in command_tokens {
        if token.trim_start().starts_with('/') {
            if resume_command_can_absorb_token(&current_command, token) {
                current_command.push(' ');
                current_command.push_str(token);
                continue;
            }
            if !current_command.is_empty() {
                commands.push(current_command);
            }
            current_command = String::from(token.as_str());
            continue;
        }

        if current_command.is_empty() {
            return Err("--resume trailing arguments must be slash commands".to_string());
        }

        current_command.push(' ');
        current_command.push_str(token);
    }

    if !current_command.is_empty() {
        commands.push(current_command);
    }

    Ok(CliAction::ResumeSession { session_path, commands })
}

fn resume_command_can_absorb_token(current_command: &str, token: &str) -> bool {
    matches!(SlashCommand::parse(current_command), Ok(Some(SlashCommand::Export { path: None })))
        && !looks_like_slash_command_token(token)
}

fn looks_like_slash_command_token(token: &str) -> bool {
    let trimmed = token.trim_start();
    let Some(name) = trimmed.strip_prefix('/').and_then(|value| {
        value.split_whitespace().next().map(str::trim).filter(|value| !value.is_empty())
    }) else {
        return false;
    };

    slash_command_specs().iter().any(|spec| spec.name == name || spec.aliases.contains(&name))
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
}

#[derive(Debug, Clone)]
struct ResumeCommandOutcome {
    session: Session,
    message: Option<String>,
}

#[derive(Debug, Clone)]
struct StatusContext {
    cwd: PathBuf,
    session_path: Option<PathBuf>,
    loaded_config_files: usize,
    discovered_config_files: usize,
    memory_file_count: usize,
    project_root: Option<PathBuf>,
    git_branch: Option<String>,
    git_summary: GitWorkspaceSummary,
    sandbox_status: runtime::SandboxStatus,
}

struct WorkspaceContext {
    cwd: PathBuf,
    project_root: Option<PathBuf>,
    session_dir: PathBuf,
    recovery_dir: PathBuf,
    sandbox_status: runtime::SandboxStatus,
}

#[derive(Debug, Clone, Copy)]
struct StatusUsage {
    message_count: usize,
    turns: u32,
    latest: TokenUsage,
    cumulative: TokenUsage,
    estimated_tokens: usize,
}

#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct GitWorkspaceSummary {
    changed_files: usize,
    staged_files: usize,
    unstaged_files: usize,
    untracked_files: usize,
    conflicted_files: usize,
}

impl GitWorkspaceSummary {
    fn is_clean(self) -> bool {
        self.changed_files == 0
    }

    fn headline(self) -> String {
        if self.is_clean() {
            "clean".to_string()
        } else {
            let mut details = Vec::new();
            if self.staged_files > 0 {
                details.push(format!("{} staged", self.staged_files));
            }
            if self.unstaged_files > 0 {
                details.push(format!("{} unstaged", self.unstaged_files));
            }
            if self.untracked_files > 0 {
                details.push(format!("{} untracked", self.untracked_files));
            }
            if self.conflicted_files > 0 {
                details.push(format!("{} conflicted", self.conflicted_files));
            }
            format!("dirty · {} files · {}", self.changed_files, details.join(", "))
        }
    }
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

fn format_model_report(model: &str, message_count: usize, turns: u32) -> String {
    let models = [
        ("deepseek-chat", "DeepSeek Chat (主力国产)"),
        ("deepseek-v4-pro", "DeepSeek V4 Pro"),
        ("mimo-v2.5-pro", "MiMo V2.5 Pro (月之暗面)"),
        ("gpt-4.1", "GPT-4.1 (ChatGPT)"),
    ];

    let current_label = "\u{25cf} current";
    let available_label = "\u{25cb} available";

    let model_list = models
        .iter()
        .map(|(name, desc)| {
            let marker = if model == *name { current_label } else { available_label };
            format!("  {name:<20} {marker:<12} {desc}")
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "Model
  Current model    {model}
  Session messages {message_count}
  Session turns    {turns}

Available models
{model_list}

Usage
  Inspect current model with /model
  Switch models with /model <name>"
    )
}

fn format_model_switch_report(previous: &str, next: &str, message_count: usize) -> String {
    format!(
        "Model updated
  Previous         {previous}
  Current          {next}
  Preserved msgs   {message_count}"
    )
}

fn format_permissions_report(mode: &str) -> String {
    let modes = [
        ("read-only", "Read/search tools only", mode == "read-only"),
        ("workspace-write", "Edit files inside the workspace", mode == "workspace-write"),
        ("danger-full-access", "Unrestricted tool access", mode == "danger-full-access"),
    ]
    .into_iter()
    .map(|(name, description, is_current)| {
        let marker = if is_current { "● current" } else { "○ available" };
        format!("  {name:<18} {marker:<11} {description}")
    })
    .collect::<Vec<_>>()
    .join(
        "
",
    );

    format!(
        "Permissions
  Active mode      {mode}
  Mode status      live session default

Modes
{modes}

Usage
  Inspect current mode with /permissions
  Switch modes with /permissions <mode>"
    )
}

fn format_permissions_switch_report(previous: &str, next: &str) -> String {
    format!(
        "Permissions updated
  Result           mode switched
  Previous mode    {previous}
  Active mode      {next}
  Applies to       subsequent tool calls
  Usage            /permissions to inspect current mode"
    )
}

fn format_cost_report(usage: TokenUsage) -> String {
    format!(
        "Cost
  Input tokens     {}
  Output tokens    {}
  Cache create     {}
  Cache read       {}
  Total tokens     {}",
        usage.input_tokens,
        usage.output_tokens,
        usage.cache_creation_input_tokens,
        usage.cache_read_input_tokens,
        usage.total_tokens(),
    )
}

fn format_resume_report(session_path: &str, message_count: usize, turns: u32) -> String {
    format!(
        "Session resumed
  Session file     {session_path}
  Messages         {message_count}
  Turns            {turns}"
    )
}

fn render_resume_usage() -> String {
    format!(
        "Resume
  Usage            /resume <session-path|session-id|{LATEST_SESSION_REFERENCE}>
  Auto-save        .claw/sessions/<session-id>.{PRIMARY_SESSION_EXTENSION}
  Tip              use /session list to inspect saved sessions"
    )
}

fn format_compact_report(removed: usize, resulting_messages: usize, skipped: bool) -> String {
    if skipped {
        format!(
            "Compact
  Result           skipped
  Reason           session below compaction threshold
  Messages kept    {resulting_messages}"
        )
    } else {
        format!(
            "Compact
  Result           compacted
  Messages removed {removed}
  Messages kept    {resulting_messages}"
        )
    }
}

fn format_auto_compaction_notice(removed: usize) -> String {
    format!("[auto-compacted: removed {removed} messages]")
}

fn parse_git_status_metadata(status: Option<&str>) -> (Option<PathBuf>, Option<String>) {
    parse_git_status_metadata_for(
        &env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        status,
    )
}

fn parse_git_status_branch(status: Option<&str>) -> Option<String> {
    let status = status?;
    let first_line = status.lines().next()?;
    let line = first_line.strip_prefix("## ")?;
    if line.starts_with("HEAD") {
        return Some("detached HEAD".to_string());
    }
    let branch = line.split(['.', ' ']).next().unwrap_or_default().trim();
    if branch.is_empty() {
        None
    } else {
        Some(branch.to_string())
    }
}

fn parse_git_workspace_summary(status: Option<&str>) -> GitWorkspaceSummary {
    let mut summary = GitWorkspaceSummary::default();
    let Some(status) = status else {
        return summary;
    };

    for line in status.lines() {
        if line.starts_with("## ") || line.trim().is_empty() {
            continue;
        }

        summary.changed_files += 1;
        let mut chars = line.chars();
        let index_status = chars.next().unwrap_or(' ');
        let worktree_status = chars.next().unwrap_or(' ');

        if index_status == '?' && worktree_status == '?' {
            summary.untracked_files += 1;
            continue;
        }

        if index_status != ' ' {
            summary.staged_files += 1;
        }
        if worktree_status != ' ' {
            summary.unstaged_files += 1;
        }
        if (matches!(index_status, 'U' | 'A') && matches!(worktree_status, 'U' | 'A'))
            || index_status == 'U'
            || worktree_status == 'U'
        {
            summary.conflicted_files += 1;
        }
    }

    summary
}

fn resolve_git_branch_for(cwd: &Path) -> Option<String> {
    let branch = run_git_capture_in(cwd, &["branch", "--show-current"])?;
    let branch = branch.trim();
    if !branch.is_empty() {
        return Some(branch.to_string());
    }

    let fallback = run_git_capture_in(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let fallback = fallback.trim();
    if fallback.is_empty() {
        None
    } else if fallback == "HEAD" {
        Some("detached HEAD".to_string())
    } else {
        Some(fallback.to_string())
    }
}

fn run_git_capture_in(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git").args(args).current_dir(cwd).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn find_git_root_in(cwd: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()?;
    if !output.status.success() {
        return Err("not a git repository".into());
    }
    let path = String::from_utf8(output.stdout)?.trim().to_string();
    if path.is_empty() {
        return Err("empty git root".into());
    }
    Ok(PathBuf::from(path))
}

fn parse_git_status_metadata_for(
    cwd: &Path,
    status: Option<&str>,
) -> (Option<PathBuf>, Option<String>) {
    let branch = resolve_git_branch_for(cwd).or_else(|| parse_git_status_branch(status));
    let project_root = find_git_root_in(cwd).ok();
    (project_root, branch)
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

fn run_repl(
    model: String,
    allowed_tools: Option<AllowedToolSet>,
    permission_mode: PermissionMode,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut cli = LiveCli::new(model, true, allowed_tools, permission_mode)?;
    let mut editor =
        input::LineEditor::new("> ", cli.repl_completion_candidates().unwrap_or_default());
    println!("{}", cli.startup_banner());

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
                    break;
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
                if let Some(intent) = parse_workspace_natural_language_intent(&trimmed) {
                    match intent {
                        WorkspaceIntent::Show => LiveCli::print_workspace_status()?,
                        WorkspaceIntent::Switch(path) => {
                            cli.switch_workspace(&path)?;
                        }
                    }
                    continue;
                }
                cli.run_turn(&trimmed)?;
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

struct RuntimeMcpState {
    runtime: tokio::runtime::Runtime,
    manager: McpServerManager,
    pending_servers: Vec<String>,
    degraded_report: Option<runtime::McpDegradedReport>,
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

impl RuntimeMcpState {
    fn new(
        runtime_config: &runtime::RuntimeConfig,
    ) -> Result<Option<(Self, runtime::McpToolDiscoveryReport)>, Box<dyn std::error::Error>> {
        let mut manager = McpServerManager::from_runtime_config(runtime_config);
        if manager.server_names().is_empty() && manager.unsupported_servers().is_empty() {
            return Ok(None);
        }

        let runtime = tokio::runtime::Runtime::new()?;
        let discovery = runtime.block_on(manager.discover_tools_best_effort());
        let pending_servers = discovery
            .failed_servers
            .iter()
            .map(|failure| failure.server_name.clone())
            .chain(discovery.unsupported_servers.iter().map(|server| server.server_name.clone()))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let available_tools =
            discovery.tools.iter().map(|tool| tool.qualified_name.clone()).collect::<Vec<_>>();
        let failed_server_names = pending_servers.iter().cloned().collect::<BTreeSet<_>>();
        let working_servers = manager
            .server_names()
            .into_iter()
            .filter(|server_name| !failed_server_names.contains(server_name))
            .collect::<Vec<_>>();
        let failed_servers = discovery
            .failed_servers
            .iter()
            .map(|failure| runtime::McpFailedServer {
                server_name: failure.server_name.clone(),
                phase: runtime::McpLifecyclePhase::ToolDiscovery,
                error: runtime::McpErrorSurface::new(
                    runtime::McpLifecyclePhase::ToolDiscovery,
                    Some(failure.server_name.clone()),
                    failure.error.clone(),
                    std::collections::BTreeMap::new(),
                    true,
                ),
            })
            .chain(discovery.unsupported_servers.iter().map(|server| runtime::McpFailedServer {
                server_name: server.server_name.clone(),
                phase: runtime::McpLifecyclePhase::ServerRegistration,
                error: runtime::McpErrorSurface::new(
                    runtime::McpLifecyclePhase::ServerRegistration,
                    Some(server.server_name.clone()),
                    server.reason.clone(),
                    std::collections::BTreeMap::from([(
                        "transport".to_string(),
                        format!("{:?}", server.transport).to_ascii_lowercase(),
                    )]),
                    false,
                ),
            }))
            .collect::<Vec<_>>();
        let degraded_report = (!failed_servers.is_empty()).then(|| {
            runtime::McpDegradedReport::new(
                working_servers,
                failed_servers,
                available_tools.clone(),
                available_tools,
            )
        });

        Ok(Some((Self { runtime, manager, pending_servers, degraded_report }, discovery)))
    }

    fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.runtime.block_on(self.manager.shutdown())?;
        Ok(())
    }

    fn pending_servers(&self) -> Option<Vec<String>> {
        (!self.pending_servers.is_empty()).then(|| self.pending_servers.clone())
    }

    fn degraded_report(&self) -> Option<runtime::McpDegradedReport> {
        self.degraded_report.clone()
    }

    fn server_names(&self) -> Vec<String> {
        self.manager.server_names()
    }

    fn call_tool(
        &mut self,
        qualified_tool_name: &str,
        arguments: Option<serde_json::Value>,
    ) -> Result<String, ToolError> {
        let response = self
            .runtime
            .block_on(self.manager.call_tool(qualified_tool_name, arguments))
            .map_err(|error| ToolError::new(error.to_string()))?;
        if let Some(error) = response.error {
            return Err(ToolError::new(format!(
                "MCP tool `{qualified_tool_name}` returned JSON-RPC error: {} ({})",
                error.message, error.code
            )));
        }

        let result = response.result.ok_or_else(|| {
            ToolError::new(format!("MCP tool `{qualified_tool_name}` returned no result payload"))
        })?;
        serde_json::to_string_pretty(&result).map_err(|error| ToolError::new(error.to_string()))
    }

    fn list_resources_for_server(&mut self, server_name: &str) -> Result<String, ToolError> {
        let result = self
            .runtime
            .block_on(self.manager.list_resources(server_name))
            .map_err(|error| ToolError::new(error.to_string()))?;
        serde_json::to_string_pretty(&json!({
            "server": server_name,
            "resources": result.resources,
        }))
        .map_err(|error| ToolError::new(error.to_string()))
    }

    fn list_resources_for_all_servers(&mut self) -> Result<String, ToolError> {
        let mut resources = Vec::new();
        let mut failures = Vec::new();

        for server_name in self.server_names() {
            match self.runtime.block_on(self.manager.list_resources(&server_name)) {
                Ok(result) => resources.push(json!({
                    "server": server_name,
                    "resources": result.resources,
                })),
                Err(error) => failures.push(json!({
                    "server": server_name,
                    "error": error.to_string(),
                })),
            }
        }

        if resources.is_empty() && !failures.is_empty() {
            let message = failures
                .iter()
                .filter_map(|failure| failure.get("error").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(ToolError::new(message));
        }

        serde_json::to_string_pretty(&json!({
            "resources": resources,
            "failures": failures,
        }))
        .map_err(|error| ToolError::new(error.to_string()))
    }

    fn read_resource(&mut self, server_name: &str, uri: &str) -> Result<String, ToolError> {
        let result = self
            .runtime
            .block_on(self.manager.read_resource(server_name, uri))
            .map_err(|error| ToolError::new(error.to_string()))?;
        serde_json::to_string_pretty(&json!({
            "server": server_name,
            "contents": result.contents,
        }))
        .map_err(|error| ToolError::new(error.to_string()))
    }
}

fn build_runtime_mcp_state(
    runtime_config: &runtime::RuntimeConfig,
) -> Result<
    (Option<Arc<Mutex<RuntimeMcpState>>>, Vec<RuntimeToolDefinition>),
    Box<dyn std::error::Error>,
> {
    let Some((mcp_state, discovery)) = RuntimeMcpState::new(runtime_config)? else {
        return Ok((None, Vec::new()));
    };

    let mut runtime_tools =
        discovery.tools.iter().map(mcp_runtime_tool_definition).collect::<Vec<_>>();
    if !mcp_state.server_names().is_empty() {
        runtime_tools.extend(mcp_wrapper_tool_definitions());
    }

    Ok((Some(Arc::new(Mutex::new(mcp_state))), runtime_tools))
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

fn mcp_wrapper_tool_definitions() -> Vec<RuntimeToolDefinition> {
    vec![
        RuntimeToolDefinition {
            name: "MCPTool".to_string(),
            description: Some(
                "Call a configured MCP tool by its qualified name and JSON arguments.".to_string(),
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "qualifiedName": { "type": "string" },
                    "arguments": {}
                },
                "required": ["qualifiedName"],
                "additionalProperties": false
            }),
            required_permission: PermissionMode::DangerFullAccess,
        },
        RuntimeToolDefinition {
            name: "ListMcpResourcesTool".to_string(),
            description: Some(
                "List MCP resources from one configured server or from every connected server."
                    .to_string(),
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string" }
                },
                "additionalProperties": false
            }),
            required_permission: PermissionMode::ReadOnly,
        },
        RuntimeToolDefinition {
            name: "ReadMcpResourceTool".to_string(),
            description: Some("Read a specific MCP resource from a configured server.".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "server": { "type": "string" },
                    "uri": { "type": "string" }
                },
                "required": ["server", "uri"],
                "additionalProperties": false
            }),
            required_permission: PermissionMode::ReadOnly,
        },
    ]
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
    fn spawn(abort_signal: runtime::HookAbortSignal) -> Self {
        Self::spawn_with_waiter(abort_signal, move |stop_rx, abort_signal| {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build()
            else {
                return;
            };

            runtime.block_on(async move {
                let wait_for_stop = tokio::task::spawn_blocking(move || {
                    let _ = stop_rx.recv();
                });

                tokio::select! {
                    result = tokio::signal::ctrl_c() => {
                        if result.is_ok() {
                            abort_signal.abort();
                        }
                    }
                    _ = wait_for_stop => {}
                }
            });
        })
    }

    fn spawn_with_waiter<F>(abort_signal: runtime::HookAbortSignal, wait_for_interrupt: F) -> Self
    where
        F: FnOnce(Receiver<()>, runtime::HookAbortSignal) + Send + 'static,
    {
        let (stop_tx, stop_rx) = mpsc::channel();
        let join_handle = thread::spawn(move || wait_for_interrupt(stop_rx, abort_signal));

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
        if next == previous {
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
        .with_hook_abort_signal(hook_abort_signal.clone());
        let hook_abort_monitor = HookAbortMonitor::spawn(hook_abort_signal);

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

    fn run_turn_capture_text(&mut self, input: &str) -> Result<String, Box<dyn std::error::Error>> {
        let machine = self.machine_output;
        // C-light (D-IDE-1): machine mode uses emit_output=false to suppress
        // conversation runtime rendering, and a fail-closed prompter that never
        // writes to stdout or reads from stdin.
        let (mut runtime, hook_abort_monitor) = self.prepare_turn_runtime(!machine)?;
        if machine {
            // Machine mode: no ProgressUI, no stdout output, fail-closed permissions.
            let mut prompter = MachinePermissionPrompter;
            let result = runtime.run_turn(input, Some(&mut prompter));
            hook_abort_monitor.stop();
            match result {
                Ok(summary) => {
                    let text = final_assistant_text(&summary);
                    self.replace_runtime(runtime)?;
                    self.persist_session()?;
                    Ok(text)
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
                    Ok(text)
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

        let model = resolve_model_alias(&model).to_string();

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

    fn run_bughunter(&self, scope: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
        println!("{}", format_bughunter_report(scope));
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
        let review_scope = ReviewScope::parse(scope)?;
        let target = collect_review_target(&env::current_dir()?, review_scope)?;
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
        let context = ReviewContext::new(target);
        let prompt = build_review_prompt(&context, ReviewPromptOptions::default());
        let review_text = self.run_turn_capture_text(&prompt)?;
        let report = ReviewReport::from_model_output(review_text);
        let artifact = persist_review_artifact(&env::current_dir()?, &context.target, &report)?;
        let highest_severity =
            report.highest_severity().map_or("none", runtime::ReviewSeverity::label);

        println!(
            "Review Report\n  ID               {}\n  Diff hash        {}\n  Parse status     {}\n  Findings         {}\n  Highest severity {}\n  Markdown         {}\n  JSON             {}\n  Index            {}",
            artifact.id,
            artifact.diff_hash,
            report.parse_status.label(),
            report.findings.len(),
            highest_severity,
            artifact.markdown_path.display(),
            artifact.json_path.display(),
            artifact.index_path.display()
        );
        Ok(())
    }
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

/// 启动提示层：parse_args 后调用，只读 recovery JSON，不写、不扫描、不调模型。
///
/// 仅对会创建/恢复可持久化 session 且可能执行模型/工具的动作触发提示
/// （Repl / Prompt / CodeReview / ResumeSession）。对 Version/Help 等纯查询动作不触发。
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
/// - 用户传 path → 尊重用户路径，不强制 .txt（区别于 /export 的 resolve_export_path）
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

fn format_missing_session_reference(reference: &str) -> String {
    format!(
        "session not found: {reference}\nHint: managed sessions live in .claw/sessions/. Try `{LATEST_SESSION_REFERENCE}` for the most recent session or `/session list` in the REPL."
    )
}

fn format_no_managed_sessions() -> String {
    format!(
        "no managed sessions found in .claw/sessions/\nStart `claw` to create a session, then rerun with `--resume {LATEST_SESSION_REFERENCE}`."
    )
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

fn format_session_modified_age(modified_epoch_millis: u128) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map_or(modified_epoch_millis, |duration| duration.as_millis());
    let delta_seconds =
        now.saturating_sub(modified_epoch_millis).checked_div(1_000).unwrap_or_default();
    match delta_seconds {
        0..=4 => "just-now".to_string(),
        5..=59 => format!("{delta_seconds}s-ago"),
        60..=3_599 => format!("{}m-ago", delta_seconds / 60),
        3_600..=86_399 => format!("{}h-ago", delta_seconds / 3_600),
        _ => format!("{}d-ago", delta_seconds / 86_400),
    }
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

fn render_repl_help() -> String {
    [
        "REPL".to_string(),
        "  /exit                Quit the REPL".to_string(),
        "  /quit                Quit the REPL".to_string(),
        "  Up/Down              Navigate prompt history".to_string(),
        "  Tab                  Complete commands, modes, and recent sessions".to_string(),
        "  Ctrl-C               Clear input (or exit on empty prompt)".to_string(),
        "  Shift+Enter/Ctrl+J   Insert a newline".to_string(),
        "  Auto-save            .claw/sessions/<session-id>.jsonl".to_string(),
        "  Resume latest        /resume latest".to_string(),
        "  Browse sessions      /session list".to_string(),
        "  Workspace            /workspace, /pwd, /cd <path>".to_string(),
        "  Natural workspace    say: 切换到 D:\\YourProject / 当前工作区".to_string(),
        String::new(),
        render_slash_command_help(),
    ]
    .join(
        "
",
    )
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

fn status_context(
    session_path: Option<&Path>,
) -> Result<StatusContext, Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let loader = ConfigLoader::default_for(&cwd);
    let discovered_config_files = loader.discover().len();
    let runtime_config = loader.load()?;
    let project_context = ProjectContext::discover_with_git(&cwd, default_date())?;
    let (project_root, git_branch) =
        parse_git_status_metadata(project_context.git_status.as_deref());
    let git_summary = parse_git_workspace_summary(project_context.git_status.as_deref());
    let sandbox_status = resolve_sandbox_status(runtime_config.sandbox(), &cwd);
    Ok(StatusContext {
        cwd,
        session_path: session_path.map(Path::to_path_buf),
        loaded_config_files: runtime_config.loaded_entries().len(),
        discovered_config_files,
        memory_file_count: project_context.instruction_files.len(),
        project_root,
        git_branch,
        git_summary,
        sandbox_status,
    })
}

fn workspace_context() -> Result<WorkspaceContext, Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let loader = ConfigLoader::default_for(&cwd);
    let runtime_config = loader.load().unwrap_or_else(|_| runtime::RuntimeConfig::empty());
    let sandbox_status = resolve_sandbox_status(runtime_config.sandbox(), &cwd);
    Ok(WorkspaceContext {
        project_root: find_git_root_in(&cwd).ok(),
        session_dir: cwd.join(".claw").join("sessions"),
        recovery_dir: runtime::recovery::recovery_dir(&cwd),
        cwd,
        sandbox_status,
    })
}

fn format_status_report(
    model: &str,
    usage: StatusUsage,
    permission_mode: &str,
    context: &StatusContext,
) -> String {
    [
        format!(
            "Status
  Model            {model}
  Permission mode  {permission_mode}
  Messages         {}
  Turns            {}
  Estimated tokens {}",
            usage.message_count, usage.turns, usage.estimated_tokens,
        ),
        format!(
            "Usage
  Latest total     {}
  Cumulative input {}
  Cumulative output {}
  Cumulative total {}",
            usage.latest.total_tokens(),
            usage.cumulative.input_tokens,
            usage.cumulative.output_tokens,
            usage.cumulative.total_tokens(),
        ),
        format!(
            "Workspace
  Cwd              {}
  Project root     {}
  Git branch       {}
  Git state        {}
  Changed files    {}
  Staged           {}
  Unstaged         {}
  Untracked        {}
  Session          {}
  Config files     loaded {}/{}
  Memory files     {}
  Suggested flow   /status → /diff → /commit",
            context.cwd.display(),
            context
                .project_root
                .as_ref()
                .map_or_else(|| "unknown".to_string(), |path| path.display().to_string()),
            context.git_branch.as_deref().unwrap_or("unknown"),
            context.git_summary.headline(),
            context.git_summary.changed_files,
            context.git_summary.staged_files,
            context.git_summary.unstaged_files,
            context.git_summary.untracked_files,
            context
                .session_path
                .as_ref()
                .map_or_else(|| "live-repl".to_string(), |path| path.display().to_string()),
            context.loaded_config_files,
            context.discovered_config_files,
            context.memory_file_count,
        ),
        format_sandbox_report(&context.sandbox_status),
    ]
    .join(
        "

",
    )
}

fn format_workspace_report(context: &WorkspaceContext) -> String {
    format!(
        "Workspace
  Active cwd       {}
  Project root     {}
  Session dir      {}
  Recovery dir     {}
  Filesystem mode  {}
  Allowed mounts   {}
  Natural input    say `切换到 D:\\YourProject` or `当前工作区`
  Command input    `sego --cwd <path>`, `/workspace`, `/cd <path>`",
        context.cwd.display(),
        context
            .project_root
            .as_ref()
            .map_or_else(|| "unknown".to_string(), |path| path.display().to_string()),
        context.session_dir.display(),
        context.recovery_dir.display(),
        context.sandbox_status.filesystem_mode.as_str(),
        if context.sandbox_status.allowed_mounts.is_empty() {
            "<workspace only>".to_string()
        } else {
            context.sandbox_status.allowed_mounts.join(", ")
        },
    )
}

fn format_workspace_switch_report(previous: &Path, next: &Path, switched: bool) -> String {
    if switched {
        format!(
            "Workspace switched
  Previous cwd     {}
  Active cwd       {}
  Session scope    new workspace-local session
  Config scope     reloaded from active cwd
  Tip              say `当前工作区` or run `/workspace` to inspect context",
            previous.display(),
            next.display(),
        )
    } else {
        format!(
            "Workspace unchanged
  Active cwd       {}
  Reason           requested path is already active",
            next.display(),
        )
    }
}

fn format_sandbox_report(status: &runtime::SandboxStatus) -> String {
    format!(
        "Sandbox
  Enabled           {}
  Active            {}
  Supported         {}
  In container      {}
  Requested ns      {}
  Active ns         {}
  Requested net     {}
  Active net        {}
  Filesystem mode   {}
  Filesystem active {}
  Allowed mounts    {}
  Markers           {}
  Fallback reason   {}",
        status.enabled,
        status.active,
        status.supported,
        status.in_container,
        status.requested.namespace_restrictions,
        status.namespace_active,
        status.requested.network_isolation,
        status.network_active,
        status.filesystem_mode.as_str(),
        status.filesystem_active,
        if status.allowed_mounts.is_empty() {
            "<none>".to_string()
        } else {
            status.allowed_mounts.join(", ")
        },
        if status.container_markers.is_empty() {
            "<none>".to_string()
        } else {
            status.container_markers.join(", ")
        },
        status.fallback_reason.clone().unwrap_or_else(|| "<none>".to_string()),
    )
}

fn format_commit_preflight_report(branch: Option<&str>, summary: GitWorkspaceSummary) -> String {
    format!(
        "Commit
  Result           ready
  Branch           {}
  Workspace        {}
  Changed files    {}
  Action           create a git commit from the current workspace changes",
        branch.unwrap_or("unknown"),
        summary.headline(),
        summary.changed_files,
    )
}

fn format_commit_skipped_report() -> String {
    "Commit
  Result           skipped
  Reason           no workspace changes
  Action           create a git commit from the current workspace changes
  Next             /status to inspect context · /diff to inspect repo changes"
        .to_string()
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
            println!("  Config files:     {} loaded", runtime_config.loaded_entries().len());
            if let Ok(status) = status_context(None) {
                println!(
                    "  Git branch:       {}",
                    status.git_branch.as_deref().unwrap_or("unknown")
                );
                println!("  Git state:        {}", status.git_summary.headline());
            }
            println!();
            println!("  Run `sego review` for session analysis.");
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

fn run_update() -> Result<(), Box<dyn std::error::Error>> {
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

    let script_path = install_dir.join("sego-update.cmd");
    let backup_exe = install_dir.join("sego.previous.exe");
    let script = format!(
        "@echo off\r\n\
         setlocal EnableExtensions\r\n\
         echo Updating Sego to {tag}...\r\n\
         timeout /t 1 /nobreak >nul\r\n\
         if exist \"{backup}\" del /f /q \"{backup}\"\r\n\
         if exist \"{current}\" move /y \"{current}\" \"{backup}\" >nul\r\n\
         move /y \"{temp}\" \"{current}\" >nul\r\n\
         echo Sego updated to {tag}.\r\n\
         \"{current}\" --version\r\n\
         pause\r\n",
        tag = release.tag_name,
        backup = backup_exe.display(),
        current = current_exe.display(),
        temp = temp_exe.display(),
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

fn normalize_permission_mode(mode: &str) -> Option<&'static str> {
    match mode.trim() {
        "read-only" => Some("read-only"),
        "workspace-write" => Some("workspace-write"),
        "danger-full-access" => Some("danger-full-access"),
        _ => None,
    }
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
            let path = path.to_string_lossy();
            (
                run_git_diff_command_in(cwd, &["diff", "--cached", "--", path.as_ref()])?,
                run_git_diff_command_in(cwd, &["diff", "--", path.as_ref()])?,
            )
        }
    };

    Ok(ReviewTarget { scope, git_status, staged_diff, unstaged_diff })
}

fn collect_staged_review_paths(cwd: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let output =
        run_git_diff_command_in(cwd, &["diff", "--cached", "--name-only", "--diff-filter=ACMR"])?;
    Ok(output.lines().map(str::trim).filter(|line| !line.is_empty()).map(PathBuf::from).collect())
}

fn run_code_review_cli(
    model: String,
    allowed_tools: Option<AllowedToolSet>,
    permission_mode: PermissionMode,
    scope: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let review_scope = ReviewScope::parse(scope)?;
    let target = collect_review_target(&env::current_dir()?, review_scope)?;
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
    cli.run_review_target(target)?;
    persist_recovery_for_cli(
        runtime::recovery::RecoveryExitState::Graceful,
        cli.session_id(),
        cli.session_path(),
        Some(cli.model_name()),
        Some(scope.unwrap_or("code-review")),
    );
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReviewHistoryCommand {
    List,
    Show { id: String },
    Status { id: String },
    Mark { id: String, finding_id: String, status: ReviewFindingStatus, note: Option<String> },
    Ready,
    Summary,
    Tools,
    Safety { scope: SafetyReviewScope },
}

fn parse_review_history_command(
    scope: Option<&str>,
) -> Result<Option<ReviewHistoryCommand>, Box<dyn std::error::Error>> {
    let Some(scope_value) = scope.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let parts = scope_value.split_whitespace().collect::<Vec<_>>();
    match parts.as_slice() {
        ["list"] => Ok(Some(ReviewHistoryCommand::List)),
        ["ready"] => Ok(Some(ReviewHistoryCommand::Ready)),
        ["ready", ..] => Err("unexpected arguments for /review ready".into()),
        ["summary"] => Ok(Some(ReviewHistoryCommand::Summary)),
        ["summary", ..] => Err("unexpected arguments for /review summary".into()),
        ["tools"] => Ok(Some(ReviewHistoryCommand::Tools)),
        ["tools", ..] => Err("unexpected arguments for /review tools".into()),
        ["safety"] => {
            Ok(Some(ReviewHistoryCommand::Safety { scope: SafetyReviewScope::Workspace }))
        }
        ["safety", "staged"] => {
            Ok(Some(ReviewHistoryCommand::Safety { scope: SafetyReviewScope::Staged }))
        }
        ["safety", ..] => Err("unexpected arguments for /review safety".into()),
        ["show"] => Err("missing review id for /review show <id>".into()),
        ["show", id] => Ok(Some(ReviewHistoryCommand::Show { id: (*id).to_string() })),
        ["show", ..] => Err("unexpected arguments for /review show <id>".into()),
        ["status"] => Err("missing review id for /review status <id>".into()),
        ["status", id] => Ok(Some(ReviewHistoryCommand::Status { id: (*id).to_string() })),
        ["status", ..] => Err("unexpected arguments for /review status <id>".into()),
        ["mark"] | ["mark", _] | ["mark", _, _] => {
            Err("missing arguments for /review mark <review-id> <finding-id> <status> [note]"
                .into())
        }
        ["mark", id, finding_id, status] => Ok(Some(ReviewHistoryCommand::Mark {
            id: (*id).to_string(),
            finding_id: (*finding_id).to_string(),
            status: ReviewFindingStatus::parse(status)?,
            note: None,
        })),
        ["mark", id, finding_id, status, note @ ..] => Ok(Some(ReviewHistoryCommand::Mark {
            id: (*id).to_string(),
            finding_id: (*finding_id).to_string(),
            status: ReviewFindingStatus::parse(status)?,
            note: Some(note.join(" ")),
        })),
        ["list", ..] => Err("unexpected arguments for /review list".into()),
        _ => Ok(None),
    }
}

fn review_history_command_to_cli_action(command: ReviewHistoryCommand) -> CliAction {
    match command {
        ReviewHistoryCommand::List => CliAction::CodeReviewList,
        ReviewHistoryCommand::Show { id } => CliAction::CodeReviewShow { id },
        ReviewHistoryCommand::Status { id } => CliAction::CodeReviewStatus { id },
        ReviewHistoryCommand::Mark { id, finding_id, status, note } => {
            CliAction::CodeReviewMark { id, finding_id, status, note }
        }
        ReviewHistoryCommand::Ready => CliAction::CodeReviewReady,
        ReviewHistoryCommand::Summary => CliAction::CodeReviewSummary,
        ReviewHistoryCommand::Tools => CliAction::CodeReviewTools,
        ReviewHistoryCommand::Safety { scope } => CliAction::CodeReviewSafety { scope },
    }
}

fn run_review_history_command(
    command: ReviewHistoryCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        ReviewHistoryCommand::List => print_code_review_history(),
        ReviewHistoryCommand::Show { id } => print_code_review_report(&id),
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

struct CodeReviewSummaryReport {
    root: String,
    git_status: String,
    staged_diff: String,
    unstaged_diff: String,
    staged_paths: Vec<PathBuf>,
    safety_report: Option<runtime::SafetyLockReport>,
    verification_plan: runtime::VerificationPlan,
    latest_review: Option<ReviewIndexEntry>,
    latest_status_counts: Option<ReviewFindingStatusCounts>,
}

#[derive(Default)]
struct ReviewFindingStatusCounts {
    open: usize,
    acknowledged: usize,
    fixed: usize,
    ignored: usize,
}

impl ReviewFindingStatusCounts {
    fn from_entries(
        statuses: &std::collections::BTreeMap<String, runtime::ReviewFindingStatusEntry>,
    ) -> Self {
        let mut counts = Self::default();
        for entry in statuses.values() {
            match entry.status {
                ReviewFindingStatus::Open => counts.open += 1,
                ReviewFindingStatus::Acknowledged => counts.acknowledged += 1,
                ReviewFindingStatus::Fixed => counts.fixed += 1,
                ReviewFindingStatus::Ignored => counts.ignored += 1,
            }
        }
        counts
    }

    fn is_empty(&self) -> bool {
        self.open == 0 && self.acknowledged == 0 && self.fixed == 0 && self.ignored == 0
    }

    fn render(&self) -> String {
        format!(
            "open {}, acknowledged {}, fixed {}, ignored {}",
            self.open, self.acknowledged, self.fixed, self.ignored
        )
    }
}

impl CodeReviewSummaryReport {
    fn render(&self) -> String {
        let mut lines = Vec::new();
        let branch_status = self.git_status.lines().next().unwrap_or("(not a git repository)");

        lines.push("Review Summary".to_string());
        lines.push(format!("  Root             {}", self.root));
        lines.push("  Mode             read-only".to_string());
        lines.push("  Scope            workspace".to_string());
        lines.push(
            "  Safety           no files were modified; no model calls were made; no tools were executed"
                .to_string(),
        );

        self.push_git_section(&mut lines, branch_status);
        self.push_staged_safety_section(&mut lines);
        self.push_latest_review_section(&mut lines);
        self.push_verify_fast_section(&mut lines);
        self.push_suggested_next_steps(&mut lines);

        lines.join("\n")
    }

    fn push_git_section(&self, lines: &mut Vec<String>, branch_status: &str) {
        lines.push(String::new());
        lines.push("  Git".to_string());
        lines.push(format!("    Branch/status  {branch_status}"));
        lines.push(format!("    Staged files   {}", self.staged_paths.len()));
        lines.push(format!(
            "    Staged diff    {}",
            if self.staged_diff.trim().is_empty() { "no" } else { "yes" }
        ));
        lines.push(format!(
            "    Unstaged diff  {}",
            if self.unstaged_diff.trim().is_empty() { "no" } else { "yes" }
        ));
    }

    fn push_staged_safety_section(&self, lines: &mut Vec<String>) {
        lines.push(String::new());
        lines.push("  Staged safety".to_string());
        match &self.safety_report {
            None => {
                lines.push("    Result         no staged files".to_string());
                lines.push("    Findings       0".to_string());
            }
            Some(report) if report.findings.is_empty() => {
                lines.push("    Result         passed".to_string());
                lines.push("    Findings       0".to_string());
            }
            Some(report) => {
                let has_high_risk = report
                    .findings
                    .iter()
                    .any(|finding| matches!(finding.severity, runtime::SafetySeverity::High));
                lines.push(format!(
                    "    Result         {}",
                    if has_high_risk { "blocked" } else { "needs review" }
                ));
                lines.push(format!("    Findings       {}", report.findings.len()));
                for finding in report.findings.iter().take(8) {
                    let location = finding.line.map_or_else(
                        || finding.file.clone(),
                        |line| format!("{}:{line}", finding.file),
                    );
                    lines.push(format!(
                        "    [{}] {} {}",
                        finding.severity.label(),
                        location,
                        finding.title
                    ));
                }
                if report.findings.len() > 8 {
                    lines.push(format!("    ... {} more", report.findings.len() - 8));
                }
            }
        }
    }

    fn push_latest_review_section(&self, lines: &mut Vec<String>) {
        lines.push(String::new());
        lines.push("  Latest review".to_string());
        if let Some(entry) = &self.latest_review {
            let highest_severity =
                entry.highest_severity.map_or("none", runtime::ReviewSeverity::label);
            lines.push(format!("    ID             {}", entry.id));
            lines.push(format!("    Scope          {}", entry.scope));
            lines.push(format!("    Findings       {}", entry.finding_count));
            lines.push(format!("    Highest        {highest_severity}"));
            lines.push(format!("    Parse status   {}", entry.parse_status.label()));
            if let Some(counts) = &self.latest_status_counts {
                if !counts.is_empty() {
                    lines.push(format!("    Statuses       {}", counts.render()));
                }
            }
        } else {
            lines.push("    ID             none".to_string());
            lines.push("    Findings       0".to_string());
        }
    }

    fn push_verify_fast_section(&self, lines: &mut Vec<String>) {
        lines.push(String::new());
        lines.push("  Verify fast".to_string());
        match &self.verification_plan.status {
            VerificationPlanStatus::Ready => {
                lines.push(format!(
                    "    Plan           {} command(s)",
                    self.verification_plan.commands.len()
                ));
                for command in &self.verification_plan.commands {
                    lines.push(format!(
                        "    {} ({})",
                        command.display_command(),
                        command.working_dir
                    ));
                }
            }
            VerificationPlanStatus::NoPlan { reason } => {
                lines.push("    Plan           no plan".to_string());
                lines.push(format!("    Reason         {reason}"));
            }
        }
    }

    fn push_suggested_next_steps(&self, lines: &mut Vec<String>) {
        lines.push(String::new());
        lines.push("  Suggested next steps".to_string());
        lines.push("    sego /review ready".to_string());
        lines.push("    sego /review safety staged".to_string());
        lines.push("    sego /review staged".to_string());
        lines.push("    sego /verify fast".to_string());
    }
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

struct CodeReviewReadinessReport {
    root: String,
    staged_paths: Vec<PathBuf>,
    safety_report: runtime::SafetyLockReport,
    verification_plan: runtime::VerificationPlan,
}

impl CodeReviewReadinessReport {
    fn render(&self) -> String {
        let has_high_risk = self
            .safety_report
            .findings
            .iter()
            .any(|finding| finding.severity == runtime::SafetySeverity::High);
        let blocked = self.staged_paths.is_empty() || has_high_risk;
        let mut lines = Vec::new();

        lines.push("Review Readiness".to_string());
        lines.push(format!("  Root             {}", self.root));
        lines.push("  Mode             read-only".to_string());
        lines.push("  Scope            staged".to_string());
        lines.push(
            "  Safety           no files were modified; no model calls were made; no tools were executed"
                .to_string(),
        );
        lines.push(format!(
            "  Result           {}",
            if blocked { "blocked" } else { "ready with manual gates" }
        ));

        lines.push(format!("  Staged files     {}", self.staged_paths.len()));
        for path in self.staged_paths.iter().take(12) {
            lines.push(format!("    {}", path.display()));
        }
        if self.staged_paths.len() > 12 {
            lines.push(format!("    ... {} more", self.staged_paths.len() - 12));
        }

        if self.safety_report.findings.is_empty() {
            lines.push("  Safety lock      passed".to_string());
        } else {
            lines.push(format!(
                "  Safety lock      {} finding(s)",
                self.safety_report.findings.len()
            ));
            for finding in &self.safety_report.findings {
                let location = finding.line.map_or_else(
                    || finding.file.clone(),
                    |line| format!("{}:{line}", finding.file),
                );
                lines.push(format!(
                    "    [{}] {} ({})",
                    finding.severity.label(),
                    location,
                    finding.title
                ));
            }
        }

        match &self.verification_plan.status {
            VerificationPlanStatus::Ready => {
                lines.push(format!(
                    "  Verify fast      planned {} command(s)",
                    self.verification_plan.commands.len()
                ));
                for command in &self.verification_plan.commands {
                    lines.push(format!(
                        "    {} ({})",
                        command.display_command(),
                        command.working_dir
                    ));
                }
            }
            VerificationPlanStatus::NoPlan { reason } => {
                lines.push("  Verify fast      no plan".to_string());
                lines.push(format!("    Reason         {reason}"));
            }
        }

        lines.push("  Review gate      manual".to_string());
        lines.push("    Command        sego /review staged".to_string());
        lines.push("  Verification     manual".to_string());
        lines.push("    Command        sego /verify fast".to_string());

        if self.staged_paths.is_empty() {
            lines.push("  Next step        stage changes before running /review ready".to_string());
        } else if has_high_risk {
            lines.push(
                "  Next step        fix high-risk safety findings before review or commit"
                    .to_string(),
            );
        } else {
            lines.push(
                "  Next step        run /review staged, then /verify fast before commit"
                    .to_string(),
            );
        }

        lines.join("\n")
    }
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
    let entry = resolve_review_entry(&entries, id)?;
    let markdown_path = resolve_review_markdown_path(&cwd, entry);
    let markdown = fs::read_to_string(&markdown_path)?;
    println!("{markdown}");
    Ok(())
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

    Ok(ReviewReport { findings, raw_text, parse_status })
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

fn build_system_prompt() -> Result<Vec<String>, Box<dyn std::error::Error>> {
    Ok(load_system_prompt(env::current_dir()?, default_date(), env::consts::OS, "unknown")?)
}

fn build_runtime_plugin_state() -> Result<RuntimePluginState, Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let loader = ConfigLoader::default_for(&cwd);
    let runtime_config = loader.load()?;
    build_runtime_plugin_state_with_loader(&cwd, &loader, &runtime_config)
}

fn build_runtime_plugin_state_with_loader(
    cwd: &Path,
    loader: &ConfigLoader,
    runtime_config: &runtime::RuntimeConfig,
) -> Result<RuntimePluginState, Box<dyn std::error::Error>> {
    let plugin_manager = build_plugin_manager(cwd, loader, runtime_config);
    let plugin_registry = plugin_manager.plugin_registry()?;
    let plugin_hook_config =
        runtime_hook_config_from_plugin_hooks(plugin_registry.aggregated_hooks()?);
    let feature_config = runtime_config
        .feature_config()
        .clone()
        .with_hooks(runtime_config.hooks().merged(&plugin_hook_config));
    let (mcp_state, runtime_tools) = build_runtime_mcp_state(runtime_config)?;
    let tool_registry = GlobalToolRegistry::with_plugin_tools(plugin_registry.aggregated_tools()?)?
        .with_runtime_tools(runtime_tools)?;
    Ok(RuntimePluginState { feature_config, tool_registry, plugin_registry, mcp_state })
}

fn build_plugin_manager(
    cwd: &Path,
    loader: &ConfigLoader,
    runtime_config: &runtime::RuntimeConfig,
) -> PluginManager {
    let plugin_settings = runtime_config.plugins();
    let mut plugin_config = PluginManagerConfig::new(loader.config_home().to_path_buf());
    plugin_config.enabled_plugins = plugin_settings.enabled_plugins().clone();
    plugin_config.external_dirs = plugin_settings
        .external_directories()
        .iter()
        .map(|path| resolve_plugin_path(cwd, loader.config_home(), path))
        .collect();
    plugin_config.install_root = plugin_settings
        .install_root()
        .map(|path| resolve_plugin_path(cwd, loader.config_home(), path));
    plugin_config.registry_path = plugin_settings
        .registry_path()
        .map(|path| resolve_plugin_path(cwd, loader.config_home(), path));
    plugin_config.bundled_root = plugin_settings
        .bundled_root()
        .map(|path| resolve_plugin_path(cwd, loader.config_home(), path));
    PluginManager::new(plugin_config)
}

fn resolve_plugin_path(cwd: &Path, config_home: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else if value.starts_with('.') {
        cwd.join(path)
    } else {
        config_home.join(path)
    }
}

fn runtime_hook_config_from_plugin_hooks(hooks: PluginHooks) -> runtime::RuntimeHookConfig {
    runtime::RuntimeHookConfig::new(
        hooks.pre_tool_use,
        hooks.post_tool_use,
        hooks.post_tool_use_failure,
    )
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
        let estimated_tokens = (estimated_chars / 4) as u32;
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

/// Default token-saving tool set — only the 15 most-used tools
static LITE_TOOLS: std::sync::LazyLock<AllowedToolSet> = std::sync::LazyLock::new(|| {
    [
        "bash",
        "read",
        "write",
        "edit",
        "grep",
        "glob",
        "web_search",
        "web_fetch",
        "agent",
        "todo_write",
        "task",
        "ask_user_question",
        "skill",
        "notebook_edit",
        "enter_plan_mode",
        "exit_plan_mode",
    ]
    .iter()
    .map(|s| s.to_string())
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

    let exit_code = parsed.get("exitCode").and_then(|v| v.as_i64()).unwrap_or(0);
    let status = if exit_code == 0 { "" } else { &" \x1b[38;5;203merror\x1b[0m" };
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
            lines.push(format!("\x1b[2m{}\x1b[0m", trimmed));
        } else if !preview.is_empty() {
            lines.push(format!("\x1b[2m{}\x1b[0m", preview));
        }
    }

    if !stderr.trim().is_empty() {
        let stderr_preview = truncate_for_summary(stderr.trim(), 80);
        lines.push(format!("\x1b[38;5;203m{}\x1b[0m", stderr_preview));
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
    writeln!(out, "      Show workflow analysis for the most recent session")?;
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

#[cfg(test)]
mod tests {
    use super::{
        build_runtime_plugin_state_with_loader, build_runtime_with_plugin_state,
        create_managed_session_handle, default_model, describe_tool_progress, filter_tool_specs,
        format_bughunter_report, format_commit_preflight_report, format_commit_skipped_report,
        format_compact_report, format_cost_report, format_internal_prompt_progress_line,
        format_issue_report, format_model_report, format_model_switch_report,
        format_permissions_report, format_permissions_switch_report, format_pr_report,
        format_resume_report, format_status_report, format_tool_call_start, format_tool_result,
        format_ultraplan_report, format_unknown_slash_command,
        format_unknown_slash_command_message, normalize_permission_mode, parse_args,
        parse_git_status_branch, parse_git_status_metadata_for, parse_git_workspace_summary,
        parse_workspace_natural_language_intent, permission_policy, print_help_to,
        push_output_block, render_code_review_readiness_for, render_code_review_summary_for,
        render_config_report, render_diff_report, render_diff_report_for, render_memory_report,
        render_repl_help, render_resume_usage, resolve_model_alias, resolve_session_reference,
        response_to_events, resume_supported_slash_commands, run_resume_command,
        slash_command_completion_candidates_with_sessions, status_context, validate_no_args,
        write_mcp_server_fixture, CliAction, CliOutputFormat, CliToolExecutor, GitWorkspaceSummary,
        InternalPromptProgressEvent, InternalPromptProgressState, LiveCli, SafetyReviewScope,
        SlashCommand, StatusUsage, WorkspaceIntent,
    };
    use api::{MessageResponse, OutputContentBlock, Usage};
    use plugins::{
        PluginManager, PluginManagerConfig, PluginTool, PluginToolDefinition, PluginToolPermission,
    };
    use runtime::ReviewFindingStatus;
    use runtime::{
        AssistantEvent, ConfigLoader, ContentBlock, ConversationMessage, MessageRole,
        PermissionMode, Session, ToolExecutor,
    };
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use tools::GlobalToolRegistry;

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
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("rusty-claude-cli-{nanos}"))
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

    fn python_command() -> String {
        std::env::var("PYTHON").unwrap_or_else(|_| {
            if cfg!(windows) {
                "python".to_string()
            } else {
                "python3".to_string()
            }
        })
    }

    fn git(args: &[&str], cwd: &Path) {
        let status = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .expect("git command should run");
        assert!(status.success(), "git command failed: git {}", args.join(" "));
    }

    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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
        if include_hooks {
            fs::create_dir_all(root.join("hooks")).expect("hooks dir");
            fs::write(root.join("hooks").join("pre.sh"), "#!/bin/sh\nprintf 'plugin pre hook'\n")
                .expect("write hook");
        }
        if include_lifecycle {
            fs::create_dir_all(root.join("lifecycle")).expect("lifecycle dir");
            fs::write(
                root.join("lifecycle").join("init.sh"),
                "#!/bin/sh\nprintf 'init\\n' >> lifecycle.log\n",
            )
            .expect("write init lifecycle");
            fs::write(
                root.join("lifecycle").join("shutdown.sh"),
                "#!/bin/sh\nprintf 'shutdown\\n' >> lifecycle.log\n",
            )
            .expect("write shutdown lifecycle");
        }

        let hooks = if include_hooks {
            ",\n  \"hooks\": {\n    \"PreToolUse\": [\"./hooks/pre.sh\"]\n  }"
        } else {
            ""
        };
        let lifecycle = if include_lifecycle {
            ",\n  \"lifecycle\": {\n    \"Init\": [\"./lifecycle/init.sh\"],\n    \"Shutdown\": [\"./lifecycle/shutdown.sh\"]\n  }"
        } else {
            ""
        };
        fs::write(
            root.join(".claude-plugin").join("plugin.json"),
            format!(
                "{{\n  \"name\": \"{name}\",\n  \"version\": \"1.0.0\",\n  \"description\": \"runtime plugin fixture\"{hooks}{lifecycle}\n}}"
            ),
        )
        .expect("write plugin manifest");
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
                permission_mode: PermissionMode::DangerFullAccess,
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
        std::fs::write(
            cwd.join(".claw").join("settings.json"),
            r#"{"permissionMode":"acceptEdits"}"#,
        )
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
        std::fs::remove_dir_all(root).expect("temp config root should clean up");

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
        std::fs::write(
            cwd.join(".claw").join("settings.json"),
            r#"{"permissionMode":"acceptEdits"}"#,
        )
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
        std::fs::remove_dir_all(root).expect("temp config root should clean up");

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
                permission_mode: PermissionMode::DangerFullAccess,
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
                permission_mode: PermissionMode::DangerFullAccess,
            }
        );
    }

    #[test]
    fn resolves_model_aliases_in_args() {
        let _guard = env_lock();
        let env_root = temp_dir();
        let _env = EnvRestore::isolate(&env_root);
        let args = vec![
            "--model".to_string(),
            "opus".to_string(),
            "explain".to_string(),
            "this".to_string(),
        ];
        assert_eq!(
            parse_args(&args).expect("args should parse"),
            CliAction::Prompt {
                prompt: "explain this".to_string(),
                model: "claude-opus-4-7".to_string(),
                output_format: CliOutputFormat::Text,
                allowed_tools: None,
                permission_mode: PermissionMode::DangerFullAccess,
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
            CliAction::Update
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
    fn parses_global_cwd_before_workspace_action() {
        let root = temp_dir();
        let workspace = root.join("中文项目");
        fs::create_dir_all(&workspace).expect("workspace dir");

        let action = with_current_dir(&root, || {
            parse_args(&[
                "--cwd".to_string(),
                workspace.display().to_string(),
                "workspace".to_string(),
            ])
            .expect("args should parse")
        });

        fs::remove_dir_all(root).expect("temp workspace should clean up");
        assert_eq!(action, CliAction::Workspace { output_format: CliOutputFormat::Text });
    }

    #[test]
    fn parses_workspace_natural_language_intents() {
        assert_eq!(
            parse_workspace_natural_language_intent("当前工作区"),
            Some(WorkspaceIntent::Show)
        );
        assert_eq!(
            parse_workspace_natural_language_intent("切换到 D:\\Project"),
            Some(WorkspaceIntent::Switch("D:\\Project".to_string()))
        );
        assert_eq!(
            parse_workspace_natural_language_intent("打开项目 E:\\中文项目"),
            Some(WorkspaceIntent::Switch("E:\\中文项目".to_string()))
        );
        assert_eq!(parse_workspace_natural_language_intent("帮我 review 当前改动"), None);
    }

    #[test]
    fn compares_release_versions() {
        assert!(super::is_newer_version("v0.1.4", "0.1.3"));
        assert!(!super::is_newer_version("v0.1.3", "0.1.3"));
        assert!(!super::is_newer_version("v0.1.2", "0.1.3"));
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
                permission_mode: PermissionMode::DangerFullAccess,
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
        assert_eq!(
            parse_args(&["login".to_string()]).expect("login should parse"),
            CliAction::Login
        );
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
                permission_mode: PermissionMode::DangerFullAccess,
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
                permission_mode: PermissionMode::DangerFullAccess,
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
    fn review_history_commands_reject_invalid_shapes() {
        let missing_id = parse_args(&["/review".to_string(), "show".to_string()])
            .expect_err("/review show should require an id");
        assert!(missing_id.contains("missing review id"));

        let extra_list_arg =
            parse_args(&["/review".to_string(), "list".to_string(), "extra".to_string()])
                .expect_err("/review list should not accept extra args");
        assert!(extra_list_arg.contains("unexpected arguments"));

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
    fn keeps_bare_review_as_workflow_review() {
        assert_eq!(
            parse_args(&["review".to_string()]).expect("review should parse"),
            CliAction::Review { last_n: None, output_format: CliOutputFormat::Text }
        );
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
    fn parses_resume_flag_with_slash_command() {
        let args =
            vec!["--resume".to_string(), "session.jsonl".to_string(), "/compact".to_string()];
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
        let policy = permission_policy(
            PermissionMode::ReadOnly,
            &feature_config,
            &registry_with_plugin_tool(),
        )
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
        std::env::set_var("ANTHROPIC_API_KEY", "test-dummy-key-for-banner-test");
        std::env::set_var("ANTHROPIC_AUTH_TOKEN", "test-dummy-token-for-banner-test");
        let root = temp_dir();
        fs::create_dir_all(&root).expect("root dir");

        let banner = with_current_dir(&root, || {
            LiveCli::new(
                "claude-sonnet-4-6".to_string(),
                true,
                None,
                PermissionMode::DangerFullAccess,
            )
            .expect("cli should initialize")
            .startup_banner()
        });

        assert!(banner.contains("Tab"));
        assert!(banner.contains("workflow completions"));

        fs::remove_dir_all(root).expect("cleanup temp dir");
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
        assert!(status.contains("Cwd              /tmp/project"));
        assert!(status.contains("Project root     /tmp"));
        assert!(status.contains("Git branch       main"));
        assert!(
            status.contains("Git state        dirty · 3 files · 1 staged, 1 unstaged, 1 untracked")
        );
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
        assert!(report
            .contains("Action           create a git commit from the current workspace changes"));
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
        fs::remove_dir_all(temp_root).expect("cleanup temp dir");
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

        fs::remove_dir_all(root).expect("cleanup temp dir");
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
        fs::write(root.join("tracked.txt"), "hello\nstaged\nunstaged\n")
            .expect("update file twice");

        let report = render_diff_report_for(&root).expect("diff report should render");
        assert!(report.contains("Staged changes:"));
        assert!(report.contains("Unstaged changes:"));
        assert!(report.contains("tracked.txt"));

        fs::remove_dir_all(root).expect("cleanup temp dir");
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

        fs::remove_dir_all(root).expect("cleanup temp dir");
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

        fs::remove_dir_all(root).expect("cleanup temp dir");
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

        fs::remove_dir_all(root).expect("cleanup temp dir");
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

        fs::remove_dir_all(root).expect("cleanup temp dir");
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

        fs::remove_dir_all(root).expect("cleanup temp dir");
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

        fs::remove_dir_all(root).expect("cleanup temp dir");
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
            Ok(Some(SlashCommand::Resume {
                session_path: Some("saved-session.jsonl".to_string())
            }))
        );
        assert_eq!(
            SlashCommand::parse("/clear --confirm"),
            Ok(Some(SlashCommand::Clear { confirm: true }))
        );
        assert_eq!(
            SlashCommand::parse("/config"),
            Ok(Some(SlashCommand::Config { section: None }))
        );
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
        let content =
            (0..200).map(|index| format!("line {index:03}")).collect::<Vec<_>>().join("\n");
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
        let stdout =
            (0..120).map(|index| format!("stdout {index:03}")).collect::<Vec<_>>().join("\n");
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
        assert_eq!(
            pending_tool,
            Some(("tool-1".to_string(), "read_file".to_string(), String::new(),))
        );
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
        assert!(
            pre_hooks[0].ends_with("hooks/pre.sh"),
            "expected installed plugin hook path, got {pre_hooks:?}"
        );

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(source_root);
    }

    #[test]
    fn build_runtime_plugin_state_discovers_mcp_tools_and_surfaces_pending_servers() {
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
        fs::write(
            config_home.join("settings.json"),
            serde_json::to_string_pretty(&settings).unwrap(),
        )
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
        assert_eq!(
            search_json["mcp_degraded"]["failed_servers"][0]["phase"],
            "server_registration"
        );
        assert_eq!(
            search_json["mcp_degraded"]["failed_servers"][0]["error"]["context"]["transport"],
            "http"
        );

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn build_runtime_runs_plugin_lifecycle_init_and_shutdown() {
        let config_home = temp_dir();
        // Inject a dummy API key so runtime construction succeeds without real credentials.
        // This test only exercises plugin lifecycle (init/shutdown), never calls the API.
        std::env::set_var("DEEPSEEK_API_KEY", "test-dummy-key-for-plugin-lifecycle");
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

        assert_eq!(fs::read_to_string(&log_path).expect("init log should exist"), "init\n");

        runtime.shutdown_plugins().expect("plugin shutdown should succeed");

        assert_eq!(
            fs::read_to_string(&log_path).expect("shutdown log should exist"),
            "init\nshutdown\n"
        );

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(source_root);
        std::env::remove_var("DEEPSEEK_API_KEY");
    }
}

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
mod sandbox_report_tests {
    use super::{format_sandbox_report, HookAbortMonitor};
    use runtime::HookAbortSignal;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn sandbox_report_renders_expected_fields() {
        let report = format_sandbox_report(&runtime::SandboxStatus::default());
        assert!(report.contains("Sandbox"));
        assert!(report.contains("Enabled"));
        assert!(report.contains("Filesystem mode"));
        assert!(report.contains("Fallback reason"));
    }

    #[test]
    fn hook_abort_monitor_stops_without_aborting() {
        let abort_signal = HookAbortSignal::new();
        let (ready_tx, ready_rx) = mpsc::channel();
        let monitor = HookAbortMonitor::spawn_with_waiter(
            abort_signal.clone(),
            move |stop_rx, abort_signal| {
                ready_tx.send(()).expect("ready signal");
                let _ = stop_rx.recv();
                assert!(!abort_signal.is_aborted());
            },
        );

        ready_rx.recv().expect("waiter should be ready");
        monitor.stop();

        assert!(!abort_signal.is_aborted());
    }

    #[test]
    fn hook_abort_monitor_propagates_interrupt() {
        let abort_signal = HookAbortSignal::new();
        let (done_tx, done_rx) = mpsc::channel();
        let monitor = HookAbortMonitor::spawn_with_waiter(
            abort_signal.clone(),
            move |_stop_rx, abort_signal| {
                abort_signal.abort();
                done_tx.send(()).expect("done signal");
            },
        );

        done_rx.recv_timeout(Duration::from_secs(1)).expect("interrupt should complete");
        monitor.stop();

        assert!(abort_signal.is_aborted());
    }
}
