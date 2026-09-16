use std::cmp::Reverse;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use glob::Pattern;
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

/// Maximum file size that can be read (10 MB).
const MAX_READ_SIZE: u64 = 10 * 1024 * 1024;

/// Maximum file size that can be written (10 MB).
const MAX_WRITE_SIZE: usize = 10 * 1024 * 1024;

/// Check whether a file appears to contain binary content by examining
/// the first chunk for NUL bytes.
fn is_binary_file(path: &Path) -> io::Result<bool> {
    use std::io::Read;
    let mut file = fs::File::open(path)?;
    let mut buffer = [0u8; 8192];
    let bytes_read = file.read(&mut buffer)?;
    Ok(buffer[..bytes_read].contains(&0))
}

/// Validate that a resolved path stays within the given workspace root.
/// Returns the canonical path on success, or an error if the path escapes
/// the workspace boundary (e.g. via `../` traversal or symlink).
///
/// Callers must pass an already-normalised path: [`normalize_path`] and
/// [`normalize_path_allow_missing`] canonicalise the file or its nearest
/// existing parent, which resolves symlinks and junctions before the
/// comparison.
fn validate_workspace_boundary(resolved: &Path, workspace_root: &Path) -> io::Result<()> {
    if !resolved.starts_with(workspace_root) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "path {} escapes workspace boundary {}",
                resolved.display(),
                workspace_root.display()
            ),
        ));
    }
    Ok(())
}

/// Workspace root that confines the file operations exposed as tools.
///
/// The tools layer already treats the process working directory as the
/// workspace - todos, the agents directory and local settings all resolve
/// against it - so confinement uses the same root rather than inventing a
/// second one. Tests that operate on a scratch tree therefore `set_current_dir`
/// into it before calling these functions.
fn workspace_root() -> io::Result<PathBuf> {
    let root = std::env::current_dir()?;
    Ok(root.canonicalize().unwrap_or(root))
}

/// Text payload returned by file-reading operations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TextFilePayload {
    #[serde(rename = "filePath")]
    pub file_path: String,
    pub content: String,
    #[serde(rename = "numLines")]
    pub num_lines: usize,
    #[serde(rename = "startLine")]
    pub start_line: usize,
    #[serde(rename = "totalLines")]
    pub total_lines: usize,
}

/// Output envelope for the `read_file` tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadFileOutput {
    #[serde(rename = "type")]
    pub kind: String,
    pub file: TextFilePayload,
}

/// Structured patch hunk emitted by write and edit operations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuredPatchHunk {
    #[serde(rename = "oldStart")]
    pub old_start: usize,
    #[serde(rename = "oldLines")]
    pub old_lines: usize,
    #[serde(rename = "newStart")]
    pub new_start: usize,
    #[serde(rename = "newLines")]
    pub new_lines: usize,
    pub lines: Vec<String>,
}

/// Output envelope for full-file write operations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriteFileOutput {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "filePath")]
    pub file_path: String,
    pub content: String,
    #[serde(rename = "structuredPatch")]
    pub structured_patch: Vec<StructuredPatchHunk>,
    #[serde(rename = "originalFile")]
    pub original_file: Option<String>,
    #[serde(rename = "gitDiff")]
    pub git_diff: Option<serde_json::Value>,
}

/// Output envelope for targeted string-replacement edits.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditFileOutput {
    #[serde(rename = "filePath")]
    pub file_path: String,
    #[serde(rename = "oldString")]
    pub old_string: String,
    #[serde(rename = "newString")]
    pub new_string: String,
    #[serde(rename = "originalFile")]
    pub original_file: String,
    #[serde(rename = "structuredPatch")]
    pub structured_patch: Vec<StructuredPatchHunk>,
    #[serde(rename = "userModified")]
    pub user_modified: bool,
    #[serde(rename = "replaceAll")]
    pub replace_all: bool,
    #[serde(rename = "gitDiff")]
    pub git_diff: Option<serde_json::Value>,
}

