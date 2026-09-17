//! Tests for the slash-command surface (moved out of `lib.rs`).
//!
//! This is a move, not a rewrite: the module keeps its name and its
//! `use super::*` imports, and `super` is still the crate root here.
//! Keeping it in a separate file lets the production surface in `lib.rs`
//! be read without scrolling past a thousand lines of tests.

use super::{
    handle_plugins_slash_command, handle_slash_command, load_agents_from_roots,
    load_skills_from_roots, render_agents_report, render_plugins_report, render_skills_report,
    render_slash_command_help, render_slash_command_help_detail, resume_supported_slash_commands,
    slash_command_category, slash_command_specs, suggest_slash_commands,
    validate_slash_command_input, DefinitionSource, SkillOrigin, SkillRoot, SlashCommand,
};
use plugins::{PluginKind, PluginManager, PluginManagerConfig, PluginMetadata, PluginSummary};
use runtime::{
    CompactionConfig, ConfigLoader, ContentBlock, ConversationMessage, MessageRole, Session,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("commands-plugin-{label}-{nanos}"))
}

fn write_external_plugin(root: &Path, name: &str, version: &str) {
    fs::create_dir_all(root.join(".claude-plugin")).expect("manifest dir");
    fs::write(
        root.join(".claude-plugin").join("plugin.json"),
        format!(
            "{{\n  \"name\": \"{name}\",\n  \"version\": \"{version}\",\n  \"description\": \"commands plugin\"\n}}"
        ),
    )
    .expect("write manifest");
}

fn write_bundled_plugin(root: &Path, name: &str, version: &str, default_enabled: bool) {
    fs::create_dir_all(root.join(".claude-plugin")).expect("manifest dir");
    fs::write(
        root.join(".claude-plugin").join("plugin.json"),
        format!(
            "{{\n  \"name\": \"{name}\",\n  \"version\": \"{version}\",\n  \"description\": \"bundled commands plugin\",\n  \"defaultEnabled\": {}\n}}",
            if default_enabled { "true" } else { "false" }
        ),
    )
    .expect("write bundled manifest");
}

fn write_agent(root: &Path, name: &str, description: &str, model: &str, reasoning: &str) {
    fs::create_dir_all(root).expect("agent root");
    fs::write(
        root.join(format!("{name}.toml")),
        format!(
            "name = \"{name}\"\ndescription = \"{description}\"\nmodel = \"{model}\"\nmodel_reasoning_effort = \"{reasoning}\"\n"
        ),
    )
    .expect("write agent");
}

fn write_skill(root: &Path, name: &str, description: &str) {
    let skill_root = root.join(name);
    fs::create_dir_all(&skill_root).expect("skill root");
    fs::write(
        skill_root.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n"),
    )
    .expect("write skill");
}

fn write_legacy_command(root: &Path, name: &str, description: &str) {
    fs::create_dir_all(root).expect("commands root");
    fs::write(
        root.join(format!("{name}.md")),
        format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n"),
    )
    .expect("write command");
}

fn parse_error_message(input: &str) -> String {
    SlashCommand::parse(input).expect_err("slash command should be rejected").to_string()
}

