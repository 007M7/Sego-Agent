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
            Self::ExternalRepositoryDetected { .. } => "external_repository_detected",
            Self::IgnoredTopLevelNotPolicyMatched { .. } => "ignored_top_level_not_policy_matched",
            Self::TargetNotFound => "target_not_found",
            Self::TargetNotDirectory => "target_not_directory",
            Self::GitPathOutsideRoot => "git_path_outside_root",
            Self::GitPathNotFound { .. } => "git_path_not_found",
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
            Self::ExternalRepositoryDetected { dir_name } => format!("external repository candidate `{dir_name}` detected via embedded-Git probe; block by default"),
            Self::IgnoredTopLevelNotPolicyMatched { dir_name } => format!("git-ignored top-level `{dir_name}` is not matched by an accepted policy rule; .gitignore is not an allow signal"),
            Self::TargetNotFound => "target path does not exist".to_string(),
            Self::TargetNotDirectory => "full review target is not a directory".to_string(),
            Self::GitPathOutsideRoot => "Git path resolves outside the Git root".to_string(),
            Self::GitPathNotFound { path } => format!("Git path `{path}` does not exist"),
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

/// PEP-001: managed build output — exact top-level directories.
const PEP_001_DIRS: &[&str] = &["target", "build", "dist", "coverage"];

/// PEP-002: editor/IDE local state — exact top-level directories.
const PEP_002_DIRS: &[&str] = &[".idea", ".vscode", ".vs"];

/// PEP-003: runtime/session/recovery output — registered paths.
const PEP_003_DIRS: &[&str] = &[".sego", ".port_sessions", ".sandbox-home", ".sandbox-tmp"];

/// PEP-004: private credentials/config — exact declared-root top-level entries.
const PEP_004_EXACT_ENTRIES: &[&str] = &[".claude", ".claw", "CLAUDE.md", "ZCODE.md"];

/// PEP-004: prefix patterns for direct top-level entries only (no recursion).
const PEP_004_PREFIXES: &[&str] = &["SEGO_SYNC_", "SEGO_TASK_"];

/// PEP-005: external repository risk-annotation name prefixes (never exclusion).
const PEP_005_PREFIXES: &[&str] = &["AionUi", "Coolearn", "hermes"];

/// Check if a top-level entry name matches PEP-001 (managed build output).
#[must_use]
fn matches_pep_001(name: &str) -> bool {
    PEP_001_DIRS.contains(&name)
}

/// Check if a top-level entry name matches PEP-002 (editor/IDE state).
#[must_use]
fn matches_pep_002(name: &str) -> bool {
    PEP_002_DIRS.contains(&name)
}

/// Check if a top-level entry name matches PEP-003 (runtime/session/recovery).
#[must_use]
fn matches_pep_003(name: &str) -> bool {
    PEP_003_DIRS.contains(&name)
}

/// Check if a top-level entry name matches PEP-004 (private credentials/config).
///
/// PEP-004 matches only direct top-level entries of the declared review root.
/// `SEGO_SYNC_*.txt` and `SEGO_TASK_*.md` match only direct top-level entries,
/// never recursing into child directories and never matching nested basenames.
#[must_use]
fn matches_pep_004(name: &str) -> bool {
    if PEP_004_EXACT_ENTRIES.contains(&name) {
        return true;
    }
    // Prefix patterns: SEGO_SYNC_*.txt, SEGO_TASK_*.md — direct top-level only.
    for prefix in PEP_004_PREFIXES {
        if let Some(rest) = name.strip_prefix(prefix) {
            if (rest.ends_with(".txt") || rest.ends_with(".md")) && !rest.is_empty() {
                return true;
            }
        }
    }
    false
}