/// Result of a glob-based filename search.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GlobSearchOutput {
    #[serde(rename = "durationMs")]
    pub duration_ms: u128,
    #[serde(rename = "numFiles")]
    pub num_files: usize,
    pub filenames: Vec<String>,
    pub truncated: bool,
}

/// Parameters accepted by the grep-style search tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrepSearchInput {
    pub pattern: String,
    pub path: Option<String>,
    pub glob: Option<String>,
    #[serde(rename = "output_mode")]
    pub output_mode: Option<String>,
    #[serde(rename = "-B")]
    pub before: Option<usize>,
    #[serde(rename = "-A")]
    pub after: Option<usize>,
    #[serde(rename = "-C")]
    pub context_short: Option<usize>,
    pub context: Option<usize>,
    #[serde(rename = "-n")]
    pub line_numbers: Option<bool>,
    #[serde(rename = "-i")]
    pub case_insensitive: Option<bool>,
    #[serde(rename = "type")]
    pub file_type: Option<String>,
    pub head_limit: Option<usize>,
    pub offset: Option<usize>,
    pub multiline: Option<bool>,
}

/// Result payload returned by the grep-style search tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrepSearchOutput {
    pub mode: Option<String>,
    #[serde(rename = "numFiles")]
    pub num_files: usize,
    pub filenames: Vec<String>,
    pub content: Option<String>,
    #[serde(rename = "numLines")]
    pub num_lines: Option<usize>,
    #[serde(rename = "numMatches")]
    pub num_matches: Option<usize>,
    #[serde(rename = "appliedLimit")]
    pub applied_limit: Option<usize>,
    #[serde(rename = "appliedOffset")]
    pub applied_offset: Option<usize>,
}

/// Reads a text file and returns a line-windowed payload.
///
/// The path must resolve inside the workspace root; absolute paths, `..`
/// traversal and symlink/junction escapes are rejected.
pub fn read_file(
    path: &str,
    offset: Option<usize>,
    limit: Option<usize>,
) -> io::Result<ReadFileOutput> {
    let absolute_path = normalize_path(path)?;
    validate_workspace_boundary(&absolute_path, &workspace_root()?)?;
    read_file_resolved(&absolute_path, offset, limit)
}

/// Read an already-validated path. Boundary enforcement is the caller's job.
fn read_file_resolved(
    absolute_path: &Path,
    offset: Option<usize>,
    limit: Option<usize>,
) -> io::Result<ReadFileOutput> {
    // Check file size before reading
    let metadata = fs::metadata(absolute_path)?;
    if metadata.len() > MAX_READ_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("file is too large ({} bytes, max {} bytes)", metadata.len(), MAX_READ_SIZE),
        ));
    }

    // Detect binary files
    if is_binary_file(absolute_path)? {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "file appears to be binary"));
    }

    let content = fs::read_to_string(absolute_path)?;
    let lines: Vec<&str> = content.lines().collect();
    let start_index = offset.unwrap_or(0).min(lines.len());
    let end_index =
        limit.map_or(lines.len(), |limit| start_index.saturating_add(limit).min(lines.len()));
    let selected = lines[start_index..end_index].join("\n");

    Ok(ReadFileOutput {
        kind: String::from("text"),
        file: TextFilePayload {
            file_path: absolute_path.to_string_lossy().into_owned(),
            content: selected,
            num_lines: end_index.saturating_sub(start_index),
            start_line: start_index.saturating_add(1),
            total_lines: lines.len(),
        },
    })
}

/// Replaces a file's contents and returns patch metadata.
///
/// The path must resolve inside the workspace root; a symlinked parent that
/// points outside it is rejected before anything is created.
pub fn write_file(path: &str, content: &str) -> io::Result<WriteFileOutput> {
    let absolute_path = normalize_path_allow_missing(path)?;
    validate_workspace_boundary(&absolute_path, &workspace_root()?)?;
    write_file_resolved(&absolute_path, content)
}

