//! The argument-parsing subsystem: `CliAction`, the `parse_args` that produces
//! it, the argument normalisation and subcommand parsers it calls, and the
//! runtime state its variants carry.
//!
//! Moved out of `main.rs` (DEV-STRUCT-01, §1.19 step 2). The boundary came from
//! the measured dependency closure - 33 items, 1,134 lines - and not from a line
//! count: the first estimate was "291 + 110 lines" for two items. The closure is
//! large because `CliAction`'s variants embed the runtime state the CLI will
//! build (`RuntimeMcpState`, `RuntimePluginState`, the tool registry), so those
//! types and their builders travel with it.
//!
//! Named `cli_args` deliberately: a `src/args.rs` already exists in this crate
//! and is **not compiled** - `main.rs` never declares it as a module. Naming
//! this one `args` would link that dead file into the build.

// The three glob imports below are deliberate. This module calls into most of
// `commands`, `plugins` and `runtime`, and the code is a verbatim move: an
// explicit list would be a second, hand-maintained copy of what those calls
// already say, and it would have to be rewritten by every later slice that
// moves more callers in. `use crate::*` is the same trade for what the crate
// root has not been split yet - it is what keeps this move compiling.
#![allow(clippy::wildcard_imports)]

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use commands::*;
use plugins::*;
use runtime::*;

// The crate root still owns much of what these functions call: the collectors,
// the runtime builders and the remaining helpers.
use crate::*;

pub(crate) fn default_model() -> String {
    std::env::var("DEEPSEEK_MODEL")
        .or_else(|_| std::env::var("ANTHROPIC_MODEL"))
        .unwrap_or_else(|_| "deepseek-chat".to_string())
}

pub(crate) fn default_date() -> String {
    std::env::var("ANTHROPIC_DATE").unwrap_or_else(|_| {
        let now =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let secs = now.as_secs();
        // Simple date calculation from Unix epoch
        let days_since_epoch = secs / 86400;
        // 1970-01-01 was a Thursday. Calculate year/month/day.
        // A clock so far out that the day count overflows i64 is not a date this
        // function can render; saturating beats wrapping into a year before 1970.
        let mut days = i64::try_from(days_since_epoch).unwrap_or(i64::MAX);
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
        let mut month = 0usize;
        while month < 12 && days >= month_days[month] {
            days -= month_days[month];
            month += 1;
        }
        format!("{:04}-{:02}-{:02}", year, month + 1, days + 1)
    })
}