#[allow(clippy::too_many_lines)]
#[test]
fn parses_supported_slash_commands() {
    assert_eq!(SlashCommand::parse("/help"), Ok(Some(SlashCommand::Help)));
    assert_eq!(SlashCommand::parse("/dir"), Ok(Some(SlashCommand::Dir)));
    assert_eq!(SlashCommand::parse(" /status "), Ok(Some(SlashCommand::Status)));
    assert_eq!(SlashCommand::parse("/sandbox"), Ok(Some(SlashCommand::Sandbox)));
    assert_eq!(SlashCommand::parse("/pwd"), Ok(Some(SlashCommand::Pwd)));
    assert_eq!(SlashCommand::parse("/cwd"), Ok(Some(SlashCommand::Pwd)));
    assert_eq!(
        SlashCommand::parse("/cd D:\\Project"),
        Ok(Some(SlashCommand::Cd { path: Some("D:\\Project".to_string()) }))
    );
    assert_eq!(
        SlashCommand::parse("/workspace D:\\Project"),
        Ok(Some(SlashCommand::Workspace { path: Some("D:\\Project".to_string()) }))
    );
    assert_eq!(SlashCommand::parse("/workdir"), Ok(Some(SlashCommand::Workspace { path: None })));
    assert_eq!(
        SlashCommand::parse("/bughunter runtime"),
        Ok(Some(SlashCommand::Bughunter { scope: Some("runtime".to_string()) }))
    );
    assert_eq!(SlashCommand::parse("/commit"), Ok(Some(SlashCommand::Commit)));
    assert_eq!(
        SlashCommand::parse("/pr ready for review"),
        Ok(Some(SlashCommand::Pr { context: Some("ready for review".to_string()) }))
    );
    assert_eq!(
        SlashCommand::parse("/issue flaky test"),
        Ok(Some(SlashCommand::Issue { context: Some("flaky test".to_string()) }))
    );
    assert_eq!(
        SlashCommand::parse("/ultraplan ship both features"),
        Ok(Some(SlashCommand::Ultraplan { task: Some("ship both features".to_string()) }))
    );
    assert_eq!(
        SlashCommand::parse("/teleport conversation.rs"),
        Ok(Some(SlashCommand::Teleport { target: Some("conversation.rs".to_string()) }))
    );
    assert_eq!(SlashCommand::parse("/debug-tool-call"), Ok(Some(SlashCommand::DebugToolCall)));
    assert_eq!(
        SlashCommand::parse("/bughunter runtime"),
        Ok(Some(SlashCommand::Bughunter { scope: Some("runtime".to_string()) }))
    );
    assert_eq!(SlashCommand::parse("/commit"), Ok(Some(SlashCommand::Commit)));
    assert_eq!(
        SlashCommand::parse("/pr ready for review"),
        Ok(Some(SlashCommand::Pr { context: Some("ready for review".to_string()) }))
    );
    assert_eq!(
        SlashCommand::parse("/issue flaky test"),
        Ok(Some(SlashCommand::Issue { context: Some("flaky test".to_string()) }))
    );
    assert_eq!(
        SlashCommand::parse("/ultraplan ship both features"),
        Ok(Some(SlashCommand::Ultraplan { task: Some("ship both features".to_string()) }))
    );
    assert_eq!(
        SlashCommand::parse("/teleport conversation.rs"),
        Ok(Some(SlashCommand::Teleport { target: Some("conversation.rs".to_string()) }))
    );
    assert_eq!(SlashCommand::parse("/debug-tool-call"), Ok(Some(SlashCommand::DebugToolCall)));
    assert_eq!(
        SlashCommand::parse("/model claude-opus"),
        Ok(Some(SlashCommand::Model { model: Some("claude-opus".to_string()) }))
    );
    assert_eq!(SlashCommand::parse("/model"), Ok(Some(SlashCommand::Model { model: None })));
    assert_eq!(
        SlashCommand::parse("/permissions read-only"),
        Ok(Some(SlashCommand::Permissions { mode: Some("read-only".to_string()) }))
    );
    assert_eq!(SlashCommand::parse("/clear"), Ok(Some(SlashCommand::Clear { confirm: false })));
    assert_eq!(
        SlashCommand::parse("/clear --confirm"),
        Ok(Some(SlashCommand::Clear { confirm: true }))
    );
    assert_eq!(SlashCommand::parse("/cost"), Ok(Some(SlashCommand::Cost)));
    assert_eq!(
        SlashCommand::parse("/resume session.json"),
        Ok(Some(SlashCommand::Resume { session_path: Some("session.json".to_string()) }))
    );
    assert_eq!(SlashCommand::parse("/config"), Ok(Some(SlashCommand::Config { section: None })));
    assert_eq!(
        SlashCommand::parse("/config env"),
        Ok(Some(SlashCommand::Config { section: Some("env".to_string()) }))
    );
    assert_eq!(
        SlashCommand::parse("/mcp"),
        Ok(Some(SlashCommand::Mcp { action: None, target: None }))
    );
    assert_eq!(
        SlashCommand::parse("/mcp show remote"),
        Ok(Some(SlashCommand::Mcp {
            action: Some("show".to_string()),
            target: Some("remote".to_string())
        }))
    );
    assert_eq!(SlashCommand::parse("/memory"), Ok(Some(SlashCommand::Memory)));
    assert_eq!(SlashCommand::parse("/init"), Ok(Some(SlashCommand::Init)));
    assert_eq!(SlashCommand::parse("/diff"), Ok(Some(SlashCommand::Diff)));
    assert_eq!(SlashCommand::parse("/version"), Ok(Some(SlashCommand::Version)));
    assert_eq!(
        SlashCommand::parse("/export notes.txt"),
        Ok(Some(SlashCommand::Export { path: Some("notes.txt".to_string()) }))
    );
    assert_eq!(
        SlashCommand::parse("/session switch abc123"),
        Ok(Some(SlashCommand::Session {
            action: Some("switch".to_string()),
            target: Some("abc123".to_string())
        }))
    );
    assert_eq!(
        SlashCommand::parse("/plugins install demo"),
        Ok(Some(SlashCommand::Plugins {
            action: Some("install".to_string()),
            target: Some("demo".to_string())
        }))
    );
    assert_eq!(
        SlashCommand::parse("/plugins list"),
        Ok(Some(SlashCommand::Plugins { action: Some("list".to_string()), target: None }))
    );
    assert_eq!(
        SlashCommand::parse("/plugins enable demo"),
        Ok(Some(SlashCommand::Plugins {
            action: Some("enable".to_string()),
            target: Some("demo".to_string())
        }))
    );
    assert_eq!(
        SlashCommand::parse("/skills install ./fixtures/help-skill"),
        Ok(Some(SlashCommand::Skills { args: Some("install ./fixtures/help-skill".to_string()) }))
    );
    assert_eq!(
        SlashCommand::parse("/plugins disable demo"),
        Ok(Some(SlashCommand::Plugins {
            action: Some("disable".to_string()),
            target: Some("demo".to_string())
        }))
    );
    assert_eq!(
        SlashCommand::parse("/session fork incident-review"),
        Ok(Some(SlashCommand::Session {
            action: Some("fork".to_string()),
            target: Some("incident-review".to_string())
        }))
    );
}

