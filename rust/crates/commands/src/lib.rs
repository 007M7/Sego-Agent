use std::fmt;
use std::path::PathBuf;

use runtime::{compact_session, CompactionConfig, Session};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandManifestEntry {
    pub name: String,
    pub source: CommandSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSource {
    Builtin,
    InternalOnly,
    FeatureGated,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandRegistry {
    entries: Vec<CommandManifestEntry>,
}

impl CommandRegistry {
    #[must_use]
    pub fn new(entries: Vec<CommandManifestEntry>) -> Self {
        Self { entries }
    }

    #[must_use]
    pub fn entries(&self) -> &[CommandManifestEntry] {
        &self.entries
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    Help,
    Dir,
    Status,
    Sandbox,
    Pwd,
    Cd { path: Option<String> },
    Workspace { path: Option<String> },
    Compact,
    Bughunter { scope: Option<String> },
    Commit,
    Pr { context: Option<String> },
    Issue { context: Option<String> },
    Ultraplan { task: Option<String> },
    Teleport { target: Option<String> },
    DebugToolCall,
    Model { model: Option<String> },
    Permissions { mode: Option<String> },
    Clear { confirm: bool },
    Cost,
    Resume { session_path: Option<String> },
    Config { section: Option<String> },
    Mcp { action: Option<String>, target: Option<String> },
    Memory,
    Init,
    Diff,
    Version,
    Export { path: Option<String> },
    RecoveryExport { path: Option<String> },
    Session { action: Option<String>, target: Option<String> },
    Plugins { action: Option<String>, target: Option<String> },
    Agents { args: Option<String> },
    Skills { args: Option<String> },
    Doctor,
    Login,
    Logout,
    Vim,
    Upgrade,
    Stats,
    Share,
    Feedback,
    Files,
    Fast,
    Exit,
    Summary,
    Desktop,
    Brief,
    Advisor,
    Stickers,
    Insights,
    Thinkback,
    ReleaseNotes,
    SecurityReview,
    Keybindings,
    PrivacySettings,
    Plan { mode: Option<String> },
    Review { scope: Option<String> },
    Verify { scope: Option<String> },
    Tasks { args: Option<String> },
    Theme { name: Option<String> },
    Voice { mode: Option<String> },
    Usage { scope: Option<String> },
    Rename { name: Option<String> },
    Copy { target: Option<String> },
    Hooks { args: Option<String> },
    Context { action: Option<String> },
    Color { scheme: Option<String> },
    Effort { level: Option<String> },
    Branch { name: Option<String> },
    Rewind { steps: Option<String> },
    Ide { target: Option<String> },
    Tag { label: Option<String> },
    OutputStyle { style: Option<String> },
    AddDir { path: Option<String> },
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommandParseError {
    message: String,
}

impl SlashCommandParseError {
    fn new(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }
}

impl fmt::Display for SlashCommandParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for SlashCommandParseError {}

impl SlashCommand {
    pub fn parse(input: &str) -> Result<Option<Self>, SlashCommandParseError> {
        validate_slash_command_input(input)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommandResult {
    pub message: String,
    pub session: Session,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginsCommandResult {
    pub message: String,
    pub reload_runtime: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DefinitionSource {
    ProjectCodex,
    ProjectClaude,
    UserCodexHome,
    UserCodex,
    UserClaude,
}

impl DefinitionSource {
    fn label(self) -> &'static str {
        match self {
            Self::ProjectCodex => "Project (.codex)",
            Self::ProjectClaude => "Project (.claude)",
            Self::UserCodexHome => "User ($CODEX_HOME)",
            Self::UserCodex => "User (~/.codex)",
            Self::UserClaude => "User (~/.claude)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentSummary {
    name: String,
    description: Option<String>,
    model: Option<String>,
    reasoning_effort: Option<String>,
    source: DefinitionSource,
    shadowed_by: Option<DefinitionSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillSummary {
    name: String,
    description: Option<String>,
    source: DefinitionSource,
    shadowed_by: Option<DefinitionSource>,
    origin: SkillOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkillOrigin {
    SkillsDir,
    LegacyCommandsDir,
}

impl SkillOrigin {
    fn detail_label(self) -> Option<&'static str> {
        match self {
            Self::SkillsDir => None,
            Self::LegacyCommandsDir => Some("legacy /commands"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SkillRoot {
    source: DefinitionSource,
    path: PathBuf,
    origin: SkillOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstalledSkill {
    invocation_name: String,
    display_name: Option<String>,
    source: PathBuf,
    registry_root: PathBuf,
    installed_path: PathBuf,
}

#[must_use]
pub fn handle_slash_command(
    input: &str,
    session: &Session,
    compaction: CompactionConfig,
) -> Option<SlashCommandResult> {
    let command = match SlashCommand::parse(input) {
        Ok(Some(command)) => command,
        Ok(None) => return None,
        Err(error) => {
            return Some(SlashCommandResult {
                message: error.to_string(),
                session: session.clone(),
            });
        }
    };

    match command {
        SlashCommand::Compact => {
            let result = compact_session(session, compaction);
            let message = if result.removed_message_count == 0 {
                "Compaction skipped: session is below the compaction threshold.".to_string()
            } else {
                format!(
                    "Compacted {} messages into a resumable system summary.",
                    result.removed_message_count
                )
            };
            Some(SlashCommandResult { message, session: result.compacted_session })
        }
        SlashCommand::Help | SlashCommand::Dir => Some(SlashCommandResult {
            message: render_slash_command_help(),
            session: session.clone(),
        }),
        SlashCommand::Status
        | SlashCommand::Bughunter { .. }
        | SlashCommand::Commit
        | SlashCommand::Pr { .. }
        | SlashCommand::Issue { .. }
        | SlashCommand::Ultraplan { .. }
        | SlashCommand::Teleport { .. }
        | SlashCommand::DebugToolCall
        | SlashCommand::Sandbox
        | SlashCommand::Pwd
        | SlashCommand::Cd { .. }
        | SlashCommand::Workspace { .. }
        | SlashCommand::Model { .. }
        | SlashCommand::Permissions { .. }
        | SlashCommand::Clear { .. }
        | SlashCommand::Cost
        | SlashCommand::Resume { .. }
        | SlashCommand::Config { .. }
        | SlashCommand::Mcp { .. }
        | SlashCommand::Memory
        | SlashCommand::Init
        | SlashCommand::Diff
        | SlashCommand::Version
        | SlashCommand::Export { .. }
        | SlashCommand::RecoveryExport { .. }
        | SlashCommand::Session { .. }
        | SlashCommand::Plugins { .. }
        | SlashCommand::Agents { .. }
        | SlashCommand::Skills { .. }
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
        | SlashCommand::AddDir { .. }
        | SlashCommand::Unknown(_) => None,
    }
}

mod specs;

#[cfg(test)]
// The tests walk the table; nothing in the library reaches it from here.
pub(crate) use specs::SLASH_COMMAND_SPECS;

mod parsing;

// `validate_slash_command_input` was public before the move and is used by the
// CLI crate, so it is re-exported publicly; the rest were crate-private.
pub use parsing::validate_slash_command_input;
#[cfg(test)]
// The names below are used by the tests through `super::`; the library itself
// calls them from inside `parsing`, so re-exporting them here under
// `cfg(test)` keeps the build warning-free without hiding anything.
pub(crate) use parsing::{
    command_error, optional_single_arg, parse_clear_args, parse_config_section,
    parse_list_or_help_args, parse_mcp_command, parse_permissions_mode, parse_plugin_command,
    parse_session_command, parse_skills_args, remainder_after_command, require_remainder,
    usage_error, validate_no_args,
};

mod definitions;

#[cfg(test)]
// Reached from inside `definitions` and from the tests now.
pub(crate) use definitions::{
    discover_definition_roots, discover_skill_roots, install_skill, load_agents_from_roots,
    load_skills_from_roots,
};

// Only the tests reach these; the library calls the rest of the layer
// through the names above.
#[cfg(test)]
pub(crate) use definitions::{
    copy_directory_contents, default_skill_install_root, derive_skill_install_name,
    install_skill_into, parse_skill_frontmatter, parse_toml_string, push_unique_root,
    push_unique_skill_root, resolve_skill_install_source, sanitize_skill_invocation_name,
    unquote_frontmatter_value,
};

mod reports;

pub(crate) use reports::normalize_optional_args;

// The renderers below are used by the handlers and the tests.
#[cfg(test)]
pub(crate) use reports::{
    render_agents_report, render_agents_usage, render_mcp_server_report, render_mcp_summary_report,
    render_mcp_usage, render_skill_install_report, render_skills_report, render_skills_usage,
};

// The formatting helpers below are called only from inside the renderers and
// from the tests, so the crate root re-exports them under `cfg(test)`.
#[cfg(test)]
pub(crate) use reports::{
    agent_detail, config_source_label, format_mcp_oauth, format_optional_keys,
    format_optional_list, mcp_server_summary, mcp_transport_label,
};

mod help;

pub(crate) use help::command_root_name;
pub use help::{
    render_slash_command_help, render_slash_command_help_detail, resume_supported_slash_commands,
    slash_command_specs, suggest_slash_commands,
};

// Reached only from inside `help` and from the tests.
#[cfg(test)]
pub(crate) use help::{
    find_slash_command_spec, format_slash_command_help_line, levenshtein_distance,
    slash_command_category, slash_command_detail_lines, slash_command_usage,
};

mod handlers;

pub use handlers::{
    handle_agents_slash_command, handle_mcp_slash_command, handle_plugins_slash_command,
    handle_skills_slash_command, render_plugins_report,
};

// Used inside `handlers` and by the tests.
#[cfg(test)]
pub(crate) use handlers::{
    render_mcp_report_for, render_plugin_install_report, resolve_plugin_target,
};

#[cfg(test)]
mod tests;
