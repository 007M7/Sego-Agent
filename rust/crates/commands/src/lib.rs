use std::fmt;
use std::path::{Path, PathBuf};

use plugins::{PluginError, PluginManager, PluginSummary};
use runtime::{compact_session, CompactionConfig, ConfigLoader, Session};

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

#[allow(clippy::too_many_lines)]
pub fn handle_plugins_slash_command(
    action: Option<&str>,
    target: Option<&str>,
    manager: &mut PluginManager,
) -> Result<PluginsCommandResult, PluginError> {
    match action {
        None | Some("list") => Ok(PluginsCommandResult {
            message: render_plugins_report(&manager.list_installed_plugins()?),
            reload_runtime: false,
        }),
        Some("install") => {
            let Some(target) = target else {
                return Ok(PluginsCommandResult {
                    message: "Usage: /plugins install <path>".to_string(),
                    reload_runtime: false,
                });
            };
            let install = manager.install(target)?;
            let plugin = manager
                .list_installed_plugins()?
                .into_iter()
                .find(|plugin| plugin.metadata.id == install.plugin_id);
            Ok(PluginsCommandResult {
                message: render_plugin_install_report(&install.plugin_id, plugin.as_ref()),
                reload_runtime: true,
            })
        }
        Some("enable") => {
            let Some(target) = target else {
                return Ok(PluginsCommandResult {
                    message: "Usage: /plugins enable <name>".to_string(),
                    reload_runtime: false,
                });
            };
            let plugin = resolve_plugin_target(manager, target)?;
            manager.enable(&plugin.metadata.id)?;
            Ok(PluginsCommandResult {
                message: format!(
                    "Plugins\n  Result           enabled {}\n  Name             {}\n  Version          {}\n  Status           enabled",
                    plugin.metadata.id, plugin.metadata.name, plugin.metadata.version
                ),
                reload_runtime: true,
            })
        }
        Some("disable") => {
            let Some(target) = target else {
                return Ok(PluginsCommandResult {
                    message: "Usage: /plugins disable <name>".to_string(),
                    reload_runtime: false,
                });
            };
            let plugin = resolve_plugin_target(manager, target)?;
            manager.disable(&plugin.metadata.id)?;
            Ok(PluginsCommandResult {
                message: format!(
                    "Plugins\n  Result           disabled {}\n  Name             {}\n  Version          {}\n  Status           disabled",
                    plugin.metadata.id, plugin.metadata.name, plugin.metadata.version
                ),
                reload_runtime: true,
            })
        }
        Some("uninstall") => {
            let Some(target) = target else {
                return Ok(PluginsCommandResult {
                    message: "Usage: /plugins uninstall <plugin-id>".to_string(),
                    reload_runtime: false,
                });
            };
            manager.uninstall(target)?;
            Ok(PluginsCommandResult {
                message: format!("Plugins\n  Result           uninstalled {target}"),
                reload_runtime: true,
            })
        }
        Some("update") => {
            let Some(target) = target else {
                return Ok(PluginsCommandResult {
                    message: "Usage: /plugins update <plugin-id>".to_string(),
                    reload_runtime: false,
                });
            };
            let update = manager.update(target)?;
            let plugin = manager
                .list_installed_plugins()?
                .into_iter()
                .find(|plugin| plugin.metadata.id == update.plugin_id);
            Ok(PluginsCommandResult {
                message: format!(
                    "Plugins\n  Result           updated {}\n  Name             {}\n  Old version      {}\n  New version      {}\n  Status           {}",
                    update.plugin_id,
                    plugin
                        .as_ref()
                        .map_or_else(|| update.plugin_id.clone(), |plugin| plugin.metadata.name.clone()),
                    update.old_version,
                    update.new_version,
                    plugin
                        .as_ref()
                        .map_or("unknown", |plugin| if plugin.enabled { "enabled" } else { "disabled" }),
                ),
                reload_runtime: true,
            })
        }
        Some(other) => Ok(PluginsCommandResult {
            message: format!(
                "Unknown /plugins action '{other}'. Use list, install, enable, disable, uninstall, or update."
            ),
            reload_runtime: false,
        }),
    }
}

pub fn handle_agents_slash_command(args: Option<&str>, cwd: &Path) -> std::io::Result<String> {
    match normalize_optional_args(args) {
        None | Some("list") => {
            let roots = discover_definition_roots(cwd, "agents");
            let agents = load_agents_from_roots(&roots)?;
            Ok(render_agents_report(&agents))
        }
        Some("-h" | "--help" | "help") => Ok(render_agents_usage(None)),
        Some(args) => Ok(render_agents_usage(Some(args))),
    }
}