#[test]
fn rejects_unexpected_arguments_for_no_arg_commands() {
    // given
    let input = "/compact now";

    // when
    let error = parse_error_message(input);

    // then
    assert!(error.contains("Unexpected arguments for /compact."));
    assert!(error.contains("  Usage            /compact"));
    assert!(error.contains("  Summary          Compact local session history"));
}

#[test]
fn rejects_invalid_argument_values() {
    // given
    let input = "/permissions admin";

    // when
    let error = parse_error_message(input);

    // then
    assert!(error.contains(
        "Unsupported /permissions mode 'admin'. Use read-only, workspace-write, or danger-full-access."
    ));
    assert!(error.contains(
        "  Usage            /permissions [read-only|workspace-write|danger-full-access]"
    ));
}

#[test]
fn rejects_missing_required_arguments() {
    // given
    let input = "/teleport";

    // when
    let error = parse_error_message(input);

    // then
    assert!(error.contains("Usage: /teleport <symbol-or-path>"));
    assert!(error.contains("  Category         Discovery & debugging"));
}

#[test]
fn rejects_invalid_session_and_plugin_shapes() {
    // given
    let session_input = "/session switch";
    let plugin_input = "/plugins list extra";

    // when
    let session_error = parse_error_message(session_input);
    let plugin_error = parse_error_message(plugin_input);

    // then
    assert!(session_error.contains("Usage: /session switch <session-id>"));
    assert!(session_error.contains("/session"));
    assert!(plugin_error.contains("Usage: /plugin list"));
    assert!(plugin_error.contains("Aliases          /plugins, /marketplace"));
}

#[test]
fn rejects_invalid_agents_and_skills_arguments() {
    // given
    let agents_input = "/agents show planner";
    let skills_input = "/skills show help";

    // when
    let agents_error = parse_error_message(agents_input);
    let skills_error = parse_error_message(skills_input);

    // then
    assert!(agents_error.contains(
        "Unexpected arguments for /agents: show planner. Use /agents, /agents list, or /agents help."
    ));
    assert!(agents_error.contains("  Usage            /agents [list|help]"));
    assert!(skills_error.contains(
        "Unexpected arguments for /skills: show help. Use /skills, /skills list, /skills install <path>, or /skills help."
    ));
    assert!(skills_error.contains("  Usage            /skills [list|install <path>|help]"));
}

#[test]
fn rejects_invalid_mcp_arguments() {
    let show_error = parse_error_message("/mcp show alpha beta");
    assert!(show_error.contains("Unexpected arguments for /mcp show."));
    assert!(show_error.contains("  Usage            /mcp show <server>"));

    let action_error = parse_error_message("/mcp inspect alpha");
    assert!(
        action_error.contains("Unknown /mcp action 'inspect'. Use list, show <server>, or help.")
    );
    assert!(action_error.contains("  Usage            /mcp [list|show <server>|help]"));
}

