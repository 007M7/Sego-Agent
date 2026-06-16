//! Bash command risk classifier for the `review-trust` permission profile.
//!
//! When `review-trust` is active, the `bash` tool is still declared as
//! `DangerFullAccess` at the spec level (see Codex decision §2), but the
//! command string is classified here to decide whether the specific invocation
//! should be allowed without prompting, requires confirmation, or must be
//! denied outright.
//!
//! Classification is intentionally conservative: unknown commands fall back to
//! `UnknownAsk` so that the worst case is an extra confirmation, never a silent
//! security bypass.

/// Risk category assigned to a single bash command string under `review-trust`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BashCommandRisk {
    /// Read-only inspection commands: `cat`, `rg`, `grep`, `ls`, `find`, git read-only.
    /// Allowed without prompting.
    SafeReadonly,
    /// Build/test/lint verification commands: `cargo test`, `cargo build`, `npm test`, etc.
    /// Allowed (or covered by one-time verify approval).
    SafeVerify,
    /// Writes to Sego-owned metadata under `.sego/`.
    /// Allowed without prompting.
    SegaMetadataWrite,
    /// Source-tree mutations, dependency installs, out-of-workspace writes.
    /// Requires confirmation.
    AskMutation,
    /// Destructive or irreversible commands: `rm -rf`, `git reset --hard`, `git push`, `sudo`.
    /// Denied outright under `review-trust`.
    DenyDangerous,
    /// Anything the classifier cannot confidently categorise.
    /// Requires confirmation (fail-safe).
    UnknownAsk,
}

impl BashCommandRisk {
    /// Returns `true` when the category should be auto-allowed under `review-trust`.
    #[must_use]
    pub fn is_auto_allow(self) -> bool {
        matches!(self, Self::SafeReadonly | Self::SafeVerify | Self::SegaMetadataWrite)
    }
}

/// Classify a bash command string into a risk category.
///
/// The classifier inspects the leading command token(s) only; it does not run a
/// full shell parser. Complex pipelines (`cat foo | rm`) are treated as
/// `UnknownAsk` unless every stage is recognisably safe.
#[must_use]
pub fn classify_bash_command(command: &str) -> BashCommandRisk {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return BashCommandRisk::UnknownAsk;
    }

    // Writes under .sego/ metadata directory are Sego-owned. Check this BEFORE
    // unsafe-chain detection, because `.sego/` writes legitimately use `>`
    // redirection (e.g. `echo {} > .sego/reviews/idx.json`).
    if targets_sego_metadata(trimmed) {
        return BashCommandRisk::SegaMetadataWrite;
    }

    // Reject anything containing dangerous operators that could chain a safe
    // command into a destructive one (e.g. `cat foo; rm -rf /`).
    if contains_unsafe_chain(trimmed) {
        return BashCommandRisk::DenyDangerous;
    }

    // Destructive commands are denied first, even if they start with a safe verb.
    if is_dangerous(trimmed) {
        return BashCommandRisk::DenyDangerous;
    }

    // Read-only inspection commands.
    if is_safe_readonly(trimmed) {
        return BashCommandRisk::SafeReadonly;
    }

    // Build/test/lint verification commands.
    if is_safe_verify(trimmed) {
        return BashCommandRisk::SafeVerify;
    }

    // Source-tree mutations / installs / out-of-workspace writes.
    if is_likely_mutation(trimmed) {
        return BashCommandRisk::AskMutation;
    }

    BashCommandRisk::UnknownAsk
}

/// Classify a bash tool invocation given its raw tool input.
///
/// Real bash tool calls pass JSON like `{"command":"git status"}` rather than
/// a bare command string. This wrapper extracts the `command` field before
/// classification, and fails safe (`UnknownAsk`) when the input is not valid
/// JSON, lacks a `command` field, or `command` is not a string.
///
/// See Codex c10/b second review.
#[must_use]
pub fn classify_bash_tool_input(input: &str) -> BashCommandRisk {
    let trimmed = input.trim();
    // Fast path: if it doesn't look like JSON, classify as-is.
    if !trimmed.starts_with('{') {
        return classify_bash_command(trimmed);
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(value) => match value.get("command") {
            Some(serde_json::Value::String(command)) => classify_bash_command(command),
            _ => BashCommandRisk::UnknownAsk,
        },
        Err(_) => BashCommandRisk::UnknownAsk,
    }
}