/// Write an already-validated path. Boundary enforcement is the caller's job.
fn write_file_resolved(absolute_path: &Path, content: &str) -> io::Result<WriteFileOutput> {
    if content.len() > MAX_WRITE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("content is too large ({} bytes, max {} bytes)", content.len(), MAX_WRITE_SIZE),
        ));
    }

    let original_file = fs::read_to_string(absolute_path).ok();
    if let Some(parent) = absolute_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(absolute_path, content)?;

    Ok(WriteFileOutput {
        kind: if original_file.is_some() { String::from("update") } else { String::from("create") },
        file_path: absolute_path.to_string_lossy().into_owned(),
        content: content.to_owned(),
        structured_patch: make_patch(original_file.as_deref().unwrap_or(""), content),
        original_file,
        git_diff: None,
    })
}

/// Performs an in-file string replacement and returns patch metadata.
///
/// The path must resolve inside the workspace root.
pub fn edit_file(
    path: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> io::Result<EditFileOutput> {
    let absolute_path = normalize_path(path)?;
    validate_workspace_boundary(&absolute_path, &workspace_root()?)?;
    edit_file_resolved(&absolute_path, old_string, new_string, replace_all)
}

/// Edit an already-validated path. Boundary enforcement is the caller's job.
fn edit_file_resolved(
    absolute_path: &Path,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
) -> io::Result<EditFileOutput> {
    let original_file = fs::read_to_string(absolute_path)?;
    if old_string == new_string {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "old_string and new_string must differ",
        ));
    }
    if !original_file.contains(old_string) {
        return Err(io::Error::new(io::ErrorKind::NotFound, "old_string not found in file"));
    }

    let updated = if replace_all {
        original_file.replace(old_string, new_string)
    } else {
        original_file.replacen(old_string, new_string, 1)
    };
    fs::write(absolute_path, &updated)?;

    Ok(EditFileOutput {
        file_path: absolute_path.to_string_lossy().into_owned(),
        old_string: old_string.to_owned(),
        new_string: new_string.to_owned(),
        original_file: original_file.clone(),
        structured_patch: make_patch(&original_file, &updated),
        user_modified: false,
        replace_all,
        git_diff: None,
    })
}

/// Expands a glob pattern and returns matching filenames.
pub fn glob_search(pattern: &str, path: Option<&str>) -> io::Result<GlobSearchOutput> {
    let started = Instant::now();
    let base_dir = path.map(normalize_path).transpose()?.unwrap_or(std::env::current_dir()?);
    let search_pattern = if Path::new(pattern).is_absolute() {
        pattern.to_owned()
    } else {
        base_dir.join(pattern).to_string_lossy().into_owned()
    };

    let mut matches = Vec::new();
    let entries = glob::glob(&search_pattern)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
    for entry in entries.flatten() {
        if entry.is_file() {
            matches.push(entry);
        }
    }

    matches.sort_by_key(|path| {
        fs::metadata(path).and_then(|metadata| metadata.modified()).ok().map(Reverse)
    });

    let truncated = matches.len() > 100;
    let filenames = matches
        .into_iter()
        .take(100)
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    Ok(GlobSearchOutput {
        duration_ms: started.elapsed().as_millis(),
        num_files: filenames.len(),
        filenames,
        truncated,
    })
}