#[test]
fn renders_help_from_shared_specs() {
    let help = render_slash_command_help();
    assert!(help.contains("Start here        /status, /diff, /agents, /skills, /commit"));
    assert!(help.contains("[resume]          also works with --resume SESSION.jsonl"));
    assert!(help.contains("Session & visibility"));
    assert!(help.contains("Workspace & git"));
    assert!(help.contains("Discovery & debugging"));
    assert!(help.contains("Analysis & automation"));
    assert!(help.contains("/help"));
    assert!(help.contains("/dir"));
    assert!(help.contains("/status"));
    assert!(help.contains("/sandbox"));
    assert!(help.contains("/compact"));
    assert!(help.contains("/bughunter [scope]"));
    assert!(help.contains("/commit"));
    assert!(help.contains("/pr [context]"));
    assert!(help.contains("/issue [context]"));
    assert!(help.contains("/ultraplan [task]"));
    assert!(help.contains("/teleport <symbol-or-path>"));
    assert!(help.contains("/debug-tool-call"));
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
    assert!(help.contains("/recovery-export [file]"));
    assert!(help.contains("/session [list|switch <session-id>|fork [branch-name]]"));
    assert!(help.contains("/sandbox"));
    assert!(help.contains(
        "/plugin [list|install <path>|enable <name>|disable <name>|uninstall <id>|update <id>]"
    ));
    assert!(help.contains("aliases: /plugins, /marketplace"));
    assert!(help.contains("/agents [list|help]"));
    assert!(help.contains("/skills [list|install <path>|help]"));
    assert!(help.contains("/verify [auto|fast|full]"));
    assert_eq!(slash_command_specs().len(), 147);
    assert!(resume_supported_slash_commands().len() >= 39);
}

#[test]
fn recovery_export_parses_with_and_without_path() {
    // 无参数：path 为 None
    let parsed = SlashCommand::parse("/recovery-export").expect("parse without path");
    assert!(matches!(parsed, Some(SlashCommand::RecoveryExport { path: None })));

    // 带参数：path 为 Some
    let parsed_with_path =
        SlashCommand::parse("/recovery-export summary.md").expect("parse with path");
    assert!(matches!(
        parsed_with_path,
        Some(SlashCommand::RecoveryExport { path: Some(ref p) }) if p == "summary.md"
    ));
}

#[test]
fn recovery_export_is_resume_supported_and_categorized() {
    // recovery-export 的 spec 必须标记 resume_supported = true
    let spec = slash_command_specs()
        .iter()
        .find(|spec| spec.name == "recovery-export")
        .expect("recovery-export spec must exist");
    assert!(spec.resume_supported, "recovery-export must be resume-supported");

    // recovery-export 必须归类到 Workspace & git（和 export 同组）
    assert_eq!(slash_command_category("recovery-export"), "Workspace & git");

    // recovery-export 必须出现在 help 里
    let help = render_slash_command_help();
    assert!(help.contains("/recovery-export"), "recovery-export must appear in help");
}

#[test]
fn renders_per_command_help_detail() {
    // given
    let command = "plugins";

    // when
    let help = render_slash_command_help_detail(command).expect("detail help should exist");

    // then
    assert!(help.contains("/plugin"));
    assert!(help.contains("Summary          Manage Sego Agent plugins"));
    assert!(help.contains("Aliases          /plugins, /marketplace"));
    assert!(help.contains("Category         Workspace & git"));
}

#[test]
fn renders_per_command_help_detail_for_mcp() {
    let help = render_slash_command_help_detail("mcp").expect("detail help should exist");
    assert!(help.contains("/mcp"));
    assert!(help.contains("Summary          Inspect configured MCP servers"));
    assert!(help.contains("Category         Discovery & debugging"));
    assert!(help.contains("Resume           Supported with --resume SESSION.jsonl"));
}

#[test]
fn validate_slash_command_input_rejects_extra_single_value_arguments() {
    // given
    let session_input = "/session switch current next";
    let plugin_input = "/plugin enable demo extra";

    // when
    let session_error = validate_slash_command_input(session_input)
        .expect_err("session input should be rejected")
        .to_string();
    let plugin_error = validate_slash_command_input(plugin_input)
        .expect_err("plugin input should be rejected")
        .to_string();

    // then
    assert!(session_error.contains("Unexpected arguments for /session switch."));
    assert!(session_error.contains("  Usage            /session switch <session-id>"));
    assert!(plugin_error.contains("Unexpected arguments for /plugin enable."));
    assert!(plugin_error.contains("  Usage            /plugin enable <name>"));
}

#[test]
fn suggests_closest_slash_commands_for_typos_and_aliases() {
    let suggestions = suggest_slash_commands("stats", 3);
    assert!(suggestions.contains(&"/stats".to_string()));
    assert!(suggestions.contains(&"/status".to_string()));
    assert!(suggestions.len() <= 3);
    let plugin_suggestions = suggest_slash_commands("/plugns", 3);
    assert!(plugin_suggestions.contains(&"/plugin".to_string()));
    assert_eq!(suggest_slash_commands("zzz", 3), Vec::<String>::new());
}

