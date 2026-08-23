use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::{command_runner::plugin_command, PluginError, PluginHooks, PluginRegistry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
}

impl HookEvent {
    fn as_str(self) -> &'static str {
        match self {
            Self::PreToolUse => "PreToolUse",
            Self::PostToolUse => "PostToolUse",
            Self::PostToolUseFailure => "PostToolUseFailure",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRunResult {
    denied: bool,
    failed: bool,
    messages: Vec<String>,
}

impl HookRunResult {
    #[must_use]
    pub fn allow(messages: Vec<String>) -> Self {
        Self { denied: false, failed: false, messages }
    }

    #[must_use]
    pub fn is_denied(&self) -> bool {
        self.denied
    }

    #[must_use]
    pub fn is_failed(&self) -> bool {
        self.failed
    }

    #[must_use]
    pub fn messages(&self) -> &[String] {
        &self.messages
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HookCommand {
    command: String,
    working_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct HookPlan {
    pre_tool_use: Vec<HookCommand>,
    post_tool_use: Vec<HookCommand>,
    post_tool_use_failure: Vec<HookCommand>,
}

impl HookPlan {
    fn without_working_dir(hooks: PluginHooks) -> Self {
        let commands = |entries: Vec<String>| {
            entries.into_iter().map(|command| HookCommand { command, working_dir: None }).collect()
        };
        Self {
            pre_tool_use: commands(hooks.pre_tool_use),
            post_tool_use: commands(hooks.post_tool_use),
            post_tool_use_failure: commands(hooks.post_tool_use_failure),
        }
    }

    fn extend(&mut self, hooks: &PluginHooks, working_dir: Option<&Path>) {
        let append = |target: &mut Vec<HookCommand>, commands: &[String]| {
            target.extend(commands.iter().map(|command| HookCommand {
                command: command.clone(),
                working_dir: working_dir.map(Path::to_path_buf),
            }));
        };

        append(&mut self.pre_tool_use, &hooks.pre_tool_use);
        append(&mut self.post_tool_use, &hooks.post_tool_use);
        append(&mut self.post_tool_use_failure, &hooks.post_tool_use_failure);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HookRunner {
    plan: HookPlan,
}

impl HookRunner {
    #[must_use]
    pub fn new(hooks: PluginHooks) -> Self {
        Self { plan: HookPlan::without_working_dir(hooks) }
    }

    pub fn from_registry(plugin_registry: &PluginRegistry) -> Result<Self, PluginError> {
        let mut plan = HookPlan::default();
        for plugin in plugin_registry.plugins().iter().filter(|plugin| plugin.is_enabled()) {
            plugin.validate()?;
            plan.extend(plugin.hooks(), plugin.metadata().root.as_deref());
        }
        Ok(Self { plan })
    }

    #[must_use]
    pub fn run_pre_tool_use(&self, tool_name: &str, tool_input: &str) -> HookRunResult {
        Self::run_commands(
            HookEvent::PreToolUse,
            &self.plan.pre_tool_use,
            tool_name,
            tool_input,
            None,
            false,
        )
    }

    #[must_use]
    pub fn run_post_tool_use(
        &self,
        tool_name: &str,
        tool_input: &str,
        tool_output: &str,
        is_error: bool,
    ) -> HookRunResult {
        Self::run_commands(
            HookEvent::PostToolUse,
            &self.plan.post_tool_use,
            tool_name,
            tool_input,
            Some(tool_output),
            is_error,
        )
    }

    #[must_use]
    pub fn run_post_tool_use_failure(
        &self,
        tool_name: &str,
        tool_input: &str,
        tool_error: &str,
    ) -> HookRunResult {
        Self::run_commands(
            HookEvent::PostToolUseFailure,
            &self.plan.post_tool_use_failure,
            tool_name,
            tool_input,
            Some(tool_error),
            true,
        )
    }

    fn run_commands(
        event: HookEvent,
        commands: &[HookCommand],
        tool_name: &str,
        tool_input: &str,
        tool_output: Option<&str>,
        is_error: bool,
    ) -> HookRunResult {
        if commands.is_empty() {
            return HookRunResult::allow(Vec::new());
        }

        let payload = hook_payload(event, tool_name, tool_input, tool_output, is_error).to_string();

        let mut messages = Vec::new();

        for command in commands {
            match Self::run_command(
                command,
                event,
                tool_name,
                tool_input,
                tool_output,
                is_error,
                &payload,
            ) {
                HookCommandOutcome::Allow { message } => {
                    if let Some(message) = message {
                        messages.push(message);
                    }
                }
                HookCommandOutcome::Deny { message } => {
                    messages.push(message.unwrap_or_else(|| {
                        format!("{} hook denied tool `{tool_name}`", event.as_str())
                    }));
                    return HookRunResult { denied: true, failed: false, messages };
                }
                HookCommandOutcome::Failed { message } => {
                    messages.push(message);
                    return HookRunResult { denied: false, failed: true, messages };
                }
            }
        }

        HookRunResult::allow(messages)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_command(
        hook_command: &HookCommand,
        event: HookEvent,
        tool_name: &str,
        tool_input: &str,
        tool_output: Option<&str>,
        is_error: bool,
        payload: &str,
    ) -> HookCommandOutcome {
        let command = hook_command.command.as_str();
        let mut process = match plugin_command(command) {
            Ok(process) => process,
            Err(error) => {
                return HookCommandOutcome::Failed {
                    message: format!(
                        "{} hook `{command}` could not be prepared for `{tool_name}`: {error}",
                        event.as_str()
                    ),
                };
            }
        };
        if let Some(working_dir) = &hook_command.working_dir {
            process.current_dir(working_dir).env("CLAWD_PLUGIN_ROOT", working_dir.as_os_str());
        }

        let mut child = CommandWithStdin::new(process);
        child.stdin(std::process::Stdio::piped());
        child.stdout(std::process::Stdio::piped());
        child.stderr(std::process::Stdio::piped());
        child.env("HOOK_EVENT", event.as_str());
        child.env("HOOK_TOOL_NAME", tool_name);
        child.env("HOOK_TOOL_INPUT", tool_input);
        child.env("HOOK_TOOL_IS_ERROR", if is_error { "1" } else { "0" });
        if let Some(tool_output) = tool_output {
            child.env("HOOK_TOOL_OUTPUT", tool_output);
        }

        match child.output_with_stdin(payload.as_bytes()) {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let message = (!stdout.is_empty()).then_some(stdout);
                match output.status.code() {
                    Some(0) => HookCommandOutcome::Allow { message },
                    Some(2) => HookCommandOutcome::Deny { message },
                    Some(code) => HookCommandOutcome::Failed {
                        message: format_hook_warning(
                            command,
                            code,
                            message.as_deref(),
                            stderr.as_str(),
                        ),
                    },
                    None => HookCommandOutcome::Failed {
                        message: format!(
                            "{} hook `{command}` terminated by signal while handling `{tool_name}`",
                            event.as_str()
                        ),
                    },
                }
            }
            Err(error) => HookCommandOutcome::Failed {
                message: format!(
                    "{} hook `{command}` failed to start for `{tool_name}`: {error}",
                    event.as_str()
                ),
            },
        }
    }
}

enum HookCommandOutcome {
    Allow { message: Option<String> },
    Deny { message: Option<String> },
    Failed { message: String },
}

fn hook_payload(
    event: HookEvent,
    tool_name: &str,
    tool_input: &str,
    tool_output: Option<&str>,
    is_error: bool,
) -> serde_json::Value {
    match event {
        HookEvent::PostToolUseFailure => json!({
            "hook_event_name": event.as_str(),
            "tool_name": tool_name,
            "tool_input": parse_tool_input(tool_input),
            "tool_input_json": tool_input,
            "tool_error": tool_output,
            "tool_result_is_error": true,
        }),
        _ => json!({
            "hook_event_name": event.as_str(),
            "tool_name": tool_name,
            "tool_input": parse_tool_input(tool_input),
            "tool_input_json": tool_input,
            "tool_output": tool_output,
            "tool_result_is_error": is_error,
        }),
    }
}

fn parse_tool_input(tool_input: &str) -> serde_json::Value {
    serde_json::from_str(tool_input).unwrap_or_else(|_| json!({ "raw": tool_input }))
}

fn format_hook_warning(command: &str, code: i32, stdout: Option<&str>, stderr: &str) -> String {
    let mut message = format!("Hook `{command}` exited with status {code}");
    if let Some(stdout) = stdout.filter(|stdout| !stdout.is_empty()) {
        message.push_str(": ");
        message.push_str(stdout);
    } else if !stderr.is_empty() {
        message.push_str(": ");
        message.push_str(stderr);
    }
    message
}

struct CommandWithStdin {
    command: std::process::Command,
}

impl CommandWithStdin {
    fn new(command: std::process::Command) -> Self {
        Self { command }
    }

    fn stdin(&mut self, cfg: std::process::Stdio) -> &mut Self {
        self.command.stdin(cfg);
        self
    }

    fn stdout(&mut self, cfg: std::process::Stdio) -> &mut Self {
        self.command.stdout(cfg);
        self
    }

    fn stderr(&mut self, cfg: std::process::Stdio) -> &mut Self {
        self.command.stderr(cfg);
        self
    }

    fn env<K, V>(&mut self, key: K, value: V) -> &mut Self
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        self.command.env(key, value);
        self
    }

    fn output_with_stdin(&mut self, stdin: &[u8]) -> std::io::Result<std::process::Output> {
        let mut child = self.command.spawn()?;
        if let Some(mut child_stdin) = child.stdin.take() {
            use std::io::Write as _;
            if let Err(error) = child_stdin.write_all(stdin) {
                if error.kind() != std::io::ErrorKind::BrokenPipe {
                    return Err(error);
                }
            }
        }
        child.wait_with_output()
    }
}

#[cfg(test)]
mod tests {
    use super::{HookRunResult, HookRunner};
    use crate::{PluginManager, PluginManagerConfig};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("plugins-hook-runner-{label}-{nanos}"))
    }

    fn write_hook_plugin(
        root: &Path,
        name: &str,
        pre_message: &str,
        post_message: &str,
        failure_message: &str,
    ) {
        fs::create_dir_all(root.join(".claude-plugin")).expect("manifest dir");
        fs::create_dir_all(root.join("hooks")).expect("hooks dir");
        let (pre_script, pre_body) = hook_script("pre", pre_message);
        let (post_script, post_body) = hook_script("post", post_message);
        let (failure_script, failure_body) = hook_script("failure", failure_message);
        fs::write(
            root.join("hooks").join(Path::new(&pre_script).file_name().expect("file name")),
            pre_body,
        )
        .expect("write pre hook");
        fs::write(
            root.join("hooks").join(Path::new(&post_script).file_name().expect("file name")),
            post_body,
        )
        .expect("write post hook");
        fs::write(
            root.join("hooks").join(Path::new(&failure_script).file_name().expect("file name")),
            failure_body,
        )
        .expect("write failure hook");
        fs::write(
            root.join(".claude-plugin").join("plugin.json"),
            format!(
                "{{\n  \"name\": \"{name}\",\n  \"version\": \"1.0.0\",\n  \"description\": \"hook plugin\",\n  \"hooks\": {{\n    \"PreToolUse\": [\"{pre_script}\"],\n    \"PostToolUse\": [\"{post_script}\"],\n    \"PostToolUseFailure\": [\"{failure_script}\"]\n  }}\n}}"
            ),
        )
        .expect("write plugin manifest");
    }

    fn write_root_probe_hook_plugin(root: &Path, name: &str) {
        fs::create_dir_all(root.join(".claude-plugin")).expect("manifest dir");
        fs::create_dir_all(root.join("hooks")).expect("hooks dir");
        let (script_command, script_body) = root_probe_script();
        fs::write(root.join(script_command.trim_start_matches("./")), script_body)
            .expect("write root probe hook");
        fs::write(
            root.join(".claude-plugin").join("plugin.json"),
            format!(
                "{{\n  \"name\": \"{name}\",\n  \"version\": \"1.0.0\",\n  \"description\": \"root probe hook plugin\",\n  \"hooks\": {{\n    \"PreToolUse\": [\"{script_command}\"]\n  }}\n}}"
            ),
        )
        .expect("write root probe manifest");
    }

    #[cfg(windows)]
    fn root_probe_script() -> (String, String) {
        (
            "./hooks/root-probe.cmd".to_string(),
            "@echo off\r\n>> hook-root.log echo rooted\r\necho root ok\r\n".to_string(),
        )
    }

    #[cfg(not(windows))]
    fn root_probe_script() -> (String, String) {
        (
            "./hooks/root-probe.sh".to_string(),
            "#!/bin/sh\nprintf 'rooted\\n' >> hook-root.log\nprintf 'root ok\\n'\n".to_string(),
        )
    }

    #[cfg(windows)]
    fn hook_script(name: &str, message: &str) -> (String, String) {
        (format!("./hooks/{name}.cmd"), format!("@echo off\r\necho {message}\r\n"))
    }

    #[cfg(not(windows))]
    fn hook_script(name: &str, message: &str) -> (String, String) {
        (format!("./hooks/{name}.sh"), format!("#!/bin/sh\nprintf '%s\\n' '{message}'\n"))
    }

    #[cfg(windows)]
    fn hook_command(message: &str, exit_code: i32) -> String {
        format!("<nul set /p={message} & exit /B {exit_code}")
    }

    #[cfg(not(windows))]
    fn hook_command(message: &str, exit_code: i32) -> String {
        format!("printf '{message}'; exit {exit_code}")
    }

    #[test]
    fn collects_and_runs_hooks_from_enabled_plugins() {
        // given
        let config_home = temp_dir("config");
        let first_source_root = temp_dir("source-a");
        let second_source_root = temp_dir("source-b");
        write_hook_plugin(
            &first_source_root,
            "first",
            "plugin pre one",
            "plugin post one",
            "plugin failure one",
        );
        write_hook_plugin(
            &second_source_root,
            "second",
            "plugin pre two",
            "plugin post two",
            "plugin failure two",
        );

        let mut manager = PluginManager::new(PluginManagerConfig::new(&config_home));
        manager
            .install(first_source_root.to_str().expect("utf8 path"))
            .expect("first plugin install should succeed");
        manager
            .install(second_source_root.to_str().expect("utf8 path"))
            .expect("second plugin install should succeed");
        let registry = manager.plugin_registry().expect("registry should build");

        // when
        let runner = HookRunner::from_registry(&registry).expect("plugin hooks should load");

        // then
        assert_eq!(
            runner.run_pre_tool_use("Read", r#"{"path":"README.md"}"#),
            HookRunResult::allow(vec!["plugin pre one".to_string(), "plugin pre two".to_string(),])
        );
        assert_eq!(
            runner.run_post_tool_use("Read", r#"{"path":"README.md"}"#, "ok", false),
            HookRunResult::allow(vec![
                "plugin post one".to_string(),
                "plugin post two".to_string(),
            ])
        );
        assert_eq!(
            runner.run_post_tool_use_failure("Read", r#"{"path":"README.md"}"#, "tool failed",),
            HookRunResult::allow(vec![
                "plugin failure one".to_string(),
                "plugin failure two".to_string(),
            ])
        );

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(first_source_root);
        let _ = fs::remove_dir_all(second_source_root);
    }

    #[test]
    fn registry_hooks_run_synchronously_in_their_plugin_root() {
        let config_home = temp_dir("root-probe-config");
        let source_root = temp_dir("root-probe-source");
        write_root_probe_hook_plugin(&source_root, "root-probe");

        let mut manager = PluginManager::new(PluginManagerConfig::new(&config_home));
        let install = manager
            .install(source_root.to_str().expect("utf8 path"))
            .expect("plugin install should succeed");
        let registry = manager.plugin_registry().expect("registry should build");
        let runner = HookRunner::from_registry(&registry).expect("plugin hooks should load");

        let result = runner.run_pre_tool_use("Read", r#"{"path":"README.md"}"#);

        assert_eq!(result, HookRunResult::allow(vec!["root ok".to_string()]));
        assert_eq!(
            fs::read_to_string(install.install_path.join("hook-root.log"))
                .expect("hook root log should exist")
                .replace("\r\n", "\n"),
            "rooted\n"
        );
        assert!(!source_root.join("hook-root.log").exists());

        let _ = fs::remove_dir_all(config_home);
        let _ = fs::remove_dir_all(source_root);
    }

    #[test]
    fn pre_tool_use_denies_when_plugin_hook_exits_two() {
        // given
        let runner = HookRunner::new(crate::PluginHooks {
            pre_tool_use: vec![hook_command("blocked by plugin", 2)],
            post_tool_use: Vec::new(),
            post_tool_use_failure: Vec::new(),
        });

        // when
        let result = runner.run_pre_tool_use("Bash", r#"{"command":"pwd"}"#);

        // then
        assert!(result.is_denied());
        assert_eq!(result.messages(), &["blocked by plugin".to_string()]);
    }

    #[test]
    fn propagates_plugin_hook_failures() {
        // given
        let runner = HookRunner::new(crate::PluginHooks {
            pre_tool_use: vec![
                hook_command("broken plugin hook", 1),
                hook_command("later plugin hook", 0),
            ],
            post_tool_use: Vec::new(),
            post_tool_use_failure: Vec::new(),
        });

        // when
        let result = runner.run_pre_tool_use("Bash", r#"{"command":"pwd"}"#);

        // then
        assert!(result.is_failed());
        assert!(result.messages().iter().any(|message| message.contains("broken plugin hook")));
        assert!(!result.messages().iter().any(|message| message == "later plugin hook"));
    }
}