/// Check if a top-level entry name is a PEP-005 risk annotation (never exclusion).
#[must_use]
fn is_pep_005_annotation(name: &str) -> bool {
    PEP_005_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

/// Try to match a top-level entry against the accepted PEP policy.
/// Returns the exclusion record if matched.
#[must_use]
fn try_match_pep_policy(name: &str) -> Option<PreflightExclusion> {
    let v = PREFLIGHT_POLICY_VERSION.to_string();
    if matches_pep_001(name) {
        return Some(PreflightExclusion {
            policy_version: v,
            rule_id: "PEP-001".to_string(),
            category: "managed_build_output".to_string(),
            relative_path: name.to_string(),
            owner: "Product Engineering".to_string(),
            rationale: "exact top-level build output directory".to_string(),
        });
    }
    if matches_pep_002(name) {
        return Some(PreflightExclusion {
            policy_version: v,
            rule_id: "PEP-002".to_string(),
            category: "editor_ide_local_state".to_string(),
            relative_path: name.to_string(),
            owner: "Product Engineering".to_string(),
            rationale: "exact top-level editor/IDE state directory".to_string(),
        });
    }
    if matches_pep_003(name) {
        return Some(PreflightExclusion {
            policy_version: v,
            rule_id: "PEP-003".to_string(),
            category: "runtime_session_recovery".to_string(),
            relative_path: name.to_string(),
            owner: "Sego Master".to_string(),
            rationale: "registered runtime/session/recovery output path".to_string(),
        });
    }
    if matches_pep_004(name) {
        return Some(PreflightExclusion {
            policy_version: v,
            rule_id: "PEP-004".to_string(),
            category: "private_credentials_config".to_string(),
            relative_path: name.to_string(),
            owner: "Founder/Sego Master".to_string(),
            rationale: "declared-root direct top-level private/config entry".to_string(),
        });
    }
    None
}

// ---------------------------------------------------------------------------
// Git helpers (reused from main, but isolated for testability)
// ---------------------------------------------------------------------------

/// Run a git command in `cwd` and return stdout. Returns Err on non-zero exit.
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

/// Check if a path is inside a Git worktree.
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

/// Find the Git root (toplevel) for a path inside a worktree.
fn git_toplevel(cwd: &Path) -> Option<PathBuf> {
    git_output(cwd, &["rev-parse", "--show-toplevel"]).ok().and_then(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(PathBuf::from(trimmed))
        }
    })
}

/// Get untracked top-level directories via `git status --porcelain --untracked-files=normal`.
fn untracked_top_level_dirs(repo_root: &Path) -> Vec<String> {
    let status = git_output(repo_root, &["status", "--porcelain", "--untracked-files=normal"])
        .unwrap_or_default();

    let mut dirs: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for line in status.lines() {
        let raw = line.trim_start();
        if !raw.starts_with("?? ") {
            continue;
        }
        let path_str = raw["?? ".len()..].trim();
        if path_str.is_empty() {
            continue;
        }
        // Strip surrounding quotes git may add.
        let path_str =
            path_str.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(path_str);
        // Trailing slash indicates a directory.
        let dir_name = path_str.trim_end_matches('/').trim_end_matches('\\');
        if dir_name.is_empty() {
            continue;
        }
        // Only top-level: no path separator.
        if dir_name.contains('/') || dir_name.contains('\\') {
            continue;
        }
        if seen.insert(dir_name.to_string()) {
            dirs.push(dir_name.to_string());
        }
    }
    dirs.sort();
    dirs
}

/// Check if a top-level directory contains a `.git` entry (embedded Git repo probe).
///
/// This is a bounded probe: it only checks for the existence of `.git` inside
/// the immediate top-level child. It does NOT recurse or enumerate content.
fn has_embedded_git(repo_root: &Path, dir_name: &str) -> bool {
    let git_path = repo_root.join(dir_name).join(".git");
    git_path.exists()
}

/// Get ignored top-level directories via `git status --porcelain --ignored`.
///
/// Ignored directories are not visible in normal `git status`. We need to
/// detect them separately so that FSP-12 can block ignored external content
/// that is not matched by an accepted policy rule.
fn ignored_top_level_dirs(repo_root: &Path) -> Vec<String> {
    let status = git_output(repo_root, &["status", "--porcelain", "--ignored"]).unwrap_or_default();

    let mut dirs: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for line in status.lines() {
        let raw = line.trim_start();
        // Ignored entries start with "!! ".
        if !raw.starts_with("!! ") {
            continue;
        }
        let path_str = raw["!! ".len()..].trim();
        if path_str.is_empty() {
            continue;
        }
        let path_str =
            path_str.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(path_str);
        let dir_name = path_str.trim_end_matches('/').trim_end_matches('\\');
        if dir_name.is_empty() {
            continue;
        }
        // Only top-level: no path separator.
        if dir_name.contains('/') || dir_name.contains('\\') {
            continue;
        }
        if seen.insert(dir_name.to_string()) {
            dirs.push(dir_name.to_string());
        }
    }
    dirs.sort();
    dirs
}