#[test]
fn compacts_sessions_via_slash_command() {
    let mut session = Session::new();
    session.messages = vec![
        ConversationMessage::user_text("a ".repeat(200)),
        ConversationMessage::assistant(vec![ContentBlock::Text { text: "b ".repeat(200) }]),
        ConversationMessage::tool_result("1", "bash", "ok ".repeat(200), false),
        ConversationMessage::assistant(vec![ContentBlock::Text { text: "recent".to_string() }]),
    ];

    let result = handle_slash_command(
        "/compact",
        &session,
        CompactionConfig { preserve_recent_messages: 2, max_estimated_tokens: 1 },
    )
    .expect("slash command should be handled");

    assert!(result.message.contains("Compacted 2 messages"));
    assert_eq!(result.session.messages[0].role, MessageRole::System);
}

#[test]
fn help_command_is_non_mutating() {
    let session = Session::new();
    let result = handle_slash_command("/help", &session, CompactionConfig::default())
        .expect("help command should be handled");
    assert_eq!(result.session, session);
    assert!(result.message.contains("Slash commands"));
}

#[test]
fn ignores_unknown_or_runtime_bound_slash_commands() {
    let session = Session::new();
    assert!(handle_slash_command("/unknown", &session, CompactionConfig::default()).is_none());
    assert!(handle_slash_command("/status", &session, CompactionConfig::default()).is_none());
    assert!(handle_slash_command("/sandbox", &session, CompactionConfig::default()).is_none());
    assert!(handle_slash_command("/bughunter", &session, CompactionConfig::default()).is_none());
    assert!(handle_slash_command("/commit", &session, CompactionConfig::default()).is_none());
    assert!(handle_slash_command("/pr", &session, CompactionConfig::default()).is_none());
    assert!(handle_slash_command("/issue", &session, CompactionConfig::default()).is_none());
    assert!(handle_slash_command("/ultraplan", &session, CompactionConfig::default()).is_none());
    assert!(handle_slash_command("/teleport foo", &session, CompactionConfig::default()).is_none());
    assert!(
        handle_slash_command("/debug-tool-call", &session, CompactionConfig::default()).is_none()
    );
    assert!(handle_slash_command("/model claude", &session, CompactionConfig::default()).is_none());
    assert!(handle_slash_command("/permissions read-only", &session, CompactionConfig::default())
        .is_none());
    assert!(handle_slash_command("/clear", &session, CompactionConfig::default()).is_none());
    assert!(
        handle_slash_command("/clear --confirm", &session, CompactionConfig::default()).is_none()
    );
    assert!(handle_slash_command("/cost", &session, CompactionConfig::default()).is_none());
    assert!(handle_slash_command("/resume session.json", &session, CompactionConfig::default())
        .is_none());
    assert!(handle_slash_command("/resume session.jsonl", &session, CompactionConfig::default())
        .is_none());
    assert!(handle_slash_command("/config", &session, CompactionConfig::default()).is_none());
    assert!(handle_slash_command("/config env", &session, CompactionConfig::default()).is_none());
    assert!(handle_slash_command("/mcp list", &session, CompactionConfig::default()).is_none());
    assert!(handle_slash_command("/diff", &session, CompactionConfig::default()).is_none());
    assert!(handle_slash_command("/version", &session, CompactionConfig::default()).is_none());
    assert!(
        handle_slash_command("/export note.txt", &session, CompactionConfig::default()).is_none()
    );
    assert!(handle_slash_command("/session list", &session, CompactionConfig::default()).is_none());
    assert!(handle_slash_command("/plugins list", &session, CompactionConfig::default()).is_none());
}

#[test]
fn renders_plugins_report_with_name_version_and_status() {
    let rendered = render_plugins_report(&[
        PluginSummary {
            metadata: PluginMetadata {
                id: "demo@external".to_string(),
                name: "demo".to_string(),
                version: "1.2.3".to_string(),
                description: "demo plugin".to_string(),
                kind: PluginKind::External,
                source: "demo".to_string(),
                default_enabled: false,
                root: None,
            },
            enabled: true,
        },
        PluginSummary {
            metadata: PluginMetadata {
                id: "sample@external".to_string(),
                name: "sample".to_string(),
                version: "0.9.0".to_string(),
                description: "sample plugin".to_string(),
                kind: PluginKind::External,
                source: "sample".to_string(),
                default_enabled: false,
                root: None,
            },
            enabled: false,
        },
    ]);

    assert!(rendered.contains("demo"));
    assert!(rendered.contains("v1.2.3"));
    assert!(rendered.contains("enabled"));
    assert!(rendered.contains("sample"));
    assert!(rendered.contains("v0.9.0"));
    assert!(rendered.contains("disabled"));
}

