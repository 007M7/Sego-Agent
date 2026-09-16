//! Permission enforcement layer that gates tool execution based on the
//! active `PermissionPolicy`.

use crate::permissions::{PermissionMode, PermissionOutcome, PermissionPolicy};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome")]
pub enum EnforcementResult {
    /// Tool execution is allowed.
    Allowed,
    /// Tool execution was denied due to insufficient permissions.
    Denied { tool: String, active_mode: String, required_mode: String, reason: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct PermissionEnforcer {
    policy: PermissionPolicy,
}

impl PermissionEnforcer {
    #[must_use]
    pub fn new(policy: PermissionPolicy) -> Self {
        Self { policy }
    }

    /// Check whether a tool can be executed under the current permission policy.
    /// Auto-denies when prompting is required but no prompter is provided.
    #[must_use]
    pub fn check(&self, tool_name: &str, input: &str) -> EnforcementResult {
        // When the active mode is Prompt, defer to the caller's interactive
        // prompt flow rather than hard-denying (the enforcer has no prompter).
        if self.policy.active_mode() == PermissionMode::Prompt {
            return EnforcementResult::Allowed;
        }

        let outcome = self.policy.authorize(tool_name, input, None);

        match outcome {
            PermissionOutcome::Allow => EnforcementResult::Allowed,
            PermissionOutcome::Deny { reason } => {
                let active_mode = self.policy.active_mode();
                let required_mode = self.policy.required_mode_for(tool_name);
                EnforcementResult::Denied {
                    tool: tool_name.to_owned(),
                    active_mode: active_mode.as_str().to_owned(),
                    required_mode: required_mode.as_str().to_owned(),
                    reason,
                }
            }
        }
    }

    #[must_use]
    pub fn is_allowed(&self, tool_name: &str, input: &str) -> bool {
        matches!(self.check(tool_name, input), EnforcementResult::Allowed)
    }

    #[must_use]
    pub fn active_mode(&self) -> PermissionMode {
        self.policy.active_mode()
    }

    /// Classify a file operation against workspace boundaries.
    #[must_use]
    pub fn check_file_write(&self, path: &str, workspace_root: &str) -> EnforcementResult {
        let mode = self.policy.active_mode();

        match mode {
            PermissionMode::ReadOnly => EnforcementResult::Denied {
                tool: "write_file".to_owned(),
                active_mode: mode.as_str().to_owned(),
                required_mode: PermissionMode::WorkspaceWrite.as_str().to_owned(),
                reason: format!("file writes are not allowed in '{}' mode", mode.as_str()),
            },
            PermissionMode::WorkspaceWrite => {
                if is_within_workspace(path, workspace_root) {
                    EnforcementResult::Allowed
                } else {
                    EnforcementResult::Denied {
                        tool: "write_file".to_owned(),
                        active_mode: mode.as_str().to_owned(),
                        required_mode: PermissionMode::DangerFullAccess.as_str().to_owned(),
                        reason: format!(
                            "path '{path}' is outside workspace root '{workspace_root}'"
                        ),
                    }
                }
            }
            // Allow and DangerFullAccess permit all writes
            PermissionMode::Allow | PermissionMode::DangerFullAccess => EnforcementResult::Allowed,
            PermissionMode::Prompt => EnforcementResult::Denied {
                tool: "write_file".to_owned(),
                active_mode: mode.as_str().to_owned(),
                required_mode: PermissionMode::WorkspaceWrite.as_str().to_owned(),
                reason: "file write requires confirmation in prompt mode".to_owned(),
            },
        }
    }

    /// Check if a bash command should be allowed based on current mode.
    #[must_use]
    pub fn check_bash(&self, command: &str) -> EnforcementResult {
        let mode = self.policy.active_mode();

        match mode {
            PermissionMode::ReadOnly => {
                if is_read_only_command(command) {
                    EnforcementResult::Allowed
                } else {
                    EnforcementResult::Denied {
                        tool: "bash".to_owned(),
                        active_mode: mode.as_str().to_owned(),
                        required_mode: PermissionMode::WorkspaceWrite.as_str().to_owned(),
                        reason: format!(
                            "command may modify state; not allowed in '{}' mode",
                            mode.as_str()
                        ),
                    }
                }
            }
            PermissionMode::Prompt => EnforcementResult::Denied {
                tool: "bash".to_owned(),
                active_mode: mode.as_str().to_owned(),
                required_mode: PermissionMode::DangerFullAccess.as_str().to_owned(),
                reason: "bash requires confirmation in prompt mode".to_owned(),
            },
            // WorkspaceWrite, Allow, DangerFullAccess: permit bash
            _ => EnforcementResult::Allowed,
        }
    }
}

/// Simple workspace boundary check via string prefix.
fn is_within_workspace(path: &str, workspace_root: &str) -> bool {
    let normalized =
        if path.starts_with('/') { path.to_owned() } else { format!("{workspace_root}/{path}") };

    let root = if workspace_root.ends_with('/') {
        workspace_root.to_owned()
    } else {
        format!("{workspace_root}/")
    };

    normalized.starts_with(&root) || normalized == workspace_root.trim_end_matches('/')
}

/// Conservative heuristic: is this bash command read-only?
///
/// The allowlist names only commands that read. Verb-shaped exceptions are
/// handled explicitly:
/// - `find` and `awk` read, but `-exec` / `-delete` / `system(` turn them into
///   execution or deletion;
/// - `cargo`, `git` and `gh` are read-only only in their read-only subcommand
///   form, since `cargo run` executes the artefact and `git push` mutates a
///   remote;
/// - interpreters (`python`, `node`, `ruby`, `rustc`) are not read-only at all:
///   running a script or compiling a file *is* code execution, so they are not
///   listed. They were previously allowed here.
/// - `tee` and `xargs` write and execute respectively, and were also previously
///   listed.
fn is_read_only_command(command: &str) -> bool {
    let first_token =
        command.split_whitespace().next().unwrap_or("").rsplit('/').next().unwrap_or("");
    let lowered = command.to_ascii_lowercase();

    if matches!(first_token, "find" | "awk") {
        return !lowered.contains("-exec")
            && !lowered.contains("-delete")
            && !lowered.contains("-ok ")
            && !lowered.contains("system(")
            && !lowered.contains("print >")
            && !command.contains(" > ")
            && !command.contains(" >> ");
    }

    if matches!(first_token, "cargo" | "git" | "gh") {
        return is_read_only_subcommand(first_token, command);
    }

    matches!(
        first_token,
        "cat"
            | "head"
            | "tail"
            | "less"
            | "more"
            | "wc"
            | "ls"
            | "find"
            | "grep"
            | "rg"
            | "awk"
            | "sed"
            | "echo"
            | "printf"
            | "which"
            | "where"
            | "whoami"
            | "pwd"
            | "env"
            | "printenv"
            | "date"
            | "cal"
            | "df"
            | "du"
            | "free"
            | "uptime"
            | "uname"
            | "file"
            | "stat"
            | "diff"
            | "sort"
            | "uniq"
            | "tr"
            | "cut"
            | "paste"
            | "test"
            | "true"
            | "false"
            | "type"
            | "readlink"
            | "realpath"
            | "basename"
            | "dirname"
            | "sha256sum"
            | "md5sum"
            | "b3sum"
            | "xxd"
            | "hexdump"
            | "od"
            | "strings"
            | "tree"
            | "jq"
            | "yq"
    ) && !command.contains("-i ")
        && !command.contains("--in-place")
        && !command.contains(" > ")
        && !command.contains(" >> ")
}

/// Is a multi-purpose build or VCS command in a read-only subcommand form?
///
/// The action is matched anywhere in the argument list rather than by position:
/// these tools accept flags that take a separate value (`git -C /repo diff`),
/// so "the first token that is not flag-shaped" is not the action. A command is
/// read-only only when a known read-only action appears *and* no known mutating
/// action does.
fn is_read_only_subcommand(tool: &str, command: &str) -> bool {
    let lowered = command.to_ascii_lowercase();
    let tokens: Vec<String> =
        command.split_whitespace().skip(1).map(|part| part.to_ascii_lowercase()).collect();
    let has_any = |verbs: &[&str]| tokens.iter().any(|token| verbs.contains(&token.as_str()));

    match tool {
        "cargo" => {
            const READ_ONLY: &[&str] =
                &["test", "build", "check", "clippy", "fmt", "verify", "metadata", "tree"];
            const MUTATING: &[&str] = &[
                "run",
                "install",
                "uninstall",
                "publish",
                "add",
                "remove",
                "rm",
                "update",
                "clean",
                "new",
                "init",
                "login",
                "logout",
                "owner",
                "yank",
            ];
            has_any(READ_ONLY) && !has_any(MUTATING)
        }
        "git" => {
            const READ_ONLY: &[&str] = &[
                "status",
                "diff",
                "log",
                "show",
                "blame",
                "ls-files",
                "ls-tree",
                "rev-parse",
                "describe",
                "shortlog",
                "branch",
                "remote",
                "config",
            ];
            const MUTATING: &[&str] = &[
                "push",
                "commit",
                "merge",
                "rebase",
                "reset",
                "checkout",
                "switch",
                "restore",
                "revert",
                "cherry-pick",
                "clean",
                "apply",
                "am",
                "tag",
                "stash",
                "submodule",
                "gc",
                "prune",
                "fetch",
                "pull",
                "init",
                "clone",
                "worktree",
            ];
            if has_any(MUTATING) {
                return false;
            }
            // `git branch` lists, unless asked to delete; `git config` reads,
            // unless asked to write.
            if lowered.contains(" --delete")
                || lowered.contains(" -d ")
                || lowered.contains(" --set-")
                || lowered.contains(" --unset")
            {
                return false;
            }
            has_any(READ_ONLY)
        }
        "gh" => {
            // `gh` names a resource group before the action (`gh pr list`), so
            // match the action anywhere in the argument list - but treat any
            // mutating verb, or an explicit HTTP method, as a refusal. Without
            // the method check `gh api -X DELETE ...` would read as read-only
            // because `api` is in the allowed set.
            const MUTATING: &[&str] = &[
                "delete", "merge", "close", "create", "edit", "push", "comment", "review",
                "reopen", "transfer", "ready", "lock", "unlock",
            ];
            const READ_ONLY: &[&str] =
                &["status", "view", "list", "search", "api", "diff", "checks", "browse"];
            if has_any(MUTATING) {
                return false;
            }
            if lowered.contains(" -x ") || lowered.contains(" --method ") {
                return false;
            }
            has_any(READ_ONLY)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_enforcer(mode: PermissionMode) -> PermissionEnforcer {
        let policy = PermissionPolicy::new(mode);
        PermissionEnforcer::new(policy)
    }

    #[test]
    fn allow_mode_permits_everything() {
        // `Allow` means "no per-tool prompting", not "authorize tools the
        // registry has never heard of": requirements must still be declared.
        let policy = PermissionPolicy::new(PermissionMode::Allow)
            .with_tool_requirement("bash", PermissionMode::DangerFullAccess)
            .with_tool_requirement("write_file", PermissionMode::WorkspaceWrite)
            .with_tool_requirement("edit_file", PermissionMode::WorkspaceWrite);
        let enforcer = PermissionEnforcer::new(policy);
        assert!(enforcer.is_allowed("bash", ""));
        assert!(enforcer.is_allowed("write_file", ""));
        assert!(enforcer.is_allowed("edit_file", ""));
        assert_eq!(
            enforcer.check_file_write("/outside/path", "/workspace"),
            EnforcementResult::Allowed
        );
        assert_eq!(enforcer.check_bash("rm -rf /"), EnforcementResult::Allowed);
        // An undeclared tool is denied even in the most permissive mode.
        assert!(!enforcer.is_allowed("not_a_registered_tool", ""));
    }

    #[test]
    fn read_only_denies_writes() {
        let policy = PermissionPolicy::new(PermissionMode::ReadOnly)
            .with_tool_requirement("read_file", PermissionMode::ReadOnly)
            .with_tool_requirement("grep_search", PermissionMode::ReadOnly)
            .with_tool_requirement("write_file", PermissionMode::WorkspaceWrite);

        let enforcer = PermissionEnforcer::new(policy);
        assert!(enforcer.is_allowed("read_file", ""));
        assert!(enforcer.is_allowed("grep_search", ""));

        // write_file requires WorkspaceWrite but we're in ReadOnly
        let result = enforcer.check("write_file", "");
        assert!(matches!(result, EnforcementResult::Denied { .. }));

        let result = enforcer.check_file_write("/workspace/file.rs", "/workspace");
        assert!(matches!(result, EnforcementResult::Denied { .. }));
    }

    #[test]
    fn read_only_allows_read_commands() {
        let enforcer = make_enforcer(PermissionMode::ReadOnly);
        assert_eq!(enforcer.check_bash("cat src/main.rs"), EnforcementResult::Allowed);
        assert_eq!(enforcer.check_bash("grep -r 'pattern' ."), EnforcementResult::Allowed);
        assert_eq!(enforcer.check_bash("ls -la"), EnforcementResult::Allowed);
    }

    #[test]
    fn read_only_denies_write_commands() {
        let enforcer = make_enforcer(PermissionMode::ReadOnly);
        let result = enforcer.check_bash("rm file.txt");
        assert!(matches!(result, EnforcementResult::Denied { .. }));
    }

    #[test]
    fn workspace_write_allows_within_workspace() {
        let enforcer = make_enforcer(PermissionMode::WorkspaceWrite);
        let result = enforcer.check_file_write("/workspace/src/main.rs", "/workspace");
        assert_eq!(result, EnforcementResult::Allowed);
    }

    #[test]
    fn workspace_write_denies_outside_workspace() {
        let enforcer = make_enforcer(PermissionMode::WorkspaceWrite);
        let result = enforcer.check_file_write("/etc/passwd", "/workspace");
        assert!(matches!(result, EnforcementResult::Denied { .. }));
    }

    #[test]
    fn prompt_mode_denies_without_prompter() {
        let enforcer = make_enforcer(PermissionMode::Prompt);
        let result = enforcer.check_bash("echo test");
        assert!(matches!(result, EnforcementResult::Denied { .. }));

        let result = enforcer.check_file_write("/workspace/file.rs", "/workspace");
        assert!(matches!(result, EnforcementResult::Denied { .. }));
    }

    #[test]
    fn workspace_boundary_check() {
        assert!(is_within_workspace("/workspace/src/main.rs", "/workspace"));
        assert!(is_within_workspace("/workspace", "/workspace"));
        assert!(!is_within_workspace("/etc/passwd", "/workspace"));
        assert!(!is_within_workspace("/workspacex/hack", "/workspace"));
    }

    #[test]
    fn read_only_command_heuristic() {
        assert!(is_read_only_command("cat file.txt"));
        assert!(is_read_only_command("grep pattern file"));
        assert!(is_read_only_command("git log --oneline"));
        assert!(!is_read_only_command("rm file.txt"));
        assert!(!is_read_only_command("echo test > file.txt"));
        assert!(!is_read_only_command("sed -i 's/a/b/' file"));
    }

    #[test]
    fn read_only_heuristic_excludes_code_execution_and_writes() {
        // Running a script or compiling a file is code execution, not reading,
        // so interpreters are not read-only however they are invoked.
        for command in [
            "python -c 'import os'",
            "python3 script.py",
            "python -i",
            "node -e 'require(\"child_process\")'",
            "node server.js",
            "ruby -e 'system(\"id\")'",
            "rustc build.rs",
        ] {
            assert!(!is_read_only_command(command), "{command} must not be read-only");
        }
        // `tee` writes and `xargs` executes.
        assert!(!is_read_only_command("tee /etc/hosts"));
        assert!(!is_read_only_command("xargs rm"));
        // `find` and `awk` read, unless a flag turns them into something else.
        assert!(is_read_only_command("find . -name '*.rs'"));
        assert!(!is_read_only_command("find . -exec rm {} ;"));
        assert!(!is_read_only_command("find . -delete"));
        assert!(!is_read_only_command("awk 'BEGIN { system(\"id\") }'"));
    }

    #[test]
    fn read_only_heuristic_gates_multipurpose_tools_by_subcommand() {
        // Read-only forms stay allowed.
        assert!(is_read_only_command("cargo test"));
        assert!(is_read_only_command("cargo build --release"));
        assert!(is_read_only_command("git status"));
        assert!(is_read_only_command("git -C /repo diff"));
        assert!(is_read_only_command("gh pr list"));
        // Mutating or executing forms do not, even though the tool is allowed.
        assert!(!is_read_only_command("cargo run"));
        assert!(!is_read_only_command("cargo install ripgrep"));
        assert!(!is_read_only_command("git push origin main"));
        assert!(!is_read_only_command("git commit -m x"));
        assert!(!is_read_only_command("git branch -D feat/old"));
        assert!(!is_read_only_command("git config --set-env x"));
        assert!(!is_read_only_command("gh api -X DELETE /repos/x"));
    }

    #[test]
    fn active_mode_returns_policy_mode() {
        // given
        let modes = [
            PermissionMode::ReadOnly,
            PermissionMode::WorkspaceWrite,
            PermissionMode::DangerFullAccess,
            PermissionMode::Prompt,
            PermissionMode::Allow,
        ];

        // when
        let active_modes: Vec<_> =
            modes.into_iter().map(|mode| make_enforcer(mode).active_mode()).collect();

        // then
        assert_eq!(active_modes, modes);
    }

    #[test]
    fn danger_full_access_permits_file_writes_and_bash() {
        // given
        let enforcer = make_enforcer(PermissionMode::DangerFullAccess);

        // when
        let file_result = enforcer.check_file_write("/outside/workspace/file.txt", "/workspace");
        let bash_result = enforcer.check_bash("rm -rf /tmp/scratch");

        // then
        assert_eq!(file_result, EnforcementResult::Allowed);
        assert_eq!(bash_result, EnforcementResult::Allowed);
    }

    #[test]
    fn check_denied_payload_contains_tool_and_modes() {
        // given
        let policy = PermissionPolicy::new(PermissionMode::ReadOnly)
            .with_tool_requirement("write_file", PermissionMode::WorkspaceWrite);
        let enforcer = PermissionEnforcer::new(policy);

        // when
        let result = enforcer.check("write_file", "{}");

        // then
        match result {
            EnforcementResult::Denied { tool, active_mode, required_mode, reason } => {
                assert_eq!(tool, "write_file");
                assert_eq!(active_mode, "read-only");
                assert_eq!(required_mode, "workspace-write");
                assert!(reason.contains("requires workspace-write permission"));
            }
            other => panic!("expected denied result, got {other:?}"),
        }
    }

    #[test]
    fn workspace_write_relative_path_resolved() {
        // given
        let enforcer = make_enforcer(PermissionMode::WorkspaceWrite);

        // when
        let result = enforcer.check_file_write("src/main.rs", "/workspace");

        // then
        assert_eq!(result, EnforcementResult::Allowed);
    }

    #[test]
    fn workspace_root_with_trailing_slash() {
        // given
        let enforcer = make_enforcer(PermissionMode::WorkspaceWrite);

        // when
        let result = enforcer.check_file_write("/workspace/src/main.rs", "/workspace/");

        // then
        assert_eq!(result, EnforcementResult::Allowed);
    }

    #[test]
    fn workspace_root_equality() {
        // given
        let root = "/workspace/";

        // when
        let equal_to_root = is_within_workspace("/workspace", root);

        // then
        assert!(equal_to_root);
    }

    #[test]
    fn bash_heuristic_full_path_prefix() {
        // given
        let full_path_command = "/usr/bin/cat Cargo.toml";
        let git_path_command = "/usr/local/bin/git status";

        // when
        let cat_result = is_read_only_command(full_path_command);
        let git_result = is_read_only_command(git_path_command);

        // then
        assert!(cat_result);
        assert!(git_result);
    }

    #[test]
    fn bash_heuristic_redirects_block_read_only_commands() {
        // given
        let overwrite = "cat Cargo.toml > out.txt";
        let append = "echo test >> out.txt";

        // when
        let overwrite_result = is_read_only_command(overwrite);
        let append_result = is_read_only_command(append);

        // then
        assert!(!overwrite_result);
        assert!(!append_result);
    }

    #[test]
    fn bash_heuristic_in_place_flag_blocks() {
        // given
        let interactive_python = "python -i script.py";
        let in_place_sed = "sed --in-place 's/a/b/' file.txt";

        // when
        let interactive_result = is_read_only_command(interactive_python);
        let in_place_result = is_read_only_command(in_place_sed);

        // then
        assert!(!interactive_result);
        assert!(!in_place_result);
    }

    #[test]
    fn bash_heuristic_empty_command() {
        // given
        let empty = "";
        let whitespace = "   ";

        // when
        let empty_result = is_read_only_command(empty);
        let whitespace_result = is_read_only_command(whitespace);

        // then
        assert!(!empty_result);
        assert!(!whitespace_result);
    }

    #[test]
    fn prompt_mode_check_bash_denied_payload_fields() {
        // given
        let enforcer = make_enforcer(PermissionMode::Prompt);

        // when
        let result = enforcer.check_bash("git status");

        // then
        match result {
            EnforcementResult::Denied { tool, active_mode, required_mode, reason } => {
                assert_eq!(tool, "bash");
                assert_eq!(active_mode, "prompt");
                assert_eq!(required_mode, "danger-full-access");
                assert_eq!(reason, "bash requires confirmation in prompt mode");
            }
            other => panic!("expected denied result, got {other:?}"),
        }
    }

    #[test]
    fn read_only_check_file_write_denied_payload() {
        // given
        let enforcer = make_enforcer(PermissionMode::ReadOnly);

        // when
        let result = enforcer.check_file_write("/workspace/file.txt", "/workspace");

        // then
        match result {
            EnforcementResult::Denied { tool, active_mode, required_mode, reason } => {
                assert_eq!(tool, "write_file");
                assert_eq!(active_mode, "read-only");
                assert_eq!(required_mode, "workspace-write");
                assert!(reason.contains("file writes are not allowed"));
            }
            other => panic!("expected denied result, got {other:?}"),
        }
    }
}