/// Detect operators that can chain or redirect a safe command into a dangerous one.
fn contains_unsafe_chain(command: &str) -> bool {
    // `;`, `&&`, `||`, `|` (pipe), `>` (redirect), backticks, `$(` command substitution.
    // We treat any of these as potentially unsafe under review-trust because the
    // second stage of a pipeline is not inspected.
    // Note: `|` is checked independently of `||` so that plain pipes like
    // `echo harmless | dangerous-cmd` are caught (Sego c10/b review Medium finding).
    command.contains("&&")
        || command.contains("||")
        || command.contains('|')
        || command.contains(';')
        || command.contains('`')
        || command.contains("$(")
        || command.contains('>')
}

/// Heuristic: does the command write to Sego-owned `.sego/` metadata?
fn targets_sego_metadata(command: &str) -> bool {
    // Common patterns: redirection to .sego/, or commands whose target path
    // starts with .sego/. This is intentionally broad — writing under .sego/ is
    // always Sego metadata under review-trust.
    command.contains(".sego/")
        && (command.contains("write")
            || command.contains("mkdir")
            || command.contains("mv ")
            || command.contains("cp ")
            || command.contains("tee")
            || command.contains("cat >")
            || command.contains("echo")
            || command.contains("python")
            || command.contains("node"))
}

/// Commands that are unambiguously destructive or irreversible.
fn is_dangerous(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    // rm -rf, del /s, Remove-Item -Recurse
    if lower.starts_with("rm ")
        && (lower.contains("-rf") || lower.contains("-fr") || lower.contains("--recursive"))
    {
        return true;
    }
    if lower.starts_with("rm -rf ") {
        return true;
    }
    if lower.contains("rmdir /s")
        || lower.contains("del /s")
        || lower.contains("remove-item -recurse")
    {
        return true;
    }
    // Hard resets / clean
    if lower.starts_with("git reset --hard") || lower.starts_with("git clean") {
        return true;
    }
    // Remote mutations
    if lower.starts_with("git push") {
        return true;
    }
    // Privilege escalation / system mutation
    if lower.starts_with("sudo ") || lower.starts_with("chmod ") || lower.starts_with("chown ") {
        return true;
    }
    // Force-delete branches (`-D` lowercases to `-d`, so check both spellings).
    if lower.starts_with("git branch -d ") || lower.starts_with("git branch --delete --force") {
        return true;
    }
    // Format / mkfs
    if lower.starts_with("format ") || lower.starts_with("mkfs") {
        return true;
    }
    false
}

/// Read-only inspection commands that never mutate state.
fn is_safe_readonly(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let first = first_token(&lower);

    // File inspection
    matches!(
        first,
        "cat" | "head" | "tail" | "less" | "more" | "wc" | "file" | "stat" | "du" | "df"
    ) && !is_write_redirect(command)
    // Search
    || matches!(first, "rg" | "grep" | "ag" | "ack" | "findstr")
    // Listing / traversal
    || matches!(first, "ls" | "dir" | "tree" | "find" | "pwd" | "echo" | "printenv")
        && !is_write_redirect(command)
    // Git read-only
    || is_git_readonly(&lower)
    // gh read-only (view, list, status)
    || is_gh_readonly(&lower)
}

/// Build / test / lint commands emitted by verify workflows.
fn is_safe_verify(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let first = first_token(&lower);

    // Rust
    if first == "cargo" {
        return matches!(
            second_token(&lower).as_deref(),
            Some("test")
                | Some("build")
                | Some("check")
                | Some("clippy")
                | Some("fmt")
                | Some("verify")
        );
    }
    // Node
    if first == "npm" {
        let second = second_token(&lower);
        return matches!(second.as_deref(), Some("test") | Some("ci"))
            || lower.starts_with("npm run test")
            || lower.starts_with("npm run lint")
            || lower.starts_with("npm run build");
    }
    if matches!(first, "pnpm" | "yarn" | "pnpx") {
        return lower.contains("test") || lower.contains("lint") || lower.contains("build");
    }
    // Python
    if first == "python" || first == "python3" {
        return lower.contains("-m pytest")
            || lower.contains("-m unittest")
            || lower.contains("-m mypy");
    }
    if first == "pytest" || first == "mypy" || first == "ruff" || first == "black" {
        return true;
    }
    // Go
    if first == "go" {
        return matches!(
            second_token(&lower).as_deref(),
            Some("test") | Some("build") | Some("vet")
        );
    }
    // Generic lint/test wrappers
    matches!(first, "make")
        && (lower.contains("test") || lower.contains("lint") || lower.contains("check"))
}

