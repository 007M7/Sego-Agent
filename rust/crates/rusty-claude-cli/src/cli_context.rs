//! Gathering the context the reports describe.
//!
//! Moved out of `main.rs` (DEV-STRUCT-01 step 3, the tail of §1.23 - the
//! collectors that slice left behind). The boundary came from the measured
//! dependency closure: **9 items, 140 lines** - the two collectors and the
//! git-status reading they share.
//!
//! This module is the I/O half of the report split, and saying so is the point:
//! `cli_reports` is deliberately pure (given values, return text), and these are
//! the pieces that run `git`, read config and look at the filesystem. Keeping
//! them apart is what makes the purity of the other module checkable at a
//! glance instead of a claim.
//!
//! The view types themselves stay in `cli_reports`, because they are what the
//! report is written against. Their fields are `pub(crate)` for the same reason:
//! the collectors here build them and the formatters there read them, so a
//! boundary that splits those two has to widen the fields.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use runtime::PluginLifecycle;

use crate::{
    default_date, resolve_sandbox_status, ConfigLoader, GitWorkspaceSummary, ProjectContext,
    StatusContext, WorkspaceContext,
};
pub(crate) fn parse_git_status_metadata(status: Option<&str>) -> (Option<PathBuf>, Option<String>) {
    parse_git_status_metadata_for(
        &env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        status,
    )
}

pub(crate) fn parse_git_status_branch(status: Option<&str>) -> Option<String> {
    let status = status?;
    let first_line = status.lines().next()?;
    let line = first_line.strip_prefix("## ")?;
    if line.starts_with("HEAD") {
        return Some("detached HEAD".to_string());
    }
    let branch = line.split(['.', ' ']).next().unwrap_or_default().trim();
    if branch.is_empty() {
        None
    } else {
        Some(branch.to_string())
    }
}

pub(crate) fn parse_git_workspace_summary(status: Option<&str>) -> GitWorkspaceSummary {
    let mut summary = GitWorkspaceSummary::default();
    let Some(status) = status else {
        return summary;
    };

    for line in status.lines() {
        if line.starts_with("## ") || line.trim().is_empty() {
            continue;
        }

        summary.changed_files += 1;
        let mut chars = line.chars();
        let index_status = chars.next().unwrap_or(' ');
        let worktree_status = chars.next().unwrap_or(' ');

        if index_status == '?' && worktree_status == '?' {
            summary.untracked_files += 1;
            continue;
        }

        if index_status != ' ' {
            summary.staged_files += 1;
        }
        if worktree_status != ' ' {
            summary.unstaged_files += 1;
        }
        if (matches!(index_status, 'U' | 'A') && matches!(worktree_status, 'U' | 'A'))
            || index_status == 'U'
            || worktree_status == 'U'
        {
            summary.conflicted_files += 1;
        }
    }

    summary
}

pub(crate) fn resolve_git_branch_for(cwd: &Path) -> Option<String> {
    let branch = run_git_capture_in(cwd, &["branch", "--show-current"])?;
    let branch = branch.trim();
    if !branch.is_empty() {
        return Some(branch.to_string());
    }

    let fallback = run_git_capture_in(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let fallback = fallback.trim();
    if fallback.is_empty() {
        None
    } else if fallback == "HEAD" {
        Some("detached HEAD".to_string())
    } else {
        Some(fallback.to_string())
    }
}

pub(crate) fn run_git_capture_in(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git").args(args).current_dir(cwd).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

pub(crate) fn find_git_root_in(cwd: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()?;
    if !output.status.success() {
        return Err("not a git repository".into());
    }
    let path = String::from_utf8(output.stdout)?.trim().to_string();
    if path.is_empty() {
        return Err("empty git root".into());
    }
    Ok(PathBuf::from(path))
}

pub(crate) fn parse_git_status_metadata_for(
    cwd: &Path,
    status: Option<&str>,
) -> (Option<PathBuf>, Option<String>) {
    let branch = resolve_git_branch_for(cwd).or_else(|| parse_git_status_branch(status));
    let project_root = find_git_root_in(cwd).ok();
    (project_root, branch)
}

pub(crate) fn status_context(
    session_path: Option<&Path>,
) -> Result<StatusContext, Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let loader = ConfigLoader::default_for(&cwd);
    let discovered_config_files = loader.discover().len();
    let runtime_config = loader.load()?;
    let project_context = ProjectContext::discover_with_git(&cwd, default_date())?;
    let (project_root, git_branch) =
        parse_git_status_metadata(project_context.git_status.as_deref());
    let git_summary = parse_git_workspace_summary(project_context.git_status.as_deref());
    let sandbox_status = resolve_sandbox_status(runtime_config.sandbox(), &cwd);
    Ok(StatusContext {
        cwd,
        session_path: session_path.map(Path::to_path_buf),
        loaded_config_files: runtime_config.loaded_entries().len(),
        discovered_config_files,
        memory_file_count: project_context.instruction_files.len(),
        project_root,
        git_branch,
        git_summary,
        sandbox_status,
    })
}

pub(crate) fn workspace_context() -> Result<WorkspaceContext, Box<dyn std::error::Error>> {
    let cwd = env::current_dir()?;
    let loader = ConfigLoader::default_for(&cwd);
    let runtime_config = loader.load().unwrap_or_else(|_| runtime::RuntimeConfig::empty());
    let sandbox_status = resolve_sandbox_status(runtime_config.sandbox(), &cwd);
    Ok(WorkspaceContext {
        project_root: find_git_root_in(&cwd).ok(),
        session_dir: cwd.join(".claw").join("sessions"),
        recovery_dir: runtime::recovery::recovery_dir(&cwd),
        cwd,
        sandbox_status,
    })
}