#[test]
fn lists_agents_from_project_and_user_roots() {
    let workspace = temp_dir("agents-workspace");
    let project_agents = workspace.join(".codex").join("agents");
    let user_home = temp_dir("agents-home");
    let user_agents = user_home.join(".codex").join("agents");

    write_agent(&project_agents, "planner", "Project planner", "gpt-5.4", "medium");
    write_agent(&user_agents, "planner", "User planner", "gpt-5.4-mini", "high");
    write_agent(&user_agents, "verifier", "Verification agent", "gpt-5.4-mini", "high");

    let roots = vec![
        (DefinitionSource::ProjectCodex, project_agents),
        (DefinitionSource::UserCodex, user_agents),
    ];
    let report =
        render_agents_report(&load_agents_from_roots(&roots).expect("agent roots should load"));

    assert!(report.contains("Agents"));
    assert!(report.contains("2 active agents"));
    assert!(report.contains("Project (.codex):"));
    assert!(report.contains("planner · Project planner · gpt-5.4 · medium"));
    assert!(report.contains("User (~/.codex):"));
    assert!(report.contains("(shadowed by Project (.codex)) planner · User planner"));
    assert!(report.contains("verifier · Verification agent · gpt-5.4-mini · high"));

    let _ = fs::remove_dir_all(workspace);
    let _ = fs::remove_dir_all(user_home);
}

#[test]
fn lists_skills_from_project_and_user_roots() {
    let workspace = temp_dir("skills-workspace");
    let project_skills = workspace.join(".codex").join("skills");
    let project_commands = workspace.join(".claude").join("commands");
    let user_home = temp_dir("skills-home");
    let user_skills = user_home.join(".codex").join("skills");

    write_skill(&project_skills, "plan", "Project planning guidance");
    write_legacy_command(&project_commands, "deploy", "Legacy deployment guidance");
    write_skill(&user_skills, "plan", "User planning guidance");
    write_skill(&user_skills, "help", "Help guidance");

    let roots = vec![
        SkillRoot {
            source: DefinitionSource::ProjectCodex,
            path: project_skills,
            origin: SkillOrigin::SkillsDir,
        },
        SkillRoot {
            source: DefinitionSource::ProjectClaude,
            path: project_commands,
            origin: SkillOrigin::LegacyCommandsDir,
        },
        SkillRoot {
            source: DefinitionSource::UserCodex,
            path: user_skills,
            origin: SkillOrigin::SkillsDir,
        },
    ];
    let report =
        render_skills_report(&load_skills_from_roots(&roots).expect("skill roots should load"));

    assert!(report.contains("Skills"));
    assert!(report.contains("3 available skills"));
    assert!(report.contains("Project (.codex):"));
    assert!(report.contains("plan · Project planning guidance"));
    assert!(report.contains("Project (.claude):"));
    assert!(report.contains("deploy · Legacy deployment guidance · legacy /commands"));
    assert!(report.contains("User (~/.codex):"));
    assert!(report.contains("(shadowed by Project (.codex)) plan · User planning guidance"));
    assert!(report.contains("help · Help guidance"));

    let _ = fs::remove_dir_all(workspace);
    let _ = fs::remove_dir_all(user_home);
}

#[test]
fn agents_and_skills_usage_support_help_and_unexpected_args() {
    let cwd = temp_dir("slash-usage");

    let agents_help = super::handle_agents_slash_command(Some("help"), &cwd).expect("agents help");
    assert!(agents_help.contains("Usage            /agents [list|help]"));
    assert!(agents_help.contains("Direct CLI       claw agents"));

    let agents_unexpected =
        super::handle_agents_slash_command(Some("show planner"), &cwd).expect("agents usage");
    assert!(agents_unexpected.contains("Unexpected       show planner"));

    let skills_help =
        super::handle_skills_slash_command(Some("--help"), &cwd).expect("skills help");
    assert!(skills_help.contains("Usage            /skills [list|install <path>|help]"));
    assert!(skills_help.contains("Install root     $CODEX_HOME/skills or ~/.codex/skills"));
    assert!(skills_help.contains("legacy /commands"));

    let skills_unexpected =
        super::handle_skills_slash_command(Some("show help"), &cwd).expect("skills usage");
    assert!(skills_unexpected.contains("Unexpected       show help"));

    let _ = fs::remove_dir_all(cwd);
}