/// Likely source-tree mutations, dependency installs, or out-of-workspace writes.
fn is_likely_mutation(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let first = first_token(&lower);

    // Dependency installs
    matches!(
        first,
        "npm" | "pnpm" | "yarn" | "pip" | "pip3" | "cargo" | "brew" | "apt" | "apt-get" | "choco"
    ) && (lower.contains("install") || lower.contains("add") || lower.contains("update") || lower.contains("upgrade"))
    // Source edits
    || matches!(first, "sed" | "awk" | "perl") && is_write_redirect(command)
    // File moves / copies into source tree
    || matches!(first, "mv" | "cp" | "copy") && !targets_sego_metadata(command)
    // Git mutations (commit, merge, rebase, etc.) that are not read-only
    || (first == "git" && !is_git_readonly(&lower))
    // Direct file-write tools
    || matches!(first, "tee" | "dd") && !targets_sego_metadata(command)
}

/// Detect output redirection (`>`, `>>`, `tee`).
fn is_write_redirect(command: &str) -> bool {
    command.contains('>') || command.contains("tee ")
}

/// Git read-only subcommands.
fn is_git_readonly(lower: &str) -> bool {
    if !lower.starts_with("git ") {
        return false;
    }
    let rest = &lower[4..];
    let sub = first_token(rest);
    // `git branch` is read-only for listing, but `-D`/`--delete --force` is destructive.
    // Note: `lower` is already lowercased, so `-D` appears as `-d`.
    if sub == "branch" {
        return !rest.contains(" -d ") && !rest.contains(" --delete --force");
    }
    matches!(
        sub,
        "status"
            | "diff"
            | "log"
            | "show"
            | "blame"
            | "ls-files"
            | "ls-tree"
            | "rev-parse"
            | "remote"
            | "config"
            | "describe"
            | "shortlog"
            | "name-rev"
    ) && !rest.contains(" --set-")
}

/// `gh` read-only subcommands (view, list, status, api GET).
fn is_gh_readonly(lower: &str) -> bool {
    if !lower.starts_with("gh ") {
        return false;
    }
    let rest = &lower[3..];
    let sub = first_token(rest);
    matches!(sub, "status" | "view" | "list" | "search" | "api")
}

/// Extract the first whitespace-delimited token from a command string.
fn first_token(lower: &str) -> &str {
    lower.split_whitespace().next().unwrap_or("")
}

