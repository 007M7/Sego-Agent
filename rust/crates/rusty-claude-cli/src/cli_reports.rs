//! The terminal reports: the text a user reads after asking for state.
//!
//! Moved out of `main.rs` (DEV-STRUCT-01 step 3, §1.23 class E - extract the
//! formatting a terminal command prints, so the assertion is on the text rather
//! than on the terminal). Bodies are verbatim.
//!
//! The boundary came from the measured dependency closure: **25 items, 485
//! lines**, and every one of them is pure - given values, return a `String`.
//! Measuring it exposed a third extractor defect, fixed before the move: the
//! closure's reference detector matched calls (`NAME(`), paths (`NAME::`), type
//! positions (`: NAME`) and return types (`-> NAME`), so a constant used only as
//! an inline format argument (`{NAME}`) was invisible. That is a bare
//! identifier reference, and two items were missing from the count because of
//! it.
//!
//! Two of the items the corrected closure named stay at the crate root on
//! purpose: `PRIMARY_SESSION_EXTENSION` and `LATEST_SESSION_REFERENCE` are
//! session-layout facts, not report facts, and the session-resolution code in
//! `main.rs` is where they belong. They reach this module through `use crate::*`.
//!
//! The collectors that build the view types below live in `cli_context`, which
//! is the I/O half of this split: it runs `git`, reads config and looks at the
//! filesystem, while everything here is pure.
//!
//! The `print_*` methods that write to the terminal stay in `main.rs` and are
//! one line each; that split is the whole point of the class.

use std::path::{Path, PathBuf};

use api::*;
use runtime::*;

use crate::{
    detect_provider_kind, render_slash_command_help, TokenUsage, LATEST_SESSION_REFERENCE,
    PRIMARY_SESSION_EXTENSION, UNIX_EPOCH,
};

// The four view types below are `pub(crate)` down to their fields because they
// are built in `cli_context` and read here: a struct literal needs every field
// visible from where it is written, so a boundary that puts the collector and the
// formatter in different modules has to widen the fields. That is the honest cost
// of the split, not an oversight - and it is why moving the collectors did not let
// the fields be narrowed again.
#[derive(Debug, Clone)]
pub(crate) struct StatusContext {
    pub(crate) cwd: PathBuf,
    pub(crate) session_path: Option<PathBuf>,
    pub(crate) loaded_config_files: usize,
    pub(crate) discovered_config_files: usize,
    pub(crate) memory_file_count: usize,
    pub(crate) project_root: Option<PathBuf>,
    pub(crate) git_branch: Option<String>,
    pub(crate) git_summary: GitWorkspaceSummary,
    pub(crate) sandbox_status: runtime::SandboxStatus,
}

pub(crate) fn provider_kind_label(kind: api::ProviderKind) -> &'static str {
    match kind {
        api::ProviderKind::Anthropic => "anthropic",
        api::ProviderKind::Xai => "xai",
        api::ProviderKind::OpenAi => "openai",
        api::ProviderKind::DeepSeek => "deepseek",
    }
}

pub(crate) struct WorkspaceContext {
    pub(crate) cwd: PathBuf,
    pub(crate) project_root: Option<PathBuf>,
    pub(crate) session_dir: PathBuf,
    pub(crate) recovery_dir: PathBuf,
    pub(crate) sandbox_status: runtime::SandboxStatus,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct StatusUsage {
    pub(crate) message_count: usize,
    pub(crate) turns: u32,
    pub(crate) latest: TokenUsage,
    pub(crate) cumulative: TokenUsage,
    pub(crate) estimated_tokens: usize,
}

#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct GitWorkspaceSummary {
    pub(crate) changed_files: usize,
    pub(crate) staged_files: usize,
    pub(crate) unstaged_files: usize,
    pub(crate) untracked_files: usize,
    pub(crate) conflicted_files: usize,
}

impl GitWorkspaceSummary {
    pub(crate) fn is_clean(self) -> bool {
        self.changed_files == 0
    }