#[test]
fn mcp_usage_supports_help_and_unexpected_args() {
    let cwd = temp_dir("mcp-usage");

    let help = super::handle_mcp_slash_command(Some("help"), &cwd).expect("mcp help");
    assert!(help.contains("Usage            /mcp [list|show <server>|help]"));
    assert!(help.contains("Direct CLI       claw mcp [list|show <server>|help]"));

    let unexpected =
        super::handle_mcp_slash_command(Some("show alpha beta"), &cwd).expect("mcp usage");
    assert!(unexpected.contains("Unexpected       show alpha beta"));

    let _ = fs::remove_dir_all(cwd);
}

#[test]
fn renders_mcp_reports_from_loaded_config() {
    let workspace = temp_dir("mcp-config-workspace");
    let config_home = temp_dir("mcp-config-home");
    fs::create_dir_all(workspace.join(".claw")).expect("workspace config dir");
    fs::create_dir_all(&config_home).expect("config home");
    fs::write(
        workspace.join(".claw").join("settings.json"),
        r#"{
          "mcpServers": {
            "alpha": {
              "command": "uvx",
              "args": ["alpha-server"],
              "env": {"ALPHA_TOKEN": "secret"},
              "toolCallTimeoutMs": 1200
            },
            "remote": {
              "type": "http",
              "url": "https://remote.example/mcp",
              "headers": {"Authorization": "Bearer secret"},
              "headersHelper": "./bin/headers",
              "oauth": {
                "clientId": "remote-client",
                "callbackPort": 7878
              }
            }
          }
        }"#,
    )
    .expect("write settings");
    fs::write(
        workspace.join(".claw").join("settings.local.json"),
        r#"{
          "mcpServers": {
            "remote": {
              "type": "ws",
              "url": "wss://remote.example/mcp"
            }
          }
        }"#,
    )
    .expect("write local settings");

    let loader = ConfigLoader::new(&workspace, &config_home);
    let list = super::render_mcp_report_for(&loader, &workspace, None)
        .expect("mcp list report should render");
    assert!(list.contains("Configured servers 2"));
    assert!(list.contains("alpha"));
    assert!(list.contains("stdio"));
    assert!(list.contains("project"));
    assert!(list.contains("uvx alpha-server"));
    assert!(list.contains("remote"));
    assert!(list.contains("ws"));
    assert!(list.contains("local"));
    assert!(list.contains("wss://remote.example/mcp"));

    let show = super::render_mcp_report_for(&loader, &workspace, Some("show alpha"))
        .expect("mcp show report should render");
    assert!(show.contains("Name              alpha"));
    assert!(show.contains("Command           uvx"));
    assert!(show.contains("Args              alpha-server"));
    assert!(show.contains("Env keys          ALPHA_TOKEN"));
    assert!(show.contains("Tool timeout      1200 ms"));

    let remote = super::render_mcp_report_for(&loader, &workspace, Some("show remote"))
        .expect("mcp show remote report should render");
    assert!(remote.contains("Transport         ws"));
    assert!(remote.contains("URL               wss://remote.example/mcp"));

    let missing = super::render_mcp_report_for(&loader, &workspace, Some("show missing"))
        .expect("missing report should render");
    assert!(missing.contains("server `missing` is not configured"));

    let _ = fs::remove_dir_all(workspace);
    let _ = fs::remove_dir_all(config_home);
}

#[test]
fn parses_quoted_skill_frontmatter_values() {
    let contents = "---\nname: \"hud\"\ndescription: 'Quoted description'\n---\n";
    let (name, description) = super::parse_skill_frontmatter(contents);
    assert_eq!(name.as_deref(), Some("hud"));
    assert_eq!(description.as_deref(), Some("Quoted description"));
}