/// Extract the second whitespace-delimited token (for `cargo test`, `npm test`).
fn second_token(lower: &str) -> Option<String> {
    let mut parts = lower.split_whitespace();
    parts.next()?; // skip first
                   // For "npm run lint" we want the subcommand pair; return the next single token.
    parts.next().map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readonly_commands_are_safe() {
        assert_eq!(classify_bash_command("cat foo.rs"), BashCommandRisk::SafeReadonly);
        assert_eq!(classify_bash_command("rg \"pattern\" src/"), BashCommandRisk::SafeReadonly);
        assert_eq!(classify_bash_command("grep -r foo ."), BashCommandRisk::SafeReadonly);
        assert_eq!(classify_bash_command("ls -la"), BashCommandRisk::SafeReadonly);
        assert_eq!(classify_bash_command("git status"), BashCommandRisk::SafeReadonly);
        assert_eq!(classify_bash_command("git diff HEAD~1"), BashCommandRisk::SafeReadonly);
        assert_eq!(classify_bash_command("git log --oneline -5"), BashCommandRisk::SafeReadonly);
    }

    #[test]
    fn verify_commands_are_safe() {
        assert_eq!(classify_bash_command("cargo test"), BashCommandRisk::SafeVerify);
        assert_eq!(classify_bash_command("cargo build"), BashCommandRisk::SafeVerify);
        assert_eq!(classify_bash_command("cargo clippy"), BashCommandRisk::SafeVerify);
        assert_eq!(classify_bash_command("npm test"), BashCommandRisk::SafeVerify);
        assert_eq!(classify_bash_command("go test ./..."), BashCommandRisk::SafeVerify);
        assert_eq!(classify_bash_command("pytest"), BashCommandRisk::SafeVerify);
    }

    #[test]
    fn sego_metadata_writes_are_allowed() {
        assert_eq!(
            classify_bash_command("mkdir -p .sego/reviews"),
            BashCommandRisk::SegaMetadataWrite
        );
        assert_eq!(
            classify_bash_command("echo {} > .sego/reviews/idx.json"),
            BashCommandRisk::SegaMetadataWrite
        );
    }

    #[test]
    fn dangerous_commands_are_denied() {
        assert_eq!(classify_bash_command("rm -rf target/"), BashCommandRisk::DenyDangerous);
        assert_eq!(
            classify_bash_command("git reset --hard origin/main"),
            BashCommandRisk::DenyDangerous
        );
        assert_eq!(classify_bash_command("git push origin main"), BashCommandRisk::DenyDangerous);
        assert_eq!(classify_bash_command("git branch -D feat/old"), BashCommandRisk::DenyDangerous);
        assert_eq!(classify_bash_command("sudo rm /etc/hosts"), BashCommandRisk::DenyDangerous);
    }

    #[test]
    fn mutations_require_ask() {
        assert_eq!(classify_bash_command("npm install react"), BashCommandRisk::AskMutation);
        assert_eq!(classify_bash_command("cargo install ripgrep"), BashCommandRisk::AskMutation);
        assert_eq!(classify_bash_command("git commit -m msg"), BashCommandRisk::AskMutation);
        assert_eq!(classify_bash_command("git merge feat/x"), BashCommandRisk::AskMutation);
    }

    #[test]
    fn unsafe_chains_are_denied() {
        assert_eq!(classify_bash_command("cat foo && rm -rf /"), BashCommandRisk::DenyDangerous);
        assert_eq!(classify_bash_command("ls; git push"), BashCommandRisk::DenyDangerous);
        assert_eq!(classify_bash_command("echo $(rm -rf /)"), BashCommandRisk::DenyDangerous);
        assert_eq!(
            classify_bash_command("echo harmless | dangerous-cmd"),
            BashCommandRisk::DenyDangerous,
            "pipe commands must not be classified as safe (Sego c10/b review)"
        );
    }

    #[test]
    fn unknown_commands_ask() {
        assert_eq!(classify_bash_command("some-weird-tool --flag"), BashCommandRisk::UnknownAsk);
        assert_eq!(classify_bash_command(""), BashCommandRisk::UnknownAsk);
    }

    #[test]
    fn is_auto_allow_flag() {
        assert!(BashCommandRisk::SafeReadonly.is_auto_allow());
        assert!(BashCommandRisk::SafeVerify.is_auto_allow());
        assert!(BashCommandRisk::SegaMetadataWrite.is_auto_allow());
        assert!(!BashCommandRisk::AskMutation.is_auto_allow());
        assert!(!BashCommandRisk::DenyDangerous.is_auto_allow());
        assert!(!BashCommandRisk::UnknownAsk.is_auto_allow());
    }

    #[test]
    fn classify_bash_tool_input_handles_json_command_field() {
        // Safe readonly via JSON -> Allow
        assert_eq!(
            classify_bash_tool_input(r#"{"command":"git status"}"#),
            BashCommandRisk::SafeReadonly
        );
        // Safe verify via JSON -> Allow
        assert_eq!(
            classify_bash_tool_input(r#"{"command":"cargo test"}"#),
            BashCommandRisk::SafeVerify
        );
        // Pipe inside JSON command -> DenyDangerous
        assert_eq!(
            classify_bash_tool_input(r#"{"command":"echo harmless | dangerous-cmd"}"#),
            BashCommandRisk::DenyDangerous
        );
        // Unknown tool via JSON -> UnknownAsk
        assert_eq!(
            classify_bash_tool_input(r#"{"command":"some-weird-tool --flag"}"#),
            BashCommandRisk::UnknownAsk
        );
        // Non-string command -> fail safe
        assert_eq!(classify_bash_tool_input(r#"{"command":123}"#), BashCommandRisk::UnknownAsk);
        // Missing command field -> fail safe
        assert_eq!(classify_bash_tool_input(r#"{"other":"value"}"#), BashCommandRisk::UnknownAsk);
        // Invalid JSON -> fail safe
        assert_eq!(
            classify_bash_tool_input(r#"{"command":"unclosed"#),
            BashCommandRisk::UnknownAsk
        );
        // Bare command (non-JSON) still works
        assert_eq!(classify_bash_tool_input("git log --oneline"), BashCommandRisk::SafeReadonly);
    }
}
