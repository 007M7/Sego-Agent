//! Phase 2-C: Full-scope preflight gate for `review --full` and Git-path review.
//!
//! This module implements the allow/block preflight contract defined in
//! `SEGO-PHASE2B-FULL-SCOPE-PREFLIGHT-CONTRACT-2026-07-12.md`.
//!
//! ## Terminal decision
//!
//! The preflight has exactly two terminal states: `Allow` or `Block`.
//! `exclusions` is an evidence collection, not a terminal state.
//! Only `Allow` permits snapshot collection; `Block` guarantees snapshot
//! collection and model runtime are never reached.
//!
//! ## PEP policy
//!
//! PEP-001..005 are closed, versioned policy rules. A path not matched by an
//! accepted rule is never silently excluded. `.gitignore` is Git behavior
//! evidence, not a policy rule or allow signal.

use std::path::{Path, PathBuf};

/// Current policy version. Must be incremented if PEP rules change.
pub const PREFLIGHT_POLICY_VERSION: &str = "sego-pep-v1-2026-07-12";

/// Terminal preflight decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightDecision {
    Allow,
    Block,
}

/// Reason for a block decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockReason {
    /// Non-Git directory targeted for full review (FSP-09).
    NonGitTarget,
    /// Drive-rooted target outside the current worktree (FSP-08).
    OutsideWorktreeDriveTarget,
    /// Untracked top-level directory that is not a known external candidate (FSP-03).
    UntrackedTopLevelDirectory { dir_name: String },
    /// Untracked top-level file that is not an accepted direct-entry policy match.
    UntrackedTopLevelEntry { entry_name: String },
    /// A policy parent contains a child that is not part of its accepted path set.
    PolicyEntryContainsUnmatchedContent { path: String },
    /// External repository candidate detected via embedded Git probe (FSP-02, FSP-11).
    ExternalRepositoryDetected { dir_name: String },
    /// Top-level path is git-ignored and not matched by an accepted policy rule (FSP-12).
    IgnoredTopLevelNotPolicyMatched { dir_name: String },
    /// Target path does not exist (FSP-06 for Git-path, general for full).
    TargetNotFound,
    /// Target path is not a directory for full review.
    TargetNotDirectory,
    /// Git path is outside the Git root (FSP-06 variant).
    GitPathOutsideRoot,
    /// Git path does not exist (FSP-06).
    GitPathNotFound { path: String },
    /// Existing path has no Git-tracked content and cannot form a Git path review.
    GitPathNotTracked { path: String },
    /// Unsupported flag or incompatible scope form (FSP-10).
    UnsupportedScope { detail: String },
}

impl BlockReason {
    /// Stable machine-readable code for this reason.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NonGitTarget => "non_git_target",
            Self::OutsideWorktreeDriveTarget => "outside_worktree_drive_target",
            Self::UntrackedTopLevelDirectory { .. } => "untracked_top_level_directory",
            Self::UntrackedTopLevelEntry { .. } => "untracked_top_level_entry",
            Self::PolicyEntryContainsUnmatchedContent { .. } => {
                "policy_entry_contains_unmatched_content"
            }
            Self::ExternalRepositoryDetected { .. } => "external_repository_detected",
            Self::IgnoredTopLevelNotPolicyMatched { .. } => "ignored_top_level_not_policy_matched",
            Self::TargetNotFound => "target_not_found",
            Self::TargetNotDirectory => "target_not_directory",
            Self::GitPathOutsideRoot => "git_path_outside_root",
            Self::GitPathNotFound { .. } => "git_path_not_found",
            Self::GitPathNotTracked { .. } => "git_path_not_tracked",
            Self::UnsupportedScope { .. } => "unsupported_scope",
        }
    }

    /// Human-readable explanation.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::NonGitTarget => "target is not a Git worktree; filesystem-only review is not permitted under P0 safety default".to_string(),
            Self::OutsideWorktreeDriveTarget => "drive-rooted target is outside the current worktree".to_string(),
            Self::UntrackedTopLevelDirectory { dir_name } => format!("untracked top-level directory `{dir_name}` is not matched by an accepted policy rule; use a narrower managed path"),
            Self::UntrackedTopLevelEntry { entry_name } => format!("untracked top-level entry `{entry_name}` is not matched by an accepted policy rule"),
            Self::PolicyEntryContainsUnmatchedContent { path } => format!("policy entry contains unmatched content at `{path}`"),
            Self::ExternalRepositoryDetected { dir_name } => format!("external repository candidate `{dir_name}` detected via embedded-Git probe; block by default"),
            Self::IgnoredTopLevelNotPolicyMatched { dir_name } => format!("git-ignored top-level `{dir_name}` is not matched by an accepted policy rule; .gitignore is not an allow signal"),
            Self::TargetNotFound => "target path does not exist".to_string(),
            Self::TargetNotDirectory => "full review target is not a directory".to_string(),
            Self::GitPathOutsideRoot => "Git path resolves outside the Git root".to_string(),
            Self::GitPathNotFound { path } => format!("Git path `{path}` does not exist"),
            Self::GitPathNotTracked { path } => format!("Git path `{path}` has no tracked content and cannot form a review scope"),
            Self::UnsupportedScope { detail } => format!("unsupported or incompatible scope: {detail}"),
        }
    }
}