    pub(crate) fn headline(self) -> String {
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

pub(crate) fn format_model_report(model: &str, message_count: usize, turns: u32) -> String {
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

pub(crate) fn format_model_switch_report(
    previous: &str,
    next: &str,
    message_count: usize,
) -> String {
    format!(
        "Model updated
  Previous         {previous}
  Current          {next}
  Preserved msgs   {message_count}"
    )
}

pub(crate) fn format_permissions_report(mode: &str) -> String {
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

pub(crate) fn format_permissions_switch_report(previous: &str, next: &str) -> String {
    format!(
        "Permissions updated
  Result           mode switched
  Previous mode    {previous}
  Active mode      {next}
  Applies to       subsequent tool calls
  Usage            /permissions to inspect current mode"
    )
}

pub(crate) fn format_cost_report(usage: TokenUsage) -> String {
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

pub(crate) fn format_resume_report(session_path: &str, message_count: usize, turns: u32) -> String {
    format!(
        "Session resumed
  Session file     {session_path}
  Messages         {message_count}
  Turns            {turns}"
    )
}

pub(crate) fn render_resume_usage() -> String {
    format!(
        "Resume
  Usage            /resume <session-path|session-id|{LATEST_SESSION_REFERENCE}>
  Auto-save        .claw/sessions/<session-id>.{PRIMARY_SESSION_EXTENSION}
  Tip              use /session list to inspect saved sessions"
    )
}

pub(crate) fn format_compact_report(
    removed: usize,
    resulting_messages: usize,
    skipped: bool,
) -> String {
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

pub(crate) fn format_auto_compaction_notice(removed: usize) -> String {
    format!("[auto-compacted: removed {removed} messages]")
}

pub(crate) fn format_missing_session_reference(reference: &str) -> String {
    format!(
        "session not found: {reference}\nHint: managed sessions live in .claw/sessions/. Try `{LATEST_SESSION_REFERENCE}` for the most recent session or `/session list` in the REPL."
    )
}

pub(crate) fn format_no_managed_sessions() -> String {
    format!(
        "no managed sessions found in .claw/sessions/\nStart `claw` to create a session, then rerun with `--resume {LATEST_SESSION_REFERENCE}`."
    )
}

pub(crate) fn format_session_modified_age(modified_epoch_millis: u128) -> String {
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

pub(crate) fn render_repl_help() -> String {
    [
        "REPL".to_string(),
        "  /exit                Quit the REPL".to_string(),
        "  /quit                Quit the REPL".to_string(),
        "  /dir                 Show common commands and natural-language examples".to_string(),
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

pub(crate) fn render_natural_language_directory() -> String {
    [
        "Sego 常用动作目录 (Action Directory)".to_string(),
        "  Usage: /dir  | say one of the examples below to trigger the local action".to_string(),
        String::new(),
        "  工作区 (Workspace)".to_string(),
        "    /workspace                      当前工作区 / show workspace".to_string(),
        "    /cd D:\\YourProject             切换到 D:\\YourProject / switch to D:\\YourProject"
            .to_string(),
        "      例: 切换到 D:\\Project  打开项目 E:\\code".to_string(),
        String::new(),
        "  审查 (Review)".to_string(),
        "    /review                         帮我 review 当前改动 / review current changes"
            .to_string(),
        "    /review staged                  review staged changes / 审查已暂存改动".to_string(),
        "    /review workspace               审查整个项目代码 / audit full workspace".to_string(),
        "    /review card [latest|<id>]      生成并打开 Sego 验收卡".to_string(),
        "    /review safety staged           检查已暂存代码的安全风险".to_string(),
        "    /review --full E:\\repo           审查整个仓库(无需git diff) / full repo audit"
            .to_string(),
        "      例: 审查当前改动  review staged  帮我检查代码的潜在风险".to_string(),
        String::new(),
        "  导出 (Save/Export)".to_string(),
        "    /export                         导出当前会话 / export conversation".to_string(),
        "    /export E:\\code\\session.md      导出当前会话到 E:\\code\\session.md".to_string(),
        "    把刚才的审查结果写成 E:\\code\\review.md   (must say 刚才/上一条/last/previous)"
            .to_string(),
        "    export the last review to report.md  (must say last/previous)".to_string(),
        "      例: save the last review to PR43.md  把刚才的回复保存到 E:\\out.md".to_string(),
        String::new(),
        "  更新 (Update)".to_string(),
        "    sego update --check             检查更新 / check for update".to_string(),
        "    sego update                     更新到最新版 / update sego".to_string(),
        "      例: 检查更新  帮我更新".to_string(),
        String::new(),
        "  退出 (Exit)".to_string(),
        "    /exit                           退出 Sego / exit".to_string(),
        String::new(),
        "  安全注意 (Safety)".to_string(),
        "    ExportLastResponse 必须包含 刚才/上一条/last/previous，避免导出错误内容。".to_string(),
        "    如果只说 保存报告/导出md 而不指定对象，Sego 会提示 /dir。".to_string(),
        "    Review 需要 Git 仓库 (或使用 --full 审计任意目录)。".to_string(),
        String::new(),
        "  说明".to_string(),
        "    未列出的普通编码、解释、讨论请求会继续交给模型处理。".to_string(),
    ]
    .join("\n")
}

pub(crate) fn format_status_report(
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
            "Provider/cache
  Provider         {}
  Latest cache     create {}, read {}
  Cumulative cache create {}, read {}",
            provider_kind_label(detect_provider_kind(model)),
            usage.latest.cache_creation_input_tokens,
            usage.latest.cache_read_input_tokens,
            usage.cumulative.cache_creation_input_tokens,
            usage.cumulative.cache_read_input_tokens,
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

pub(crate) fn format_workspace_report(context: &WorkspaceContext) -> String {
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

pub(crate) fn format_workspace_switch_report(
    previous: &Path,
    next: &Path,
    switched: bool,
) -> String {
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

pub(crate) fn format_sandbox_report(status: &runtime::SandboxStatus) -> String {
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

pub(crate) fn format_commit_preflight_report(
    branch: Option<&str>,
    summary: GitWorkspaceSummary,
) -> String {
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

pub(crate) fn format_commit_skipped_report() -> String {
    "Commit
  Result           skipped
  Reason           no workspace changes
  Action           create a git commit from the current workspace changes
  Next             /status to inspect context · /diff to inspect repo changes"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(input: u32, output: u32, cache_create: u32, cache_read: u32) -> TokenUsage {
        TokenUsage {
            input_tokens: input,
            output_tokens: output,
            cache_creation_input_tokens: cache_create,
            cache_read_input_tokens: cache_read,
        }
    }

    fn status_usage() -> StatusUsage {
        StatusUsage {
            message_count: 4,
            turns: 2,
            latest: usage(10, 20, 30, 40),
            cumulative: usage(100, 200, 300, 400),
            estimated_tokens: 1_234,
        }
    }

    fn status_context() -> StatusContext {
        StatusContext {
            cwd: PathBuf::from("/workspace"),
            session_path: None,
            loaded_config_files: 2,
            discovered_config_files: 3,
            memory_file_count: 1,
            project_root: None,
            git_branch: Some("main".to_string()),
            git_summary: GitWorkspaceSummary::default(),
            sandbox_status: runtime::SandboxStatus::default(),
        }
    }

    /// The first line of `report` whose trimmed text starts with `label`.
    fn line_of(report: &str, label: &str) -> String {
        report
            .lines()
            .find(|line| line.trim_start().starts_with(label))
            .unwrap_or_else(|| panic!("no line starts with `{label}` in:\n{report}"))
            .to_string()
    }

    #[test]
    fn permissions_report_marks_only_the_mode_in_force() {
        let report = format_permissions_report("read-only");
        let active = line_of(&report, "read-only");
        assert!(active.contains("● current"), "the active mode must be the marked one: {active}");
        for other in ["workspace-write", "danger-full-access"] {
            let line = line_of(&report, other);
            assert!(line.contains("○ available"), "{other} is not in force: {line}");
            assert!(!line.contains("● current"), "{other} must not claim to be current: {line}");
        }
        assert_eq!(
            report.matches("● current").count(),
            1,
            "exactly one mode is current:\n{report}"
        );
    }

    #[test]
    fn model_report_marks_the_current_model_exactly_once() {
        let report = format_model_report("deepseek-chat", 7, 3);
        assert!(report.contains("Current model    deepseek-chat"), "{report}");
        assert!(report.contains("Session messages 7"), "{report}");
        assert!(report.contains("Session turns    3"), "{report}");
        let current = line_of(&report, "deepseek-chat");
        assert!(current.contains("● current"), "{current}");
        assert_eq!(
            report.matches("● current").count(),
            1,
            "one model is current, not several:\n{report}"
        );
    }

    #[test]
    fn model_switch_report_names_both_ends_and_keeps_the_message_count() {
        let report = format_model_switch_report("claude-sonnet-4-6", "deepseek-flash", 12);
        assert!(report.contains("Previous         claude-sonnet-4-6"), "{report}");
        assert!(report.contains("Current          deepseek-flash"), "{report}");
        assert!(report.contains("Preserved msgs   12"), "{report}");
    }

    #[test]
    fn cost_report_total_is_the_sum_of_its_four_parts() {
        // Distinct values, so summing the wrong field or dropping one produces a
        // different total rather than coincidentally the same one.
        let report = format_cost_report(usage(10, 20, 30, 40));
        assert!(report.contains("Input tokens     10"), "{report}");
        assert!(report.contains("Output tokens    20"), "{report}");
        assert!(report.contains("Cache create     30"), "{report}");
        assert!(report.contains("Cache read       40"), "{report}");
        assert!(report.contains("Total tokens     100"), "{report}");
    }

    #[test]
    fn compact_report_never_reads_as_removal_when_it_skipped() {
        // "removed 0 messages" would read as a compaction that ran and achieved
        // nothing, which is a different fact from "below the threshold, so it did
        // not run at all" - the same distinction as "zero findings is not a pass".
        let skipped = format_compact_report(0, 9, true);
        assert!(skipped.contains("Result           skipped"), "{skipped}");
        assert!(skipped.contains("below compaction threshold"), "{skipped}");
        assert!(
            !skipped.contains("Messages removed"),
            "a skipped compaction must not carry a removal count:\n{skipped}"
        );

        let compacted = format_compact_report(5, 9, false);
        assert!(compacted.contains("Result           compacted"), "{compacted}");
        assert!(compacted.contains("Messages removed 5"), "{compacted}");
        assert!(!compacted.contains("skipped"), "a real compaction is not skipped:\n{compacted}");
    }

    #[test]
    fn workspace_switch_report_distinguishes_switched_from_unchanged() {
        // This text is the only thing the user sees that explains which branch
        // `switch_workspace` took, so it has to agree with the bool it was given.
        let previous = Path::new("/old");
        let next = Path::new("/new");

        let unchanged = format_workspace_switch_report(previous, next, false);
        assert!(unchanged.contains("Workspace unchanged"), "{unchanged}");
        assert!(unchanged.contains("requested path is already active"), "{unchanged}");
        assert!(
            !unchanged.contains("Workspace switched"),
            "re-selecting the active directory must not read as a switch:\n{unchanged}"
        );

        let switched = format_workspace_switch_report(previous, next, true);
        assert!(switched.contains("Workspace switched"), "{switched}");
        assert!(switched.contains("Previous cwd     /old"), "{switched}");
        assert!(switched.contains("Active cwd       /new"), "{switched}");
    }

    #[test]
    fn git_headline_lists_only_the_categories_that_have_files() {
        let clean = GitWorkspaceSummary::default();
        assert!(clean.is_clean());
        assert_eq!(clean.headline(), "clean");

        let dirty = GitWorkspaceSummary {
            changed_files: 2,
            staged_files: 1,
            unstaged_files: 1,
            untracked_files: 0,
            conflicted_files: 0,
        };
        assert!(!dirty.is_clean());
        let headline = dirty.headline();
        assert!(headline.contains("2 files"), "{headline}");
        assert!(headline.contains("1 staged"), "{headline}");
        assert!(headline.contains("1 unstaged"), "{headline}");
        assert!(
            !headline.contains("untracked") && !headline.contains("conflicted"),
            "a category with no files must not be listed, because `0 untracked` reads \
             as a problem rather than as an absence:\n{headline}"
        );
    }

    #[test]
    fn session_age_picks_the_unit_that_matches_the_magnitude() {
        let now = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_millis();
        let ago = |millis: u128| format_session_modified_age(now.saturating_sub(millis));

        // Mid-range values, so a boundary adjustment cannot make this flaky.
        let just_now = ago(2_000);
        assert!(just_now.contains("just-now"), "{just_now}");
        let seconds = ago(30_000);
        assert!(seconds.ends_with("s-ago"), "{seconds}");
        assert!(
            !seconds.starts_with("0m"),
            "thirty seconds must not be reported as zero minutes: {seconds}"
        );
        let minutes = ago(90_000);
        assert!(minutes.ends_with("m-ago"), "{minutes}");
        let hours = ago(7_200_000);
        assert!(hours.ends_with("h-ago"), "{hours}");
        let days = ago(172_800_000);
        assert!(days.ends_with("d-ago"), "{days}");
    }

    #[test]
    fn sandbox_report_says_none_rather_than_leaving_a_value_empty() {
        let empty = format_sandbox_report(&runtime::SandboxStatus::default());
        assert_eq!(
            empty.matches("<none>").count(),
            3,
            "mounts, markers and the fallback reason each need an explicit value:\n{empty}"
        );
        assert!(empty.contains("Filesystem mode"), "{empty}");

        let with_mounts = runtime::SandboxStatus {
            allowed_mounts: vec!["/a".to_string(), "/b".to_string()],
            ..runtime::SandboxStatus::default()
        };
        let listed = format_sandbox_report(&with_mounts);
        assert!(listed.contains("/a, /b"), "mounts are joined, not dropped:\n{listed}");
    }

    #[test]
    fn status_report_takes_the_provider_from_the_model() {
        // The provider line must follow the model argument: a hard-coded value
        // would look right for one model and lie for every other.
        let deepseek =
            format_status_report("deepseek-chat", status_usage(), "read-only", &status_context());
        assert!(deepseek.contains("Provider         deepseek"), "{deepseek}");
        assert!(deepseek.contains("Model            deepseek-chat"), "{deepseek}");
        assert!(deepseek.contains("Permission mode  read-only"), "{deepseek}");

        let anthropic = format_status_report(
            "claude-sonnet-4-6",
            status_usage(),
            "read-only",
            &status_context(),
        );
        assert!(anthropic.contains("Provider         anthropic"), "{anthropic}");
    }

    #[test]
    fn status_report_labels_a_live_session_and_an_unknown_project_root() {
        // "live-repl" is the label for "this session has no file yet"; a blank
        // value there would read as a session that has no path at all.
        let report =
            format_status_report("deepseek-chat", status_usage(), "read-only", &status_context());
        assert!(report.contains("Session          live-repl"), "{report}");
        assert!(report.contains("Project root     unknown"), "{report}");

        let mut with_paths = status_context();
        with_paths.session_path = Some(PathBuf::from("/sessions/s.jsonl"));
        with_paths.project_root = Some(PathBuf::from("/project"));
        let named = format_status_report("deepseek-chat", status_usage(), "read-only", &with_paths);
        assert!(named.contains("Session          /sessions/s.jsonl"), "{named}");
        assert!(named.contains("Project root     /project"), "{named}");
    }

    #[test]
    fn commit_skipped_report_does_not_read_as_ready() {
        let skipped = format_commit_skipped_report();
        assert!(skipped.contains("Result           skipped"), "{skipped}");
        assert!(skipped.contains("no workspace changes"), "{skipped}");
        assert!(
            !skipped.contains("ready"),
            "a skipped commit must not be mistakable for a prepared one:\n{skipped}"
        );
    }

    #[test]
    fn provider_kind_label_names_every_provider_the_cli_can_route_to() {
        let table = [
            (api::ProviderKind::Anthropic, "anthropic"),
            (api::ProviderKind::Xai, "xai"),
            (api::ProviderKind::OpenAi, "openai"),
            (api::ProviderKind::DeepSeek, "deepseek"),
        ];
        for (kind, expected) in table {
            assert_eq!(provider_kind_label(kind), expected, "{kind:?}");
        }
    }

    #[test]
    fn repl_help_covers_quitting_resuming_and_the_directory() {
        let help = render_repl_help();
        for needle in ["/exit", "/quit", "/dir", "/resume latest", "/session list"] {
            assert!(help.contains(needle), "the REPL help must mention {needle}:\n{help}");
        }
        assert!(
            help.contains("Clear input (or exit on empty prompt)"),
            "Ctrl-C has two behaviours and the help must state both:\n{help}"
        );
        assert!(
            help.contains("Slash commands"),
            "the REPL help must carry the slash-command index, not only the REPL block:\n{help}"
        );
    }

    #[test]
    fn action_directory_names_both_languages_of_the_workspace_phrases() {
        let directory = render_natural_language_directory();
        assert!(directory.contains("Action Directory"), "{directory}");
        assert!(directory.contains("当前工作区"), "{directory}");
        assert!(directory.contains("show workspace"), "{directory}");
        assert!(directory.contains("/review"), "{directory}");
    }

    #[test]
    fn resume_usage_names_the_auto_save_pattern_and_the_latest_alias() {
        let usage = render_resume_usage();
        assert!(usage.contains("/resume <session-path|session-id|latest>"), "{usage}");
        assert!(usage.contains(".claw/sessions/<session-id>.jsonl"), "{usage}");
    }

    #[test]
    fn resume_report_carries_the_path_and_the_counts() {
        let report = format_resume_report("/sessions/s.jsonl", 14, 6);
        assert!(report.contains("Session file     /sessions/s.jsonl"), "{report}");
        assert!(report.contains("Messages         14"), "{report}");
        assert!(report.contains("Turns            6"), "{report}");
    }

    #[test]
    fn auto_compaction_notice_carries_the_count() {
        assert_eq!(format_auto_compaction_notice(5), "[auto-compacted: removed 5 messages]");
    }
}