pub fn handle_mcp_slash_command(
    args: Option<&str>,
    cwd: &Path,
) -> Result<String, runtime::ConfigError> {
    let loader = ConfigLoader::default_for(cwd);
    render_mcp_report_for(&loader, cwd, args)
}

pub fn handle_skills_slash_command(args: Option<&str>, cwd: &Path) -> std::io::Result<String> {
    match normalize_optional_args(args) {
        None | Some("list") => {
            let roots = discover_skill_roots(cwd);
            let skills = load_skills_from_roots(&roots)?;
            Ok(render_skills_report(&skills))
        }
        Some("install") => Ok(render_skills_usage(Some("install"))),
        Some(args) if args.starts_with("install ") => {
            let target = args["install ".len()..].trim();
            if target.is_empty() {
                return Ok(render_skills_usage(Some("install")));
            }
            let install = install_skill(target, cwd)?;
            Ok(render_skill_install_report(&install))
        }
        Some("-h" | "--help" | "help") => Ok(render_skills_usage(None)),
        Some(args) => Ok(render_skills_usage(Some(args))),
    }
}

fn render_mcp_report_for(
    loader: &ConfigLoader,
    cwd: &Path,
    args: Option<&str>,
) -> Result<String, runtime::ConfigError> {
    match normalize_optional_args(args) {
        None | Some("list") => {
            let runtime_config = loader.load()?;
            Ok(render_mcp_summary_report(cwd, runtime_config.mcp().servers()))
        }
        Some("-h" | "--help" | "help") => Ok(render_mcp_usage(None)),
        Some("show") => Ok(render_mcp_usage(Some("show"))),
        Some(args) if args.split_whitespace().next() == Some("show") => {
            let mut parts = args.split_whitespace();
            let _ = parts.next();
            let Some(server_name) = parts.next() else {
                return Ok(render_mcp_usage(Some("show")));
            };
            if parts.next().is_some() {
                return Ok(render_mcp_usage(Some(args)));
            }
            let runtime_config = loader.load()?;
            Ok(render_mcp_server_report(cwd, server_name, runtime_config.mcp().get(server_name)))
        }
        Some(args) => Ok(render_mcp_usage(Some(args))),
    }
}

#[must_use]
pub fn render_plugins_report(plugins: &[PluginSummary]) -> String {
    let mut lines = vec!["Plugins".to_string()];
    if plugins.is_empty() {
        lines.push("  No plugins installed.".to_string());
        return lines.join("\n");
    }
    for plugin in plugins {
        let enabled = if plugin.enabled { "enabled" } else { "disabled" };
        lines.push(format!(
            "  {name:<20} v{version:<10} {enabled}",
            name = plugin.metadata.name,
            version = plugin.metadata.version,
        ));
    }
    lines.join("\n")
}

fn render_plugin_install_report(plugin_id: &str, plugin: Option<&PluginSummary>) -> String {
    let name = plugin.map_or(plugin_id, |plugin| plugin.metadata.name.as_str());
    let version = plugin.map_or("unknown", |plugin| plugin.metadata.version.as_str());
    let enabled = plugin.is_some_and(|plugin| plugin.enabled);
    format!(
        "Plugins\n  Result           installed {plugin_id}\n  Name             {name}\n  Version          {version}\n  Status           {}",
        if enabled { "enabled" } else { "disabled" }
    )
}

fn resolve_plugin_target(
    manager: &PluginManager,
    target: &str,
) -> Result<PluginSummary, PluginError> {
    let mut matches = manager
        .list_installed_plugins()?
        .into_iter()
        .filter(|plugin| plugin.metadata.id == target || plugin.metadata.name == target)
        .collect::<Vec<_>>();
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(PluginError::NotFound(format!(
            "plugin `{target}` is not installed or discoverable"
        ))),
        _ => Err(PluginError::InvalidManifest(format!(
            "plugin name `{target}` is ambiguous; use the full plugin id"
        ))),
    }
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

pub(crate) use reports::{
    normalize_optional_args, render_agents_report, render_agents_usage, render_mcp_server_report,
    render_mcp_summary_report, render_mcp_usage, render_skill_install_report, render_skills_report,
    render_skills_usage,
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

#[cfg(test)]
mod tests;