/// A single exclusion evidence record for a matched PEP policy rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightExclusion {
    pub policy_version: String,
    pub rule_id: String,
    pub category: String,
    pub relative_path: String,
    pub owner: String,
    pub rationale: String,
}

/// A classification event observed during preflight (for evidence/debug).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassificationEvent {
    pub event_type: String,
    pub detail: String,
}

/// Complete preflight result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightResult {
    pub decision: PreflightDecision,
    pub block_reason: Option<BlockReason>,
    pub exclusions: Vec<PreflightExclusion>,
    pub classification_events: Vec<ClassificationEvent>,
    pub resolved_target: Option<PathBuf>,
    pub git_root: Option<PathBuf>,
    pub git_status_observed: bool,
    pub snapshot_started: bool,
}

impl PreflightResult {
    #[must_use]
    pub fn allow(
        resolved_target: PathBuf,
        git_root: Option<PathBuf>,
        exclusions: Vec<PreflightExclusion>,
        events: Vec<ClassificationEvent>,
    ) -> Self {
        let git_status_observed = git_root.is_some();
        Self {
            decision: PreflightDecision::Allow,
            block_reason: None,
            exclusions,
            classification_events: events,
            resolved_target: Some(resolved_target),
            git_root,
            git_status_observed,
            snapshot_started: false,
        }
    }

    #[must_use]
    pub fn block(
        reason: BlockReason,
        resolved_target: Option<PathBuf>,
        git_root: Option<PathBuf>,
        events: Vec<ClassificationEvent>,
    ) -> Self {
        let git_status_observed = git_root.is_some();
        Self {
            decision: PreflightDecision::Block,
            block_reason: Some(reason),
            exclusions: Vec::new(),
            classification_events: events,
            resolved_target,
            git_root,
            git_status_observed,
            snapshot_started: false,
        }
    }

    #[must_use]
    pub fn is_allow(&self) -> bool {
        self.decision == PreflightDecision::Allow
    }

    #[must_use]
    pub fn is_block(&self) -> bool {
        self.decision == PreflightDecision::Block
    }
}

// ---------------------------------------------------------------------------
// PEP policy matching (PEP-001..005)
// ---------------------------------------------------------------------------

const PEP_001_DIRS: &[&str] = &["target", "build", "dist", "coverage"];
const PEP_002_DIRS: &[&str] = &[".idea", ".vscode", ".vs"];
const PEP_003_EXACT_DIRS: &[&str] = &[".port_sessions", ".sandbox-home", ".sandbox-tmp"];
const PEP_003_SEGO_CHILD_DIRS: &[&str] = &["reviews", "exports", "recovery"];
const PEP_004_EXACT_ENTRIES: &[&str] = &[".claude", ".claw", "CLAUDE.md", "ZCODE.md"];
const PEP_005_PREFIXES: &[&str] = &["AionUi", "Coolearn", "hermes"];

#[must_use]
fn matches_pep_004(name: &str) -> bool {
    PEP_004_EXACT_ENTRIES.contains(&name)
        || name
            .strip_prefix("SEGO_SYNC_")
            .is_some_and(|suffix| !suffix.is_empty() && suffix.ends_with(".txt"))
        || name
            .strip_prefix("SEGO_TASK_")
            .is_some_and(|suffix| !suffix.is_empty() && suffix.ends_with(".md"))
}