/// Runs a regex search over workspace files with optional context lines.
pub fn grep_search(input: &GrepSearchInput) -> io::Result<GrepSearchOutput> {
    let base_path =
        input.path.as_deref().map(normalize_path).transpose()?.unwrap_or(std::env::current_dir()?);

    let regex = RegexBuilder::new(&input.pattern)
        .case_insensitive(input.case_insensitive.unwrap_or(false))
        .dot_matches_new_line(input.multiline.unwrap_or(false))
        .build()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;

    let glob_filter = input
        .glob
        .as_deref()
        .map(Pattern::new)
        .transpose()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;
    let file_type = input.file_type.as_deref();
    let output_mode =
        input.output_mode.clone().unwrap_or_else(|| String::from("files_with_matches"));
    let context = input.context.or(input.context_short).unwrap_or(0);

    let mut filenames = Vec::new();
    let mut content_lines = Vec::new();
    let mut total_matches = 0usize;

    for file_path in collect_search_files(&base_path)? {
        if !matches_optional_filters(&file_path, glob_filter.as_ref(), file_type) {
            continue;
        }

        let Ok(file_contents) = fs::read_to_string(&file_path) else {
            continue;
        };

        if output_mode == "count" {
            let count = regex.find_iter(&file_contents).count();
            if count > 0 {
                filenames.push(file_path.to_string_lossy().into_owned());
                total_matches += count;
            }
            continue;
        }

        let lines: Vec<&str> = file_contents.lines().collect();
        let mut matched_lines = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            if regex.is_match(line) {
                total_matches += 1;
                matched_lines.push(index);
            }
        }

        if matched_lines.is_empty() {
            continue;
        }

        filenames.push(file_path.to_string_lossy().into_owned());
        if output_mode == "content" {
            for index in matched_lines {
                let start = index.saturating_sub(input.before.unwrap_or(context));
                let end = (index + input.after.unwrap_or(context) + 1).min(lines.len());
                for (current, line) in lines.iter().enumerate().take(end).skip(start) {
                    let prefix = if input.line_numbers.unwrap_or(true) {
                        format!("{}:{}:", file_path.to_string_lossy(), current + 1)
                    } else {
                        format!("{}:", file_path.to_string_lossy())
                    };
                    content_lines.push(format!("{prefix}{line}"));
                }
            }
        }
    }

    let (filenames, applied_limit, applied_offset) =
        apply_limit(filenames, input.head_limit, input.offset);
    let content_output = if output_mode == "content" {
        let (lines, limit, offset) = apply_limit(content_lines, input.head_limit, input.offset);
        return Ok(GrepSearchOutput {
            mode: Some(output_mode),
            num_files: filenames.len(),
            filenames,
            num_lines: Some(lines.len()),
            content: Some(lines.join("\n")),
            num_matches: None,
            applied_limit: limit,
            applied_offset: offset,
        });
    } else {
        None
    };

    Ok(GrepSearchOutput {
        mode: Some(output_mode.clone()),
        num_files: filenames.len(),
        filenames,
        content: content_output,
        num_lines: None,
        num_matches: (output_mode == "count").then_some(total_matches),
        applied_limit,
        applied_offset,
    })
}

fn collect_search_files(base_path: &Path) -> io::Result<Vec<PathBuf>> {
    if base_path.is_file() {
        return Ok(vec![base_path.to_path_buf()]);
    }

    let mut files = Vec::new();
    for entry in WalkDir::new(base_path) {
        let entry = entry.map_err(|error| io::Error::other(error.to_string()))?;
        if entry.file_type().is_file() {
            files.push(entry.path().to_path_buf());
        }
    }
    Ok(files)
}

fn matches_optional_filters(
    path: &Path,
    glob_filter: Option<&Pattern>,
    file_type: Option<&str>,
) -> bool {
    if let Some(glob_filter) = glob_filter {
        let path_string = path.to_string_lossy();
        if !glob_filter.matches(&path_string) && !glob_filter.matches_path(path) {
            return false;
        }
    }

    if let Some(file_type) = file_type {
        let extension = path.extension().and_then(|extension| extension.to_str());
        if extension != Some(file_type) {
            return false;
        }
    }

    true
}