/// Check if a top-level path is git-ignored.
fn is_git_ignored(repo_root: &Path, name: &str) -> bool {
    git_output(repo_root, &["check-ignore", name]).map(|s| !s.trim().is_empty()).unwrap_or(false)
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

    // 1. Resolve target to absolute path.
    let resolved_raw = if target.is_absolute() { target.to_path_buf() } else { cwd.join(target) };

    // 2. Check existence before canonicalize (canonicalize fails on missing).
    if !resolved_raw.exists() {
        events.push(ClassificationEvent {
            event_type: "target_check".to_string(),
            detail: format!("target does not exist: {}", resolved_raw.display()),
        });
        return PreflightResult::block(BlockReason::TargetNotFound, None, None, events);
    }

    // 3. Canonicalize.
    let resolved = match std::fs::canonicalize(&resolved_raw) {
        Ok(p) => p,
        Err(e) => {
            events.push(ClassificationEvent {
                event_type: "canonicalize_failed".to_string(),
                detail: format!("cannot canonicalize {}: {e}", resolved_raw.display()),
            });
            return PreflightResult::block(BlockReason::TargetNotFound, None, None, events);
        }
    };

    events.push(ClassificationEvent {
        event_type: "resolved_target".to_string(),
        detail: resolved.display().to_string(),
    });

    // 4. Must be a directory.
    if !resolved.is_dir() {
        events.push(ClassificationEvent {
            event_type: "target_not_directory".to_string(),
            detail: resolved.display().to_string(),
        });
        return PreflightResult::block(
            BlockReason::TargetNotDirectory,
            Some(resolved),
            None,
            events,
        );
    }

    // 5. Must be a Git worktree (P0: no filesystem-only mode).
    if !is_git_worktree_at(&resolved) {
        events.push(ClassificationEvent {
            event_type: "non_git_target".to_string(),
            detail: resolved.display().to_string(),
        });
        return PreflightResult::block(BlockReason::NonGitTarget, Some(resolved), None, events);
    }

    // 6. Find Git root.
    let git_root = git_toplevel(&resolved);
    let git_root = match git_root {
        Some(root) => {
            // Canonicalize so path comparisons use the same format as resolved.
            let canonical = std::fs::canonicalize(&root).unwrap_or(root);
            events.push(ClassificationEvent {
                event_type: "git_root".to_string(),
                detail: canonical.display().to_string(),
            });
            canonical
        }
        None => {
            return PreflightResult::block(BlockReason::NonGitTarget, Some(resolved), None, events);
        }
    };

    // 7. If target is an absolute drive-rooted path outside the current worktree's
    //    Git root, check whether it's the same as the current cwd's Git root.
    //    FSP-08: drive-rooted target outside current worktree blocks.
    if target.is_absolute() {
        if let Some(cwd_git_root) = git_toplevel(cwd) {
            let cwd_git_root = std::fs::canonicalize(&cwd_git_root).unwrap_or(cwd_git_root);
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
        }
    }

    // 8. Inspect untracked top-level directories.
    let untracked = untracked_top_level_dirs(&git_root);
    let mut exclusions: Vec<PreflightExclusion> = Vec::new();

    for dir_name in &untracked {
        // PEP-005: external repository risk annotation — always block, never exclusion.
        if is_pep_005_annotation(dir_name) {
            // Use generic embedded-Git detection as decisive evidence.
            if has_embedded_git(&git_root, dir_name) {
                events.push(ClassificationEvent {
                    event_type: "external_repository_detected".to_string(),
                    detail: format!("{dir_name} contains embedded .git"),
                });
                return PreflightResult::block(
                    BlockReason::ExternalRepositoryDetected { dir_name: dir_name.clone() },
                    Some(resolved),
                    Some(git_root),
                    events,
                );
            }
            // PEP-005 name without embedded .git is still a risk annotation;
            // block as unknown untracked top-level.
            events.push(ClassificationEvent {
                event_type: "pep005_annotation_no_git".to_string(),
                detail: dir_name.clone(),
            });
            return PreflightResult::block(
                BlockReason::UntrackedTopLevelDirectory { dir_name: dir_name.clone() },
                Some(resolved),
                Some(git_root),
                events,
            );
        }

        // Try PEP-001..004 policy match.
        if let Some(excl) = try_match_pep_policy(dir_name) {
            events.push(ClassificationEvent {
                event_type: "policy_exclusion".to_string(),
                detail: format!("{} matched {}", dir_name, excl.rule_id),
            });
            exclusions.push(excl);
            continue;
        }

        // Generic embedded-Git probe for any untracked top-level directory (FSP-02).
        if has_embedded_git(&git_root, dir_name) {
            events.push(ClassificationEvent {
                event_type: "external_repository_detected".to_string(),
                detail: format!("{dir_name} contains embedded .git (generic probe)"),
            });
            return PreflightResult::block(
                BlockReason::ExternalRepositoryDetected { dir_name: dir_name.clone() },
                Some(resolved),
                Some(git_root),
                events,
            );
        }

        // Check if git-ignored (FSP-12: ignore is not an allow signal).
        if is_git_ignored(&git_root, dir_name) {
            events.push(ClassificationEvent {
                event_type: "ignored_not_policy_matched".to_string(),
                detail: dir_name.clone(),
            });
            return PreflightResult::block(
                BlockReason::IgnoredTopLevelNotPolicyMatched { dir_name: dir_name.clone() },
                Some(resolved),
                Some(git_root),
                events,
            );
        }

        // Unknown untracked top-level directory (FSP-03).
        events.push(ClassificationEvent {
            event_type: "untracked_top_level".to_string(),
            detail: dir_name.clone(),
        });
        return PreflightResult::block(
            BlockReason::UntrackedTopLevelDirectory { dir_name: dir_name.clone() },
            Some(resolved),
            Some(git_root),
            events,
        );
    }

    // 9. Inspect ignored top-level directories (FSP-12).
    //    Ignored dirs are not visible in normal git status, so we check them
    //    separately. An ignored dir that is not matched by an accepted PEP
    //    policy rule blocks - .gitignore is not an allow signal.
    let ignored = ignored_top_level_dirs(&git_root);
    for dir_name in &ignored {
        // PEP-005: external repository risk annotation - always block.
        if is_pep_005_annotation(dir_name) {
            if has_embedded_git(&git_root, dir_name) {
                events.push(ClassificationEvent {
                    event_type: "external_repository_detected_ignored".to_string(),
                    detail: format!("{dir_name} contains embedded .git (ignored)"),
                });
                return PreflightResult::block(
                    BlockReason::ExternalRepositoryDetected { dir_name: dir_name.clone() },
                    Some(resolved),
                    Some(git_root),
                    events,
                );
            }
        }

        // Try PEP-001..004 policy match. If matched, it's an exclusion.
        if try_match_pep_policy(dir_name).is_some() {
            events.push(ClassificationEvent {
                event_type: "ignored_policy_matched".to_string(),
                detail: dir_name.clone(),
            });
            continue;
        }

        // Ignored but not policy-matched: block (FSP-12).
        events.push(ClassificationEvent {
            event_type: "ignored_not_policy_matched".to_string(),
            detail: dir_name.clone(),
        });
        return PreflightResult::block(
            BlockReason::IgnoredTopLevelNotPolicyMatched { dir_name: dir_name.clone() },
            Some(resolved),
            Some(git_root),
            events,
        );
    }

    // All checks passed: allow.
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

    // FSP-10: Unsupported/incompatible scope block before filesystem access.
    #[test]
    fn fsp_10_unsupported_scope_block() {
        let _guard = env_lock();
        let root = make_clean_git_root();

        // The parser already rejects unknown flags, but we test that a
        // non-existent target is blocked before any snapshot.
        let result = run_full_review_preflight(&root, Path::new("does-not-exist-xyz"));

        assert_eq!(result.decision, PreflightDecision::Block);
        assert!(!result.snapshot_started);
        match &result.block_reason {
            Some(BlockReason::TargetNotFound) => {}
            other => panic!("expected TargetNotFound, got {other:?}"),
        }

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

    // Extra: PEP-004 prefix matching (SEGO_SYNC_*.txt, SEGO_TASK_*.md).
    #[test]
    fn pep_004_prefix_patterns_match_direct_top_level_only() {
        assert!(matches_pep_004("SEGO_SYNC_abc.txt"));
        assert!(matches_pep_004("SEGO_TASK_001.md"));
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