#[must_use]
fn is_pep_005_annotation(name: &str) -> bool {
    PEP_005_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

fn exclusion(
    rule_id: &str,
    category: &str,
    relative_path: String,
    owner: &str,
    rationale: &str,
) -> PreflightExclusion {
    PreflightExclusion {
        policy_version: PREFLIGHT_POLICY_VERSION.to_string(),
        rule_id: rule_id.to_string(),
        category: category.to_string(),
        relative_path,
        owner: owner.to_string(),
        rationale: rationale.to_string(),
    }
}

fn pep_003_sego_exclusions(entry_path: &Path) -> Result<Vec<PreflightExclusion>, BlockReason> {
    let entries = std::fs::read_dir(entry_path).map_err(|_| {
        BlockReason::PolicyEntryContainsUnmatchedContent { path: ".sego".to_string() }
    })?;
    let mut exclusions = Vec::new();
    let mut found_accepted_child = false;

    for child in entries.flatten() {
        let child_name = child.file_name().to_string_lossy().to_string();
        let child_path = child.path();
        if PEP_003_SEGO_CHILD_DIRS.contains(&child_name.as_str()) && child_path.is_dir() {
            found_accepted_child = true;
            exclusions.push(exclusion(
                "PEP-003",
                "runtime_session_recovery",
                format!(".sego/{child_name}"),
                "Sego Master",
                "registered runtime/session/recovery output path",
            ));
        } else {
            return Err(BlockReason::PolicyEntryContainsUnmatchedContent {
                path: format!(".sego/{child_name}"),
            });
        }
    }

    if !found_accepted_child {
        return Err(BlockReason::PolicyEntryContainsUnmatchedContent { path: ".sego".to_string() });
    }
    Ok(exclusions)
}

fn try_match_pep_policy(
    name: &str,
    entry_path: &Path,
    is_dir: bool,
) -> Result<Option<Vec<PreflightExclusion>>, BlockReason> {
    if is_dir && PEP_001_DIRS.contains(&name) {
        return Ok(Some(vec![exclusion(
            "PEP-001",
            "managed_build_output",
            name.to_string(),
            "Product Engineering",
            "exact declared-review-root build output directory",
        )]));
    }
    if is_dir && PEP_002_DIRS.contains(&name) {
        return Ok(Some(vec![exclusion(
            "PEP-002",
            "editor_ide_local_state",
            name.to_string(),
            "Product Engineering",
            "exact declared-review-root editor/IDE state directory",
        )]));
    }
    if is_dir && name == ".sego" {
        return pep_003_sego_exclusions(entry_path).map(Some);
    }
    if is_dir && PEP_003_EXACT_DIRS.contains(&name) {
        return Ok(Some(vec![exclusion(
            "PEP-003",
            "runtime_session_recovery",
            name.to_string(),
            "Sego Master",
            "registered runtime/session/recovery output path",
        )]));
    }
    if matches_pep_004(name) {
        return Ok(Some(vec![exclusion(
            "PEP-004",
            "private_credentials_config",
            name.to_string(),
            "Founder/Sego Master",
            "declared-review-root direct top-level private/config entry",
        )]));
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Git helpers (reused from main, but isolated for testability)
// ---------------------------------------------------------------------------

fn git_output(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| format!("git spawn failed: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("git {} failed: {stderr}", args.join(" ")));
    }
    String::from_utf8(output.stdout).map_err(|e| format!("git output not UTF-8: {e}"))
}

#[must_use]
pub fn is_git_worktree_at(cwd: &Path) -> bool {
    std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(cwd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn git_toplevel(cwd: &Path) -> Option<PathBuf> {
    git_output(cwd, &["rev-parse", "--show-toplevel"]).ok().and_then(|s| {
        let trimmed = s.trim();
        (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
    })
}

fn git_relative_path(git_root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(git_root).ok().map(|relative| relative.to_string_lossy().replace('\\', "/"))
}

fn has_git_tracked_content(git_root: &Path, relative_path: &str) -> bool {
    git_output(git_root, &["ls-files", "--", relative_path])
        .map(|output| !output.trim().is_empty())
        .unwrap_or(false)
}

fn is_git_ignored(git_root: &Path, relative_path: &str) -> bool {
    git_output(git_root, &["check-ignore", "--", relative_path])
        .map(|output| !output.trim().is_empty())
        .unwrap_or(false)
}

fn has_embedded_git(review_root: &Path, name: &str) -> bool {
    review_root.join(name).join(".git").exists()
}

fn sorted_direct_entries(review_root: &Path) -> Result<Vec<std::fs::DirEntry>, BlockReason> {
    let entries = std::fs::read_dir(review_root).map_err(|_| {
        BlockReason::PolicyEntryContainsUnmatchedContent { path: review_root.display().to_string() }
    })?;
    let mut entries: Vec<std::fs::DirEntry> = entries.flatten().collect();
    entries.sort_by(|left, right| {
        left.file_name().to_string_lossy().cmp(&right.file_name().to_string_lossy())
    });
    Ok(entries)
}

// ---------------------------------------------------------------------------
// Full-review preflight (FSP-01..04, FSP-07..12)
// ---------------------------------------------------------------------------

/// Run the full-review preflight for `review --full <path>`.
///
/// `cwd` is the current working directory (for resolving relative paths and
/// checking worktree membership). `target` is the declared filesystem subtree
/// path (already parsed, may be relative or absolute).
///
/// Returns a `PreflightResult`. The caller must check `is_allow()` before
/// starting snapshot collection. If `is_block()`, snapshot and model runtime
/// must not be reached.
pub fn run_full_review_preflight(cwd: &Path, target: &Path) -> PreflightResult {
    let mut events: Vec<ClassificationEvent> = Vec::new();
    let resolved_raw = if target.is_absolute() { target.to_path_buf() } else { cwd.join(target) };

    if !resolved_raw.exists() {
        events.push(ClassificationEvent {
            event_type: "target_check".to_string(),
            detail: format!("target does not exist: {}", resolved_raw.display()),
        });
        return PreflightResult::block(BlockReason::TargetNotFound, None, None, events);
    }

    let resolved = match std::fs::canonicalize(&resolved_raw) {
        Ok(path) => path,
        Err(error) => {
            events.push(ClassificationEvent {
                event_type: "canonicalize_failed".to_string(),
                detail: format!("cannot canonicalize {}: {error}", resolved_raw.display()),
            });
            return PreflightResult::block(BlockReason::TargetNotFound, None, None, events);
        }
    };
    events.push(ClassificationEvent {
        event_type: "resolved_target".to_string(),
        detail: resolved.display().to_string(),
    });

    if !resolved.is_dir() {
        return PreflightResult::block(
            BlockReason::TargetNotDirectory,
            Some(resolved),
            None,
            events,
        );
    }
    if !is_git_worktree_at(&resolved) {
        return PreflightResult::block(BlockReason::NonGitTarget, Some(resolved), None, events);
    }

    let git_root = match git_toplevel(&resolved)
        .map(|root| std::fs::canonicalize(&root).unwrap_or(root))
    {
        Some(root) => root,
        None => {
            return PreflightResult::block(BlockReason::NonGitTarget, Some(resolved), None, events)
        }
    };
    let cwd_git_root =
        match git_toplevel(cwd).map(|root| std::fs::canonicalize(&root).unwrap_or(root)) {
            Some(root) => root,
            None => {
                return PreflightResult::block(
                    BlockReason::OutsideWorktreeDriveTarget,
                    Some(resolved),
                    Some(git_root),
                    events,
                )
            }
        };
    if cwd_git_root != git_root {
        events.push(ClassificationEvent {
            event_type: "outside_worktree".to_string(),
            detail: format!(
                "target git root {} != cwd git root {}",
                git_root.display(),
                cwd_git_root.display()
            ),
        });
        return PreflightResult::block(
            BlockReason::OutsideWorktreeDriveTarget,
            Some(resolved),
            Some(git_root),
            events,
        );
    }

    let mut exclusions = Vec::new();
    let entries = match sorted_direct_entries(&resolved) {
        Ok(entries) => entries,
        Err(reason) => {
            return PreflightResult::block(reason, Some(resolved), Some(git_root), events)
        }
    };

    for entry in entries {
        let name = entry.file_name().to_string_lossy().to_string();
        if resolved == git_root && name == ".git" {
            continue;
        }
        let entry_path = entry.path();
        let Some(relative_path) = git_relative_path(&git_root, &entry_path) else {
            return PreflightResult::block(
                BlockReason::OutsideWorktreeDriveTarget,
                Some(resolved),
                Some(git_root),
                events,
            );
        };
        if has_git_tracked_content(&git_root, &relative_path) {
            continue;
        }

        let is_dir = entry_path.is_dir();
        if is_dir && has_embedded_git(&resolved, &name) {
            events.push(ClassificationEvent {
                event_type: "external_repository_detected".to_string(),
                detail: format!("{name} contains embedded .git"),
            });
            return PreflightResult::block(
                BlockReason::ExternalRepositoryDetected { dir_name: name },
                Some(resolved),
                Some(git_root),
                events,
            );
        }
        if is_pep_005_annotation(&name) {
            return PreflightResult::block(
                if is_dir {
                    BlockReason::UntrackedTopLevelDirectory { dir_name: name }
                } else {
                    BlockReason::UntrackedTopLevelEntry { entry_name: name }
                },
                Some(resolved),
                Some(git_root),
                events,
            );
        }

        match try_match_pep_policy(&name, &entry_path, is_dir) {
            Ok(Some(mut matches)) => {
                events.push(ClassificationEvent {
                    event_type: "policy_exclusion".to_string(),
                    detail: format!("{name} matched accepted policy"),
                });
                exclusions.append(&mut matches);
                continue;
            }
            Ok(None) => {}
            Err(reason) => {
                return PreflightResult::block(reason, Some(resolved), Some(git_root), events)
            }
        }

        if is_git_ignored(&git_root, &relative_path) {
            return PreflightResult::block(
                BlockReason::IgnoredTopLevelNotPolicyMatched { dir_name: name },
                Some(resolved),
                Some(git_root),
                events,
            );
        }

        return PreflightResult::block(
            if is_dir {
                BlockReason::UntrackedTopLevelDirectory { dir_name: name }
            } else {
                BlockReason::UntrackedTopLevelEntry { entry_name: name }
            },
            Some(resolved),
            Some(git_root),
            events,
        );
    }

    events.push(ClassificationEvent {
        event_type: "preflight_allow".to_string(),
        detail: format!("{} exclusions recorded", exclusions.len()),
    });
    PreflightResult::allow(resolved, Some(git_root), exclusions, events)
}

// ---------------------------------------------------------------------------
// Git-path preflight (FSP-05, FSP-06)
// ---------------------------------------------------------------------------

/// Run the Git-path preflight for `review <path>`.
///
/// The path must exist and resolve inside the Git root. Missing or
/// outside-root paths block; empty diff must not masquerade as success.
pub fn run_git_path_preflight(cwd: &Path, path: &Path) -> PreflightResult {
    let mut events: Vec<ClassificationEvent> = Vec::new();

    // Must be in a Git worktree.
    if !is_git_worktree_at(cwd) {
        events.push(ClassificationEvent {
            event_type: "non_git_cwd".to_string(),
            detail: cwd.display().to_string(),
        });
        return PreflightResult::block(BlockReason::NonGitTarget, None, None, events);
    }

    let git_root = match git_toplevel(cwd) {
        Some(root) => root,
        None => {
            return PreflightResult::block(BlockReason::NonGitTarget, None, None, events);
        }
    };

    // Canonicalize the git root so path comparisons use the same format as
    // canonicalized resolved paths (e.g. \\?\C:\... on Windows).
    let git_root = std::fs::canonicalize(&git_root).unwrap_or(git_root);

    events.push(ClassificationEvent {
        event_type: "git_root".to_string(),
        detail: git_root.display().to_string(),
    });

    // Resolve the path relative to cwd.
    let resolved_raw = if path.is_absolute() { path.to_path_buf() } else { cwd.join(path) };

    // Check existence (FSP-06).
    if !resolved_raw.exists() {
        let path_str = path.to_string_lossy().to_string();
        events.push(ClassificationEvent {
            event_type: "git_path_not_found".to_string(),
            detail: path_str.clone(),
        });
        return PreflightResult::block(
            BlockReason::GitPathNotFound { path: path_str },
            None,
            Some(git_root),
            events,
        );
    }

    // Canonicalize.
    let resolved = match std::fs::canonicalize(&resolved_raw) {
        Ok(p) => p,
        Err(_) => {
            let path_str = path.to_string_lossy().to_string();
            return PreflightResult::block(
                BlockReason::GitPathNotFound { path: path_str },
                None,
                Some(git_root),
                events,
            );
        }
    };

    // Check that resolved path is inside the Git root (FSP-06 variant).
    if !resolved.starts_with(&git_root) {
        events.push(ClassificationEvent {
            event_type: "git_path_outside_root".to_string(),
            detail: resolved.display().to_string(),
        });
        return PreflightResult::block(
            BlockReason::GitPathOutsideRoot,
            Some(resolved),
            Some(git_root),
            events,
        );
    }

    let relative_path = git_relative_path(&git_root, &resolved).unwrap_or_default();
    if !has_git_tracked_content(&git_root, &relative_path) {
        events.push(ClassificationEvent {
            event_type: "git_path_not_tracked".to_string(),
            detail: relative_path.clone(),
        });
        return PreflightResult::block(
            BlockReason::GitPathNotTracked { path: relative_path },
            Some(resolved),
            Some(git_root),
            events,
        );
    }

    events.push(ClassificationEvent {
        event_type: "git_path_allow".to_string(),
        detail: resolved.display().to_string(),
    });

    // Git-path review uses Git diff semantics; no full-snapshot claim.
    PreflightResult::allow(resolved, Some(git_root), Vec::new(), events)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("sego-preflight-{nanos}"))
    }

    fn git(args: &[&str], cwd: &Path) {
        let status = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .expect("git command should run");
        assert!(status.success(), "git command failed: git {}", args.join(" "));
    }

    /// Set up a clean Git worktree with one tracked source file.
    fn make_clean_git_root() -> PathBuf {
        let root = temp_dir();
        fs::create_dir_all(&root).expect("root dir");
        git(&["init", "--quiet"], &root);
        git(&["config", "user.email", "tests@example.com"], &root);
        git(&["config", "user.name", "Sego Preflight Tests"], &root);
        fs::create_dir_all(root.join("src")).expect("src dir");
        fs::write(root.join("src").join("lib.rs"), "pub fn x() {}\n").expect("write lib");
        git(&["add", "src/lib.rs"], &root);
        git(&["commit", "-m", "init", "--quiet"], &root);
        root
    }

    // FSP-01: Clean Git root allow, empty exclusions, snapshot allowed.
    #[test]
    fn fsp_01_clean_git_root_allow() {
        let _guard = env_lock();
        let root = make_clean_git_root();

        let result = run_full_review_preflight(&root, Path::new("."));

        assert_eq!(result.decision, PreflightDecision::Allow);
        assert!(result.exclusions.is_empty());
        assert!(!result.snapshot_started);
        assert!(result.resolved_target.is_some());
        assert!(result.git_root.is_some());

        fs::remove_dir_all(&root).expect("cleanup");
    }

    // FSP-02: Top-level embedded Git external block; no snapshot/model.
    #[test]
    fn fsp_02_embedded_git_external_block() {
        let _guard = env_lock();
        let root = make_clean_git_root();

        // Create untracked top-level dir with embedded .git.
        let external = root.join("external-a");
        fs::create_dir_all(&external).expect("external dir");
        git(&["init", "--quiet"], &external);
        // Fixtures must not depend on a developer-machine global Git identity.
        git(&["config", "user.email", "tests@example.com"], &external);
        git(&["config", "user.name", "Sego Preflight Tests"], &external);
        fs::write(external.join("stub.txt"), "external\n").expect("stub");
        git(&["add", "."], &external);
        git(&["commit", "-m", "ext", "--quiet"], &external);

        let result = run_full_review_preflight(&root, Path::new("."));

        assert_eq!(result.decision, PreflightDecision::Block);
        assert!(!result.snapshot_started);
        match &result.block_reason {
            Some(BlockReason::ExternalRepositoryDetected { dir_name }) => {
                assert_eq!(dir_name, "external-a");
            }
            other => panic!("expected ExternalRepositoryDetected, got {other:?}"),
        }

        fs::remove_dir_all(&root).expect("cleanup");
    }

    // FSP-03: Unknown untracked top-level non-Git dir block.
    #[test]
    fn fsp_03_unknown_untracked_top_level_block() {
        let _guard = env_lock();
        let root = make_clean_git_root();

        fs::create_dir_all(root.join("scratch")).expect("scratch dir");
        fs::write(root.join("scratch").join("tmp.txt"), "temp\n").expect("tmp");

        let result = run_full_review_preflight(&root, Path::new("."));

        assert_eq!(result.decision, PreflightDecision::Block);
        assert!(!result.snapshot_started);
        match &result.block_reason {
            Some(BlockReason::UntrackedTopLevelDirectory { dir_name }) => {
                assert_eq!(dir_name, "scratch");
            }
            other => panic!("expected UntrackedTopLevelDirectory, got {other:?}"),
        }

        fs::remove_dir_all(&root).expect("cleanup");
    }

    // FSP-04: Accepted PEP paths -> allow + complete exclusions evidence.
    #[test]
    fn fsp_04_accepted_pep_paths_allow_with_exclusions() {
        let _guard = env_lock();
        let root = make_clean_git_root();

        // Create PEP-001, PEP-002, PEP-003 top-level dirs (untracked).
        // Each must contain a file - git does not track empty directories.
        fs::create_dir_all(root.join("target")).expect("target");
        fs::write(root.join("target").join("out.txt"), "build\n").expect("build out");
        fs::create_dir_all(root.join(".idea")).expect("idea");
        fs::write(root.join(".idea").join("workspace.xml"), "<x/>\n").expect("idea file");
        fs::create_dir_all(root.join(".sego").join("reviews")).expect("sego reviews");
        fs::write(root.join(".sego").join("reviews").join("r.json"), "{}\n").expect("sego file");

        let result = run_full_review_preflight(&root, Path::new("."));

        assert_eq!(result.decision, PreflightDecision::Allow);
        assert!(!result.snapshot_started);
        assert!(!result.exclusions.is_empty());
        let rule_ids: Vec<&str> = result.exclusions.iter().map(|e| e.rule_id.as_str()).collect();
        assert!(rule_ids.contains(&"PEP-001"));
        assert!(rule_ids.contains(&"PEP-002"));
        assert!(rule_ids.contains(&"PEP-003"));
        // Each exclusion must have policy_version, owner, rationale.
        for excl in &result.exclusions {
            assert!(!excl.policy_version.is_empty());
            assert!(!excl.owner.is_empty());
            assert!(!excl.rationale.is_empty());
        }

        fs::remove_dir_all(&root).expect("cleanup");
    }

    // FSP-05: Existing Git path allow, no full-snapshot claim.
    #[test]
    fn fsp_05_existing_git_path_allow() {
        let _guard = env_lock();
        let root = make_clean_git_root();

        let result = run_git_path_preflight(&root, Path::new("src/lib.rs"));

        assert_eq!(
            result.decision,
            PreflightDecision::Allow,
            "events: {:?}, block_reason: {:?}",
            result.classification_events,
            result.block_reason
        );
        assert!(result.exclusions.is_empty());
        // Git-path review does not produce full-snapshot claim.
        assert!(result.resolved_target.is_some());

        fs::remove_dir_all(&root).expect("cleanup");
    }

    // FSP-06: Missing Git path block, no empty-diff success.
    #[test]
    fn fsp_06_missing_git_path_block() {
        let _guard = env_lock();
        let root = make_clean_git_root();

        let result = run_git_path_preflight(&root, Path::new("src/missing.rs"));

        assert_eq!(result.decision, PreflightDecision::Block);
        assert!(!result.snapshot_started);
        match &result.block_reason {
            Some(BlockReason::GitPathNotFound { path }) => {
                assert!(path.contains("missing.rs"));
            }
            other => panic!("expected GitPathNotFound, got {other:?}"),
        }

        fs::remove_dir_all(&root).expect("cleanup");
    }

    // FSP-07: Windows drive/path with spaces — no token loss.
    #[test]
    fn fsp_07_path_with_spaces_no_token_loss() {
        let _guard = env_lock();
        let root = make_clean_git_root();

        // Use a subdirectory with spaces inside the Git root.
        let spaced = root.join("Fixture Space");
        fs::create_dir_all(&spaced).expect("spaced dir");
        // Must put a file inside - git does not track empty directories.
        fs::write(spaced.join("data.txt"), "data\n").expect("write data");
        // The spaced dir is untracked top-level, but we test that the path
        // with spaces is preserved in resolved_target.
        let result = run_full_review_preflight(&root, Path::new("Fixture Space"));

        // It will block because "Fixture Space" is an untracked top-level dir,
        // but the resolved_target must preserve the spaces.
        assert_eq!(
            result.decision,
            PreflightDecision::Block,
            "events: {:?}, block_reason: {:?}",
            result.classification_events,
            result.block_reason
        );
        if let Some(resolved) = &result.resolved_target {
            assert!(resolved.to_string_lossy().contains("Fixture Space"));
        }

        fs::remove_dir_all(&root).expect("cleanup");
    }

    // FSP-08: Worktree-external drive target block.
    #[test]
    fn fsp_08_outside_worktree_drive_target_block() {
        let _guard = env_lock();
        let root = make_clean_git_root();
        let other = make_clean_git_root();

        // Run preflight from `root` but targeting `other` (different Git root).
        let result = run_full_review_preflight(&root, &other);

        assert_eq!(result.decision, PreflightDecision::Block);
        match &result.block_reason {
            Some(BlockReason::OutsideWorktreeDriveTarget) => {}
            other_reason => panic!("expected OutsideWorktreeDriveTarget, got {other_reason:?}"),
        }

        fs::remove_dir_all(&root).expect("cleanup root");
        fs::remove_dir_all(&other).expect("cleanup other");
    }

    // FSP-09: Non-Git target block.
    #[test]
    fn fsp_09_non_git_target_block() {
        let _guard = env_lock();
        let root = temp_dir();
        fs::create_dir_all(&root).expect("non-git dir");
        assert!(!is_git_worktree_at(&root));

        let result = run_full_review_preflight(&root, Path::new("."));

        assert_eq!(result.decision, PreflightDecision::Block);
        assert!(!result.snapshot_started);
        match &result.block_reason {
            Some(BlockReason::NonGitTarget) => {}
            other => panic!("expected NonGitTarget, got {other:?}"),
        }

        fs::remove_dir_all(&root).expect("cleanup");
    }

    // Missing full target blocks before snapshot. Parser-level FSP-10 coverage
    // for incompatible scope flags lives in runtime::code_review::scope tests.
    #[test]
    fn full_target_not_found_blocks_before_snapshot() {
        let _guard = env_lock();
        let root = make_clean_git_root();

        let result = run_full_review_preflight(&root, Path::new("does-not-exist-xyz"));

        assert_eq!(result.decision, PreflightDecision::Block);
        assert!(!result.snapshot_started);
        assert!(matches!(result.block_reason, Some(BlockReason::TargetNotFound)));

        fs::remove_dir_all(&root).expect("cleanup");
    }

    // FSP-11: AionUi name is not the decisive factor; generic embedded-Git
    // detection is.
    #[test]
    fn fsp_11_named_external_uses_generic_detection() {
        let _guard = env_lock();
        let root = make_clean_git_root();

        // Create AionUi with embedded .git.
        let aion = root.join("AionUi");
        fs::create_dir_all(&aion).expect("aion dir");
        git(&["init", "--quiet"], &aion);
        fs::write(aion.join("stub.txt"), "external\n").expect("stub");
        git(&["add", "."], &aion);
        git(&["commit", "-m", "ext", "--quiet"], &aion);

        let result = run_full_review_preflight(&root, Path::new("."));

        assert_eq!(result.decision, PreflightDecision::Block);
        match &result.block_reason {
            Some(BlockReason::ExternalRepositoryDetected { dir_name }) => {
                assert_eq!(dir_name, "AionUi");
            }
            other => panic!("expected ExternalRepositoryDetected, got {other:?}"),
        }

        fs::remove_dir_all(&root).expect("cleanup");
    }

    // FSP-12: Ignored external/ is not automatically allowed/excluded.
    #[test]
    fn fsp_12_ignored_external_not_auto_allowed() {
        let _guard = env_lock();
        let root = make_clean_git_root();

        // Create an ignored top-level dir.
        fs::write(root.join(".gitignore"), "external/\n").expect("gitignore");
        git(&["add", ".gitignore"], &root);
        git(&["commit", "-m", "gitignore", "--quiet"], &root);

        fs::create_dir_all(root.join("external")).expect("external dir");
        fs::write(root.join("external").join("data.txt"), "data\n").expect("data");

        let result = run_full_review_preflight(&root, Path::new("."));

        assert_eq!(result.decision, PreflightDecision::Block);
        match &result.block_reason {
            Some(BlockReason::IgnoredTopLevelNotPolicyMatched { dir_name }) => {
                assert_eq!(dir_name, "external");
            }
            other => panic!("expected IgnoredTopLevelNotPolicyMatched, got {other:?}"),
        }

        fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn pep_003_blocks_unapproved_sego_child() {
        let _guard = env_lock();
        let root = make_clean_git_root();
        fs::create_dir_all(root.join(".sego").join("reviews")).expect("reviews");
        fs::write(root.join(".sego").join("reviews").join("r.json"), "{}\n").expect("review");
        fs::write(root.join(".sego").join("dev.toml"), "private = true\n").expect("private");

        let result = run_full_review_preflight(&root, Path::new("."));

        assert_eq!(result.decision, PreflightDecision::Block);
        assert!(matches!(
            result.block_reason,
            Some(BlockReason::PolicyEntryContainsUnmatchedContent { ref path }) if path == ".sego/dev.toml"
        ));
        fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn declared_subtree_does_not_classify_root_siblings() {
        let _guard = env_lock();
        let root = make_clean_git_root();
        let external = root.join("external");
        fs::create_dir_all(&external).expect("external");
        git(&["init", "--quiet"], &external);
        fs::write(external.join("stub.txt"), "external\n").expect("stub");

        let result = run_full_review_preflight(&root, Path::new("src"));

        assert_eq!(result.decision, PreflightDecision::Allow);
        assert!(result.exclusions.is_empty());
        fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn git_path_untracked_existing_entry_blocks() {
        let _guard = env_lock();
        let root = make_clean_git_root();
        fs::write(root.join("notes.md"), "untracked\n").expect("notes");

        let result = run_git_path_preflight(&root, Path::new("notes.md"));

        assert_eq!(result.decision, PreflightDecision::Block);
        assert!(matches!(
            result.block_reason,
            Some(BlockReason::GitPathNotTracked { ref path }) if path == "notes.md"
        ));
        fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn pep_004_wrong_extension_does_not_become_exclusion() {
        let _guard = env_lock();
        let root = make_clean_git_root();
        fs::write(root.join("SEGO_SYNC_private.md"), "private\n").expect("private");

        let result = run_full_review_preflight(&root, Path::new("."));

        assert_eq!(result.decision, PreflightDecision::Block);
        assert!(matches!(
            result.block_reason,
            Some(BlockReason::UntrackedTopLevelEntry { ref entry_name }) if entry_name == "SEGO_SYNC_private.md"
        ));
        fs::remove_dir_all(&root).expect("cleanup");
    }

    // Extra: PEP-004 prefix matching (SEGO_SYNC_*.txt, SEGO_TASK_*.md).
    #[test]
    fn pep_004_prefix_patterns_match_direct_top_level_only() {
        assert!(matches_pep_004("SEGO_SYNC_abc.txt"));
        assert!(matches_pep_004("SEGO_TASK_001.md"));
        assert!(!matches_pep_004("SEGO_SYNC_abc.md"));
        assert!(!matches_pep_004("SEGO_TASK_abc.txt"));
        assert!(!matches_pep_004("SEGO_SYNC_abc.txt.bak"));
        assert!(!matches_pep_004("sub/SEGO_SYNC_abc.txt"));
        assert!(matches_pep_004(".claude"));
        assert!(matches_pep_004("CLAUDE.md"));
        assert!(matches_pep_004("ZCODE.md"));
    }

    // Extra: PEP-005 annotation names.
    #[test]
    fn pep_005_annotation_names() {
        assert!(is_pep_005_annotation("AionUi"));
        assert!(is_pep_005_annotation("AionUi_fork"));
        assert!(is_pep_005_annotation("Coolearn"));
        assert!(is_pep_005_annotation("hermes_repo"));
        assert!(is_pep_005_annotation("hermes-agent-review"));
        assert!(!is_pep_005_annotation("src"));
    }

    // Extra: block reason codes are stable.
    #[test]
    fn block_reason_codes_are_stable() {
        assert_eq!(BlockReason::NonGitTarget.code(), "non_git_target");
        assert_eq!(
            BlockReason::UntrackedTopLevelDirectory { dir_name: "x".into() }.code(),
            "untracked_top_level_directory"
        );
        assert_eq!(
            BlockReason::ExternalRepositoryDetected { dir_name: "y".into() }.code(),
            "external_repository_detected"
        );
    }
}