fn apply_limit<T>(
    items: Vec<T>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> (Vec<T>, Option<usize>, Option<usize>) {
    let offset_value = offset.unwrap_or(0);
    let mut items = items.into_iter().skip(offset_value).collect::<Vec<_>>();
    let explicit_limit = limit.unwrap_or(250);
    if explicit_limit == 0 {
        return (items, None, (offset_value > 0).then_some(offset_value));
    }

    let truncated = items.len() > explicit_limit;
    items.truncate(explicit_limit);
    (items, truncated.then_some(explicit_limit), (offset_value > 0).then_some(offset_value))
}

fn make_patch(original: &str, updated: &str) -> Vec<StructuredPatchHunk> {
    let mut lines = Vec::new();
    for line in original.lines() {
        lines.push(format!("-{line}"));
    }
    for line in updated.lines() {
        lines.push(format!("+{line}"));
    }

    vec![StructuredPatchHunk {
        old_start: 1,
        old_lines: original.lines().count(),
        new_start: 1,
        new_lines: updated.lines().count(),
        lines,
    }]
}

fn normalize_path(path: &str) -> io::Result<PathBuf> {
    let candidate = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        std::env::current_dir()?.join(path)
    };
    candidate.canonicalize()
}

fn normalize_path_allow_missing(path: &str) -> io::Result<PathBuf> {
    let candidate = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        std::env::current_dir()?.join(path)
    };

    if let Ok(canonical) = candidate.canonicalize() {
        return Ok(canonical);
    }

    // The target may not exist yet. Walk up to the nearest existing ancestor,
    // canonicalise that, then re-append the missing tail.
    //
    // Re-appending matters for boundary checks: `canonicalize` and
    // `current_dir` do not return paths on the same basis on Windows
    // (`canonicalize` adds a `\\?\` prefix), so falling back to the raw parent
    // would compare a prefixed root against an unprefixed path and reject a
    // target that is really inside the workspace.
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cursor: &Path = &candidate;
    while let Some(parent) = cursor.parent() {
        if let Some(name) = cursor.file_name() {
            tail.push(name.to_os_string());
        }
        if let Ok(canonical_parent) = parent.canonicalize() {
            let mut resolved = canonical_parent;
            for part in tail.iter().rev() {
                resolved.push(part);
            }
            return Ok(resolved);
        }
        cursor = parent;
    }

    Ok(candidate)
}

/// Read a file with workspace boundary enforcement.
#[allow(dead_code)]
pub fn read_file_in_workspace(
    path: &str,
    offset: Option<usize>,
    limit: Option<usize>,
    workspace_root: &Path,
) -> io::Result<ReadFileOutput> {
    let absolute_path = normalize_path(path)?;
    let canonical_root =
        workspace_root.canonicalize().unwrap_or_else(|_| workspace_root.to_path_buf());
    validate_workspace_boundary(&absolute_path, &canonical_root)?;
    // The path is already resolved and checked here, so call the inner body
    // rather than the public entry that would re-check against the cwd.
    read_file_resolved(&absolute_path, offset, limit)
}

/// Write a file with workspace boundary enforcement.
#[allow(dead_code)]
pub fn write_file_in_workspace(
    path: &str,
    content: &str,
    workspace_root: &Path,
) -> io::Result<WriteFileOutput> {
    let absolute_path = normalize_path_allow_missing(path)?;
    let canonical_root =
        workspace_root.canonicalize().unwrap_or_else(|_| workspace_root.to_path_buf());
    validate_workspace_boundary(&absolute_path, &canonical_root)?;
    write_file_resolved(&absolute_path, content)
}

/// Edit a file with workspace boundary enforcement.
#[allow(dead_code)]
pub fn edit_file_in_workspace(
    path: &str,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
    workspace_root: &Path,
) -> io::Result<EditFileOutput> {
    let absolute_path = normalize_path(path)?;
    let canonical_root =
        workspace_root.canonicalize().unwrap_or_else(|_| workspace_root.to_path_buf());
    validate_workspace_boundary(&absolute_path, &canonical_root)?;
    edit_file_resolved(&absolute_path, old_string, new_string, replace_all)
}

