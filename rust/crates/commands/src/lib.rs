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

mod parsing;

// `validate_slash_command_input` was public before the move and is used by the
// CLI crate, so it is re-exported publicly; the rest were crate-private.
pub use parsing::validate_slash_command_input;

mod definitions;

#[cfg(test)]
// The tests reach these two through `super::`.
pub(crate) use definitions::{load_agents_from_roots, load_skills_from_roots};

// Only the tests reach these; the library calls the rest of the layer
// through the names above.
#[cfg(test)]
pub(crate) use definitions::{install_skill_into, parse_skill_frontmatter};

mod reports;

pub(crate) use reports::normalize_optional_args;

// The renderers below are reached from the tests through `super::`; the
// library calls the rest of the layer.
#[cfg(test)]
pub(crate) use reports::{render_agents_report, render_skill_install_report, render_skills_report};

mod help;

pub(crate) use help::command_root_name;
pub use help::{
    render_slash_command_help, render_slash_command_help_detail, resume_supported_slash_commands,
    slash_command_specs, suggest_slash_commands,
};

// Reached only from inside `help` and from the tests.
#[cfg(test)]
pub(crate) use help::slash_command_category;

mod handlers;

pub use handlers::{
    handle_agents_slash_command, handle_mcp_slash_command, handle_plugins_slash_command,
    handle_skills_slash_command, render_plugins_report,
};

// Used inside `handlers` and by the tests.
#[cfg(test)]
pub(crate) use handlers::render_mcp_report_for;

#[cfg(test)]
mod tests;