#[test]
fn installs_skill_into_user_registry_and_preserves_nested_files() {
    let workspace = temp_dir("skills-install-workspace");
    let source_root = workspace.join("source").join("help");
    let install_root = temp_dir("skills-install-root");
    write_skill(source_root.parent().expect("parent"), "help", "Helpful skill");
    let script_dir = source_root.join("scripts");
    fs::create_dir_all(&script_dir).expect("script dir");
    fs::write(script_dir.join("run.sh"), "#!/bin/sh\necho help\n").expect("write script");

    let installed = super::install_skill_into(
        source_root.to_str().expect("utf8 skill path"),
        &workspace,
        &install_root,
    )
    .expect("skill should install");

    assert_eq!(installed.invocation_name, "help");
    assert_eq!(installed.display_name.as_deref(), Some("help"));
    assert!(installed.installed_path.ends_with(Path::new("help")));
    assert!(installed.installed_path.join("SKILL.md").is_file());
    assert!(installed.installed_path.join("scripts").join("run.sh").is_file());

    let report = super::render_skill_install_report(&installed);
    assert!(report.contains("Result           installed help"));
    assert!(report.contains("Invoke as        $help"));
    assert!(report.contains(&install_root.display().to_string()));

    let roots = vec![SkillRoot {
        source: DefinitionSource::UserCodexHome,
        path: install_root.clone(),
        origin: SkillOrigin::SkillsDir,
    }];
    let listed = render_skills_report(
        &load_skills_from_roots(&roots).expect("installed skills should load"),
    );
    assert!(listed.contains("User ($CODEX_HOME):"));
    assert!(listed.contains("help · Helpful skill"));

    let _ = fs::remove_dir_all(workspace);
    let _ = fs::remove_dir_all(install_root);
}

#[test]
fn installs_plugin_from_path_and_lists_it() {
    let config_home = temp_dir("home");
    let source_root = temp_dir("source");
    write_external_plugin(&source_root, "demo", "1.0.0");

    let mut manager = PluginManager::new(PluginManagerConfig::new(&config_home));
    let install = handle_plugins_slash_command(
        Some("install"),
        Some(source_root.to_str().expect("utf8 path")),
        &mut manager,
    )
    .expect("install command should succeed");
    assert!(install.reload_runtime);
    assert!(install.message.contains("installed demo@external"));
    assert!(install.message.contains("Name             demo"));
    assert!(install.message.contains("Version          1.0.0"));
    assert!(install.message.contains("Status           enabled"));

    let list = handle_plugins_slash_command(Some("list"), None, &mut manager)
        .expect("list command should succeed");
    assert!(!list.reload_runtime);
    assert!(list.message.contains("demo"));
    assert!(list.message.contains("v1.0.0"));
    assert!(list.message.contains("enabled"));

    let _ = fs::remove_dir_all(config_home);
    let _ = fs::remove_dir_all(source_root);
}

#[test]
fn enables_and_disables_plugin_by_name() {
    let config_home = temp_dir("toggle-home");
    let source_root = temp_dir("toggle-source");
    write_external_plugin(&source_root, "demo", "1.0.0");

    let mut manager = PluginManager::new(PluginManagerConfig::new(&config_home));
    handle_plugins_slash_command(
        Some("install"),
        Some(source_root.to_str().expect("utf8 path")),
        &mut manager,
    )
    .expect("install command should succeed");

    let disable = handle_plugins_slash_command(Some("disable"), Some("demo"), &mut manager)
        .expect("disable command should succeed");
    assert!(disable.reload_runtime);
    assert!(disable.message.contains("disabled demo@external"));
    assert!(disable.message.contains("Name             demo"));
    assert!(disable.message.contains("Status           disabled"));

    let list = handle_plugins_slash_command(Some("list"), None, &mut manager)
        .expect("list command should succeed");
    assert!(list.message.contains("demo"));
    assert!(list.message.contains("disabled"));

    let enable = handle_plugins_slash_command(Some("enable"), Some("demo"), &mut manager)
        .expect("enable command should succeed");
    assert!(enable.reload_runtime);
    assert!(enable.message.contains("enabled demo@external"));
    assert!(enable.message.contains("Name             demo"));
    assert!(enable.message.contains("Status           enabled"));

    let list = handle_plugins_slash_command(Some("list"), None, &mut manager)
        .expect("list command should succeed");
    assert!(list.message.contains("demo"));
    assert!(list.message.contains("enabled"));

    let _ = fs::remove_dir_all(config_home);
    let _ = fs::remove_dir_all(source_root);
}

#[test]
fn lists_auto_installed_bundled_plugins_with_status() {
    let config_home = temp_dir("bundled-home");
    let bundled_root = temp_dir("bundled-root");
    let bundled_plugin = bundled_root.join("starter");
    write_bundled_plugin(&bundled_plugin, "starter", "0.1.0", false);

    let mut config = PluginManagerConfig::new(&config_home);
    config.bundled_root = Some(bundled_root.clone());
    let mut manager = PluginManager::new(config);

    let list = handle_plugins_slash_command(Some("list"), None, &mut manager)
        .expect("list command should succeed");
    assert!(!list.reload_runtime);
    assert!(list.message.contains("starter"));
    assert!(list.message.contains("v0.1.0"));
    assert!(list.message.contains("disabled"));

    let _ = fs::remove_dir_all(config_home);
    let _ = fs::remove_dir_all(bundled_root);
}