/// Check whether a path is a symlink that resolves outside the workspace.
#[allow(dead_code)]
pub fn is_symlink_escape(path: &Path, workspace_root: &Path) -> io::Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_symlink() {
        return Ok(false);
    }
    let resolved = path.canonicalize()?;
    let canonical_root =
        workspace_root.canonicalize().unwrap_or_else(|_| workspace_root.to_path_buf());
    Ok(!resolved.starts_with(&canonical_root))
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        edit_file, glob_search, grep_search, is_symlink_escape, read_file, read_file_in_workspace,
        write_file, GrepSearchInput, MAX_WRITE_SIZE,
    };

    fn temp_path(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        std::env::temp_dir().join(format!("clawd-native-{name}-{unique}"))
    }

    /// Serialises the tests that change the process working directory, which
    /// the file operations treat as the workspace root.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Runs a test with the temporary directory as the workspace root.
    ///
    /// These tests address scratch files by absolute path, so the workspace has
    /// to be entered before the confined entry points will accept them.
    struct TempWorkspaceCwd {
        previous: std::path::PathBuf,
    }

    impl Drop for TempWorkspaceCwd {
        fn drop(&mut self) {
            // Cleanup must tolerate failure (see AGENTS.md): the directory may
            // already be gone on Windows.
            let _ = std::env::set_current_dir(&self.previous);
        }
    }

    fn enter_temp_workspace() -> TempWorkspaceCwd {
        enter_workspace(&std::env::temp_dir())
    }

    fn enter_workspace(workspace: &std::path::Path) -> TempWorkspaceCwd {
        let previous = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(workspace).expect("enter workspace");
        TempWorkspaceCwd { previous }
    }

    #[test]
    fn live_file_entries_refuse_paths_outside_the_workspace() {
        let _guard = env_lock();
        let workspace = temp_path("live-confinement");
        std::fs::create_dir_all(&workspace).expect("workspace dir should be created");
        let _cwd = enter_workspace(&workspace);

        // A relative path inside the workspace is still accepted.
        write_file("inside.txt", "ok").expect("relative path inside the workspace");

        // Seed the target that must stay unreachable, so every rejection below
        // is a boundary decision and not a missing-file error.
        let outside = temp_path("live-confinement-outside.txt");
        std::fs::write(&outside, "content").expect("seed outside file");

        let error = write_file(outside.to_string_lossy().as_ref(), "nope")
            .expect_err("writing outside the workspace must be refused");
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("escapes workspace"), "{error}");

        let error = read_file(outside.to_string_lossy().as_ref(), None, None)
            .expect_err("reading outside the workspace must be refused");
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);

        let error = edit_file(outside.to_string_lossy().as_ref(), "content", "other", false)
            .expect_err("editing outside the workspace must be refused");
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);

        // `..` traversal resolves outside and is refused.
        let traversal = format!("..{}escape.txt", std::path::MAIN_SEPARATOR);
        let error = write_file(&traversal, "nope").expect_err("../ traversal must be refused");
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(
            !std::env::temp_dir().join("escape.txt").exists(),
            "traversal must not create a file"
        );

        // A symlink inside the workspace pointing outside does not launder a
        // write: the parent is canonicalised before the boundary check.
        #[cfg(unix)]
        {
            let link = workspace.join("escape-link");
            std::os::unix::fs::symlink(std::env::temp_dir(), &link).expect("symlink");
            let error = write_file(link.join("via-link.txt").to_string_lossy().as_ref(), "nope")
                .expect_err("a symlinked parent escaping the workspace must be refused");
            assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
            assert!(!std::env::temp_dir().join("via-link.txt").exists());
        }
    }

    #[test]
    fn reads_and_writes_files() {
        let _guard = env_lock();
        let _cwd = enter_temp_workspace();
        let path = temp_path("read-write.txt");
        let write_output = write_file(path.to_string_lossy().as_ref(), "one\ntwo\nthree")
            .expect("write should succeed");
        assert_eq!(write_output.kind, "create");

        let read_output = read_file(path.to_string_lossy().as_ref(), Some(1), Some(1))
            .expect("read should succeed");
        assert_eq!(read_output.file.content, "two");
    }

    #[test]
    fn edits_file_contents() {
        let _guard = env_lock();
        let _cwd = enter_temp_workspace();
        let path = temp_path("edit.txt");
        write_file(path.to_string_lossy().as_ref(), "alpha beta alpha")
            .expect("initial write should succeed");
        let output = edit_file(path.to_string_lossy().as_ref(), "alpha", "omega", true)
            .expect("edit should succeed");
        assert!(output.replace_all);
    }

    #[test]
    fn rejects_binary_files() {
        let _guard = env_lock();
        let _cwd = enter_temp_workspace();
        let path = temp_path("binary-test.bin");
        std::fs::write(&path, b"\x00\x01\x02\x03binary content").expect("write should succeed");
        let result = read_file(path.to_string_lossy().as_ref(), None, None);
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("binary"));
    }

    #[test]
    fn rejects_oversized_writes() {
        let _guard = env_lock();
        let _cwd = enter_temp_workspace();
        let path = temp_path("oversize-write.txt");
        let huge = "x".repeat(MAX_WRITE_SIZE + 1);
        let result = write_file(path.to_string_lossy().as_ref(), &huge);
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("too large"));
    }

    #[test]
    fn enforces_workspace_boundary() {
        let _guard = env_lock();
        let _cwd = enter_temp_workspace();
        let workspace = temp_path("workspace-boundary");
        std::fs::create_dir_all(&workspace).expect("workspace dir should be created");
        let inside = workspace.join("inside.txt");
        write_file(inside.to_string_lossy().as_ref(), "safe content")
            .expect("write inside workspace should succeed");

        // Reading inside workspace should succeed
        let result =
            read_file_in_workspace(inside.to_string_lossy().as_ref(), None, None, &workspace);
        assert!(result.is_ok());

        // Reading outside workspace should fail
        let outside = temp_path("outside-boundary.txt");
        write_file(outside.to_string_lossy().as_ref(), "unsafe content")
            .expect("write outside should succeed");
        let result =
            read_file_in_workspace(outside.to_string_lossy().as_ref(), None, None, &workspace);
        assert!(result.is_err());
        let error = result.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("escapes workspace"));
    }

    #[test]
    fn detects_symlink_escape() {
        let workspace = temp_path("symlink-workspace");
        std::fs::create_dir_all(&workspace).expect("workspace dir should be created");
        let outside = temp_path("symlink-target.txt");
        std::fs::write(&outside, "target content").expect("target should write");

        #[cfg(unix)]
        {
            let link_path = workspace.join("escape-link.txt");
            std::os::unix::fs::symlink(&outside, &link_path).expect("symlink should create");
            assert!(is_symlink_escape(&link_path, &workspace).expect("check should succeed"));
        }

        // Non-symlink file should not be an escape
        let normal = workspace.join("normal.txt");
        std::fs::write(&normal, "normal content").expect("normal file should write");
        assert!(!is_symlink_escape(&normal, &workspace).expect("check should succeed"));
    }

    #[test]
    fn globs_and_greps_directory() {
        let _guard = env_lock();
        let _cwd = enter_temp_workspace();
        let dir = temp_path("search-dir");
        std::fs::create_dir_all(&dir).expect("directory should be created");
        let file = dir.join("demo.rs");
        write_file(file.to_string_lossy().as_ref(), "fn main() {\n println!(\"hello\");\n}\n")
            .expect("file write should succeed");

        let globbed = glob_search("**/*.rs", Some(dir.to_string_lossy().as_ref()))
            .expect("glob should succeed");
        assert_eq!(globbed.num_files, 1);

        let grep_output = grep_search(&GrepSearchInput {
            pattern: String::from("hello"),
            path: Some(dir.to_string_lossy().into_owned()),
            glob: Some(String::from("**/*.rs")),
            output_mode: Some(String::from("content")),
            before: None,
            after: None,
            context_short: None,
            context: None,
            line_numbers: Some(true),
            case_insensitive: Some(false),
            file_type: None,
            head_limit: Some(10),
            offset: Some(0),
            multiline: Some(false),
        })
        .expect("grep should succeed");
        assert!(grep_output.content.unwrap_or_default().contains("hello"));
    }
}