pub(crate) fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CliAction {
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
    Dir,
    Update {
        check_only: bool,
    },
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
    CodeReviewShowJson {
        id: String,
    },
    CodeReviewCard {
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
    /// C20.6-C R6: print a pre-rendered message (e.g. task-parser blocked
    /// guidance) and exit cleanly, without the `error:` prefix or the
    /// `sego --help` footer that an `Err(String)` return would attach.
    PrintAndExit {
        message: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SafetyReviewScope {
    Workspace,
    Staged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CliOutputFormat {
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
pub(crate) fn parse_args(args: &[String]) -> Result<CliAction, String> {
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
                model.clone_from(&resolve_model_alias(value));
                index += 2;
            }
            flag if flag.starts_with("--model=") => {
                model.clone_from(&resolve_model_alias(&flag[8..]));
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
                    model: resolve_model_alias(&model).clone(),
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
    if rest.first().map(String::as_str) == Some("review") {
        if rest.get(1).is_some_and(|value| value.starts_with("--last")) {
            return Err(
                "`sego review --last` now belongs to workflow review. Use `sego workflow-review --last N` for session analysis, or `sego review [staged|workspace|<path>]` for code review."
                    .to_string(),
            );
        }
        let scope = join_optional_args(&rest[1..]);
        return parse_code_review_slash_action(scope.as_deref(), model, allowed_tools);
    }
    if rest
        .first()
        .is_some_and(|command| matches!(command.as_str(), "workflow-review" | "session-review"))
    {
        return parse_workflow_review_args(&rest[1..], output_format);
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
        "update" => {
            if rest.len() == 2 && matches!(rest[1].as_str(), "--check" | "check") {
                Ok(CliAction::Update { check_only: true })
            } else if rest.len() == 1 {
                Ok(CliAction::Update { check_only: false })
            } else {
                Err("update accepts no arguments except --check".to_string())
            }
        }
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
            // C20.6-C R4: route combined /cd ... && /review through task_parser
            // so the user gets task-parser blocked guidance instead of the
            // legacy `/cd is REPL-only` error.
            let raw = rest.join(" ");
            let is_review_history = rest.first().map(String::as_str) == Some("/review")
                && rest.get(1).is_some_and(|sub| {
                    matches!(
                        sub.as_str(),
                        "list"
                            | "show"
                            | "card"
                            | "status"
                            | "mark"
                            | "ready"
                            | "summary"
                            | "tools"
                            | "safety"
                    )
                });
            if !is_review_history {
                match task_parser::parse_required_review_command(&raw) {
                    task_parser::RequiredReviewResult::Execute { scope } => {
                        return Ok(CliAction::CodeReview {
                            scope: Some(scope),
                            model,
                            allowed_tools,
                            permission_mode: PermissionMode::ReadOnly,
                        });
                    }
                    task_parser::RequiredReviewResult::Blocked { detected, reason, guidance } => {
                        // C20.6-C R6: return a clean PrintAndExit (no `error:` prefix,
                        // no `Run `sego --help`` footer) so users see the task-parser
                        // guidance directly.
                        return Ok(CliAction::PrintAndExit {
                            message: format!(
                                "Task command blocked\n  Detected         {detected}\n  Reason           {reason}\n  Guidance         {guidance}"
                            ),
                        });
                    }
                    task_parser::RequiredReviewResult::None => {}
                }
            }
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

pub(crate) fn parse_single_word_command_alias(
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
        "update" => Some(Ok(CliAction::Update { check_only: false })),
        "status" => Some(Ok(CliAction::Status {
            model: model.to_string(),
            permission_mode: permission_mode_override.unwrap_or_else(default_permission_mode),
            output_format,
        })),
        "sandbox" => Some(Ok(CliAction::Sandbox { output_format })),
        "workspace" | "workdir" | "pwd" | "cwd" => Some(Ok(CliAction::Workspace { output_format })),
        "learn" => Some(Ok(CliAction::Learn { output_format: CliOutputFormat::Text })),
        "doctor" => Some(Ok(CliAction::Doctor { output_format: CliOutputFormat::Text })),
        "telemetry" => {
            let action = rest.get(1).cloned();
            Some(Ok(CliAction::Telemetry { action, output_format: CliOutputFormat::Text }))
        }
        other => bare_slash_command_guidance(other).map(Err),
    }
}

pub(crate) fn parse_workflow_review_args(
    args: &[String],
    output_format: CliOutputFormat,
) -> Result<CliAction, String> {
    let mut last_n = None;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--last" | "--last-n" => {
                let value =
                    args.get(index + 1).ok_or_else(|| "missing value for --last".to_string())?;
                last_n = Some(parse_positive_usize(value, "--last")?);
                index += 2;
            }
            flag if flag.starts_with("--last=") => {
                last_n = Some(parse_positive_usize(&flag[7..], "--last")?);
                index += 1;
            }
            flag if flag.starts_with("--last-n=") => {
                last_n = Some(parse_positive_usize(&flag[9..], "--last-n")?);
                index += 1;
            }
            other => {
                return Err(format!(
                    "unexpected argument for workflow review: {other}. Use `sego workflow-review [--last N]`."
                ));
            }
        }
    }

    Ok(CliAction::Review { last_n, output_format })
}

pub(crate) fn parse_positive_usize(value: &str, flag_name: &str) -> Result<usize, String> {
    let parsed =
        value.parse::<usize>().map_err(|_| format!("{flag_name} expects a positive integer"))?;
    if parsed == 0 {
        return Err(format!("{flag_name} expects a positive integer"));
    }
    Ok(parsed)
}

pub(crate) fn join_optional_args(args: &[String]) -> Option<String> {
    let joined = args.join(" ");
    let trimmed = joined.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub(crate) fn parse_direct_slash_cli_action(
    rest: &[String],
    model: String,
    allowed_tools: Option<AllowedToolSet>,
    permission_mode: PermissionMode,
) -> Result<CliAction, String> {
    let raw = rest.join(" ");
    match SlashCommand::parse(&raw) {
        Ok(Some(SlashCommand::Help)) => Ok(CliAction::Help),
        Ok(Some(SlashCommand::Dir)) => Ok(CliAction::Dir),
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

pub(crate) fn parse_code_review_slash_action(
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

pub(crate) fn resolve_cli_cwd(value: &str) -> Result<PathBuf, String> {
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

pub(crate) fn apply_requested_cwd(cwd: Option<&PathBuf>) -> Result<(), String> {
    if let Some(cwd) = cwd {
        env::set_current_dir(cwd)
            .map_err(|error| format!("failed to switch --cwd to {}: {error}", cwd.display()))?;
    }
    Ok(())
}

pub(crate) fn resolve_model_alias(model: &str) -> String {
    api::resolve_model_alias(model)
}

pub(crate) fn normalize_allowed_tools(values: &[String]) -> Result<Option<AllowedToolSet>, String> {
    if values.is_empty() {
        return Ok(None);
    }
    current_tool_registry()?.normalize_allowed_tools(values)
}

pub(crate) fn current_tool_registry() -> Result<GlobalToolRegistry, String> {
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

pub(crate) fn parse_permission_mode_arg(value: &str) -> Result<PermissionMode, String> {
    normalize_permission_mode(value)
        .ok_or_else(|| {
            format!(
                "unsupported permission mode '{value}'. Use read-only, workspace-write, or danger-full-access."
            )
        })
        .map(permission_mode_from_label)
}

pub(crate) fn parse_system_prompt_args(args: &[String]) -> Result<CliAction, String> {
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

pub(crate) fn parse_resume_args(args: &[String]) -> Result<CliAction, String> {
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

pub(crate) fn resume_command_can_absorb_token(current_command: &str, token: &str) -> bool {
    matches!(SlashCommand::parse(current_command), Ok(Some(SlashCommand::Export { path: None })))
        && !looks_like_slash_command_token(token)
}

pub(crate) struct RuntimeMcpState {
    runtime: tokio::runtime::Runtime,
    manager: McpServerManager,
    pending_servers: Vec<String>,
    degraded_report: Option<runtime::McpDegradedReport>,
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

    pub(crate) fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        self.runtime.block_on(self.manager.shutdown())?;
        Ok(())
    }

    pub(crate) fn pending_servers(&self) -> Option<Vec<String>> {
        (!self.pending_servers.is_empty()).then(|| self.pending_servers.clone())
    }

    pub(crate) fn degraded_report(&self) -> Option<runtime::McpDegradedReport> {
        self.degraded_report.clone()
    }

    fn server_names(&self) -> Vec<String> {
        self.manager.server_names()
    }

    pub(crate) fn call_tool(
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

    pub(crate) fn list_resources_for_server(
        &mut self,
        server_name: &str,
    ) -> Result<String, ToolError> {
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

    pub(crate) fn list_resources_for_all_servers(&mut self) -> Result<String, ToolError> {
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

    pub(crate) fn read_resource(
        &mut self,
        server_name: &str,
        uri: &str,
    ) -> Result<String, ToolError> {
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

/// The MCP state a runtime builds, and the tool definitions discovered with it.
/// `None` means no MCP server was configured.
pub(crate) type RuntimeMcpBuild = Result<
    (Option<Arc<Mutex<RuntimeMcpState>>>, Vec<RuntimeToolDefinition>),
    Box<dyn std::error::Error>,
>;

pub(crate) fn build_runtime_mcp_state(runtime_config: &runtime::RuntimeConfig) -> RuntimeMcpBuild {
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

pub(crate) fn mcp_wrapper_tool_definitions() -> Vec<RuntimeToolDefinition> {
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

pub(crate) fn normalize_permission_mode(mode: &str) -> Option<&'static str> {
    match mode.trim() {
        "read-only" => Some("read-only"),
        "workspace-write" => Some("workspace-write"),
        "danger-full-access" => Some("danger-full-access"),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReviewHistoryCommand {
    List,
    Show { id: String, json: bool },
    Card { id: String },
    Status { id: String },
    Mark { id: String, finding_id: String, status: ReviewFindingStatus, note: Option<String> },
    Ready,
    Summary,
    Tools,
    Safety { scope: SafetyReviewScope },
}

pub(crate) fn parse_review_history_command(
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
        ["show", "--json"] => {
            Err("missing review id or latest for /review show <id|latest> --json".into())
        }
        ["show", id] => Ok(Some(ReviewHistoryCommand::Show { id: (*id).to_string(), json: false })),
        ["show", id, "--json"] => {
            Ok(Some(ReviewHistoryCommand::Show { id: (*id).to_string(), json: true }))
        }
        ["show", ..] => Err("unexpected arguments for /review show <id|latest> [--json]".into()),
        ["card"] => Ok(Some(ReviewHistoryCommand::Card { id: "latest".to_string() })),
        ["card", id] => Ok(Some(ReviewHistoryCommand::Card { id: (*id).to_string() })),
        ["card", ..] => Err("unexpected arguments for /review card [latest|<review-id>]".into()),
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

pub(crate) fn review_history_command_to_cli_action(command: ReviewHistoryCommand) -> CliAction {
    match command {
        ReviewHistoryCommand::List => CliAction::CodeReviewList,
        ReviewHistoryCommand::Show { id, json } => {
            if json {
                CliAction::CodeReviewShowJson { id }
            } else {
                CliAction::CodeReviewShow { id }
            }
        }
        ReviewHistoryCommand::Card { id } => CliAction::CodeReviewCard { id },
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

pub(crate) fn build_runtime_plugin_state_with_loader(
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

pub(crate) fn build_plugin_manager(
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

pub(crate) fn resolve_plugin_path(cwd: &Path, config_home: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else if value.starts_with('.') {
        cwd.join(path)
    } else {
        config_home.join(path)
    }
}

pub(crate) fn runtime_hook_config_from_plugin_hooks(
    hooks: PluginHooks,
) -> runtime::RuntimeHookConfig {
    runtime::RuntimeHookConfig::new(
        hooks.pre_tool_use,
        hooks.post_tool_use,
        hooks.post_tool_use_failure,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_string()).collect()
    }

    #[test]
    fn an_unknown_option_is_reported_with_the_suggestion_machinery() {
        // The error text comes from `cli_suggestions`, so this also checks the
        // two modules still agree on the shape of that message.
        let error = parse_args(&args(&["--modle", "x"])).expect_err("--modle is not an option");
        assert!(error.contains("unknown option: --modle"), "{error}");
        assert!(error.contains("Did you mean --model?"), "{error}");
    }

    #[test]
    fn a_far_off_option_gets_no_suggestion() {
        let error = parse_args(&args(&["--zzzzzzzzzz"])).expect_err("not an option");
        assert!(error.contains("unknown option"), "{error}");
        assert!(!error.contains("Did you mean"), "{error}");
    }

    #[test]
    fn version_flags_are_recognised_in_both_forms() {
        assert_eq!(parse_args(&args(&["--version"])).expect("long form"), CliAction::Version);
        assert_eq!(parse_args(&args(&["-V"])).expect("short form"), CliAction::Version);
    }

    #[test]
    fn the_help_flags_produce_a_request_rather_than_an_error() {
        // `--help` must never be an "unknown option"; the action it produces is
        // the CLI's decision, not this module's, so only success is asserted.
        for flag in ["--help", "-h"] {
            assert!(parse_args(&args(&[flag])).is_ok(), "{flag} must parse");
        }
    }

    #[test]
    fn no_input_parses_without_a_panic() {
        // Bare invocation, an empty argument, and a lone dash: all are reachable
        // from a shell, and none may panic in the parser.
        for case in [vec![], vec![""], vec!["-"], vec!["--"], vec![" "]] {
            let case = case.iter().map(|v| (*v).to_string()).collect::<Vec<_>>();
            let _ = parse_args(&case);
        }
    }

    #[test]
    fn every_subcommand_word_parses_or_errors_but_never_panics() {
        // A table over the command surface: each entry must come back as a
        // result, and an error must be a message rather than an empty string.
        let words = [
            "agents",
            "skills",
            "mcp",
            "code-review",
            "sandbox",
            "status",
            "workspace",
            "session",
            "review",
            "doctor",
            "update",
            "config",
            "cron",
            "tasks",
            "plugin",
            "plugins",
            "diff",
            "export",
            "resume",
            "system-prompt",
        ];
        for word in words {
            match parse_args(&args(&[word])) {
                Ok(_) => {}
                Err(message) => {
                    assert!(!message.trim().is_empty(), "{word} produced an empty error");
                }
            }
        }
    }

    #[test]
    fn option_values_are_consumed_rather_than_treated_as_subcommands() {
        // `--model` takes the next argument; if it did not, the value would be
        // parsed as a subcommand and this would fail differently.
        let parsed = parse_args(&args(&["--model", "some-model", "--version"]));
        assert_eq!(parsed.expect("a flag sequence must parse"), CliAction::Version);
    }
}
