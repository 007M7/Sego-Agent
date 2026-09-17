//! The per-family slash-command handlers: plugins, agents, skills and MCP, plus
//! the plugin report rendering they use.
//!
//! Moved out of `lib.rs` as one layer (DEV-STRUCT-01). Bodies are verbatim.
//!
//! `handle_slash_command` stays in the crate root: it is the dispatcher that
//! routes a parsed command to one of these, so moving it away from the type it
//! matches on would separate the two halves of the same decision.

use std::path::Path;

use plugins::{PluginError, PluginManager, PluginSummary};

use crate::definitions::{
    discover_definition_roots, discover_skill_roots, install_skill, load_agents_from_roots,
    load_skills_from_roots,
};
use crate::reports::{
    normalize_optional_args, render_agents_report, render_agents_usage, render_mcp_server_report,
    render_mcp_summary_report, render_mcp_usage, render_skill_install_report, render_skills_report,
    render_skills_usage,
};
use runtime::ConfigLoader;

use crate::PluginsCommandResult;

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

pub(crate) fn render_mcp_report_for(
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

pub(crate) fn render_plugin_install_report(
    plugin_id: &str,
    plugin: Option<&PluginSummary>,
) -> String {
    let name = plugin.map_or(plugin_id, |plugin| plugin.metadata.name.as_str());
    let version = plugin.map_or("unknown", |plugin| plugin.metadata.version.as_str());
    let enabled = plugin.is_some_and(|plugin| plugin.enabled);
    format!(
        "Plugins\n  Result           installed {plugin_id}\n  Name             {name}\n  Version          {version}\n  Status           {}",
        if enabled { "enabled" } else { "disabled" }
    )
}

pub(crate) fn resolve_plugin_target(
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
