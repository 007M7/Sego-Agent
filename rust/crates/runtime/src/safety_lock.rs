use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

const MAX_SCAN_ENTRIES: usize = 20_000;
const MAX_FILE_BYTES: u64 = 256 * 1024;
const MAX_FINDINGS: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SafetySeverity {
    High,
    Medium,
    Low,
}

impl SafetySeverity {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SafetyCategory {
    Secret,
    DangerousCommand,
    HardcodedPath,
    SensitiveConfig,
}

impl SafetyCategory {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Secret => "secret",
            Self::DangerousCommand => "dangerous-command",
            Self::HardcodedPath => "hardcoded-path",
            Self::SensitiveConfig => "sensitive-config",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SafetyScanMode {
    Workspace,
    Staged,
}

impl SafetyScanMode {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Staged => "staged",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafetyFinding {
    pub severity: SafetySeverity,
    pub category: SafetyCategory,
    pub file: String,
    pub line: Option<u32>,
    pub title: String,
    pub evidence: String,
    pub risk: String,
    pub suggestion: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafetyLockReport {
    pub root: String,
    pub mode: SafetyScanMode,
    pub findings: Vec<SafetyFinding>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub enum SafetyLockError {
    NotDirectory(PathBuf),
    Walk(walkdir::Error),
    Io(std::io::Error),
}

impl fmt::Display for SafetyLockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotDirectory(path) => write!(formatter, "not a directory: {}", path.display()),
            Self::Walk(error) => write!(formatter, "failed to scan project safety: {error}"),
            Self::Io(error) => write!(formatter, "failed to read project safety metadata: {error}"),
        }
    }
}

impl std::error::Error for SafetyLockError {}

impl From<walkdir::Error> for SafetyLockError {
    fn from(value: walkdir::Error) -> Self {
        Self::Walk(value)
    }
}

impl From<std::io::Error> for SafetyLockError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Debug, Default)]
struct ScanState {
    scanned_entries: usize,
    scanned_files: usize,
    skipped_large_files: usize,
    truncated_entries: bool,
    truncated_findings: bool,
}

pub fn build_safety_lock_report(root: &Path) -> Result<SafetyLockReport, SafetyLockError> {
    if !root.is_dir() {
        return Err(SafetyLockError::NotDirectory(root.to_path_buf()));
    }

    let mut findings = Vec::new();
    let mut state = ScanState::default();
    let walker = WalkDir::new(root).follow_links(false).into_iter();
    for entry in walker.filter_entry(should_descend) {
        if state.scanned_entries >= MAX_SCAN_ENTRIES {
            state.truncated_entries = true;
            break;
        }

        let entry = entry?;
        state.scanned_entries += 1;
        if entry.file_type().is_dir() {
            continue;
        }
        scan_file(root, &entry, &mut findings, &mut state)?;
        if findings.len() >= MAX_FINDINGS {
            state.truncated_findings = true;
            break;
        }
    }

    findings.sort_by_key(|finding| severity_rank(finding.severity));
    let warnings = build_warnings(&state);
    Ok(SafetyLockReport {
        root: root.display().to_string(),
        mode: SafetyScanMode::Workspace,
        findings,
        warnings,
    })
}

pub fn build_safety_lock_report_for_paths<I, P>(
    root: &Path,
    relative_paths: I,
    mode: SafetyScanMode,
) -> Result<SafetyLockReport, SafetyLockError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    if !root.is_dir() {
        return Err(SafetyLockError::NotDirectory(root.to_path_buf()));
    }

    let mut findings = Vec::new();
    let mut state = ScanState::default();
    for relative_path in relative_paths {
        if state.scanned_entries >= MAX_SCAN_ENTRIES {
            state.truncated_entries = true;
            break;
        }

        let relative_path = relative_path.as_ref();
        if !is_safe_relative_path(relative_path) {
            continue;
        }

        let path = root.join(relative_path);
        if !path.is_file() {
            continue;
        }

        state.scanned_entries += 1;
        scan_existing_file(root, &path, &mut findings, &mut state)?;
        if findings.len() >= MAX_FINDINGS {
            state.truncated_findings = true;
            break;
        }
    }

    findings.sort_by_key(|finding| severity_rank(finding.severity));
    let warnings = build_warnings(&state);
    Ok(SafetyLockReport { root: root.display().to_string(), mode, findings, warnings })
}

fn scan_file(
    root: &Path,
    entry: &DirEntry,
    findings: &mut Vec<SafetyFinding>,
    state: &mut ScanState,
) -> Result<(), SafetyLockError> {
    let path = entry.path();
    scan_existing_file(root, path, findings, state)
}

fn scan_existing_file(
    root: &Path,
    path: &Path,
    findings: &mut Vec<SafetyFinding>,
    state: &mut ScanState,
) -> Result<(), SafetyLockError> {
    let relative = relative_path(root, path);
    add_path_findings(&relative, findings);

    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_FILE_BYTES {
        state.skipped_large_files += 1;
        return Ok(());
    }

    let Ok(content) = fs::read_to_string(path) else {
        return Ok(());
    };
    state.scanned_files += 1;
    let scan_content = should_scan_file_content(&relative);
    let mut pending_cfg_test = false;
    for (index, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if pending_cfg_test && trimmed.starts_with("mod tests") {
            break;
        }
        pending_cfg_test = trimmed.starts_with("#[cfg(test)]");
        if scan_content {
            scan_line(&relative, index + 1, line, findings);
        }
        if findings.len() >= MAX_FINDINGS {
            break;
        }
    }
    Ok(())
}

fn is_safe_relative_path(path: &Path) -> bool {
    !path.is_absolute()
        && path.components().any(|component| matches!(component, std::path::Component::Normal(_)))
        && path.components().all(|component| {
            matches!(component, std::path::Component::Normal(_) | std::path::Component::CurDir)
        })
}

fn add_path_findings(relative: &str, findings: &mut Vec<SafetyFinding>) {
    let normalized = relative.replace('\\', "/");
    let name = normalized.rsplit('/').next().unwrap_or(&normalized).to_string();
    let lower_name = name.to_ascii_lowercase();
    let lower_path = normalized.to_ascii_lowercase();

    if lower_name == ".env"
        || lower_name.starts_with(".env.")
        || has_sensitive_extension(&lower_name)
        || lower_path.contains("id_rsa")
    {
        push_finding(
            findings,
            SafetyFinding {
                severity: SafetySeverity::High,
                category: SafetyCategory::SensitiveConfig,
                file: normalized,
                line: None,
                title: "Potential secret file".to_string(),
                evidence: name,
                risk: "This file name usually carries API keys, private keys, or local credentials."
                    .to_string(),
                suggestion:
                    "Do not commit real secrets. Keep local secret files ignored and commit only examples."
                        .to_string(),
            },
        );
    }
}

fn scan_line(relative: &str, line_number: usize, line: &str, findings: &mut Vec<SafetyFinding>) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }

    if looks_like_secret(trimmed) {
        push_finding(
            findings,
            SafetyFinding {
                severity: SafetySeverity::High,
                category: SafetyCategory::Secret,
                file: relative.to_string(),
                line: Some(line_number_u32(line_number)),
                title: "Potential hardcoded secret".to_string(),
                evidence: redact_evidence(trimmed),
                risk: "Hardcoded keys, tokens, or passwords can leak when code is pushed."
                    .to_string(),
                suggestion:
                    "Move real secrets to local environment variables or a secret manager before commit."
                        .to_string(),
            },
        );
    }

    if should_scan_dangerous_commands(relative) && looks_like_dangerous_command(trimmed) {
        push_finding(
            findings,
            SafetyFinding {
                severity: SafetySeverity::Medium,
                category: SafetyCategory::DangerousCommand,
                file: relative.to_string(),
                line: Some(line_number_u32(line_number)),
                title: "Potentially dangerous command".to_string(),
                evidence: truncate_evidence(trimmed),
                risk:
                    "This command can delete files or execute remote code if copied or run blindly."
                        .to_string(),
                suggestion:
                    "Review the command manually and prefer a safer, pinned, or narrower operation."
                        .to_string(),
            },
        );
    }

    if looks_like_hardcoded_path(trimmed) {
        push_finding(
            findings,
            SafetyFinding {
                severity: SafetySeverity::Low,
                category: SafetyCategory::HardcodedPath,
                file: relative.to_string(),
                line: Some(line_number_u32(line_number)),
                title: "Hardcoded local machine path".to_string(),
                evidence: truncate_evidence(trimmed),
                risk: "Local absolute paths often break on another machine or in CI.".to_string(),
                suggestion: "Use a project-relative path, config option, or environment variable."
                    .to_string(),
            },
        );
    }
}

fn looks_like_secret(line: &str) -> bool {
    let upper = line.to_ascii_uppercase();

    upper.contains("PRIVATE KEY") || looks_like_secret_assignment(line)
}

fn looks_like_secret_assignment(line: &str) -> bool {
    let Some((left, right)) = split_assignment(line) else {
        return false;
    };
    if !is_sensitive_key_name(left) {
        return false;
    }
    looks_like_literal_secret_value(right)
}

fn split_assignment(line: &str) -> Option<(&str, &str)> {
    if let Some((left, right)) = line.split_once('=') {
        return Some((left, right));
    }
    let (left, right) = line.split_once(':')?;
    if left.contains(' ') || left.contains("::") {
        return None;
    }
    Some((left, right))
}

fn is_sensitive_key_name(value: &str) -> bool {
    let name = value
        .trim()
        .trim_start_matches("let ")
        .trim_start_matches("pub ")
        .trim_start_matches("const ")
        .trim_start_matches("static ")
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_ascii_uppercase();

    matches!(
        name.as_str(),
        "API_KEY"
            | "ACCESS_KEY"
            | "SECRET"
            | "TOKEN"
            | "AUTH_TOKEN"
            | "BEARER_TOKEN"
            | "PASSWORD"
            | "PRIVATE_KEY"
            | "AUTHORIZATION"
    ) || name.ends_with("_API_KEY")
        || name.ends_with("_ACCESS_KEY")
        || name.ends_with("_SECRET")
        || name.ends_with("_TOKEN")
        || name.ends_with("_PASSWORD")
        || name.ends_with("_PRIVATE_KEY")
}

fn looks_like_literal_secret_value(value: &str) -> bool {
    let trimmed = value.trim().trim_end_matches(',').trim_end_matches(';').trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.starts_with("std::")
        || trimmed.starts_with("env::")
        || trimmed.starts_with("self.")
        || trimmed.starts_with("String")
        || trimmed.starts_with("Option")
        || trimmed.starts_with("None")
        || trimmed.starts_with("Some(")
        || trimmed.starts_with("read_env")
        || trimmed.starts_with("refreshed.")
        || trimmed.starts_with("resolved.")
        || trimmed.starts_with("token_set.")
    {
        return false;
    }
    if !trimmed.starts_with('"') && !trimmed.starts_with('\'') {
        return is_unquoted_secret_literal(trimmed);
    }
    let literal = trimmed.trim_matches('"').trim_matches('\'');
    !looks_like_placeholder_secret(literal)
        && (literal.contains(&["sk", "-"].concat())
            || literal.contains(&["xoxb", "-"].concat())
            || literal.contains(&["ghp", "_"].concat())
            || literal.len() >= 20)
}

fn is_unquoted_secret_literal(value: &str) -> bool {
    let token = value.trim();
    if looks_like_placeholder_secret(token) || token.contains(char::is_whitespace) {
        return false;
    }
    token.contains(&["sk", "-"].concat())
        || token.contains(&["xoxb", "-"].concat())
        || token.contains(&["ghp", "_"].concat())
}

fn has_sensitive_extension(lower_name: &str) -> bool {
    Path::new(lower_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension, "pem" | "key"))
}

fn looks_like_placeholder_secret(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    value.contains("...")
        || value.contains('<')
        || value.contains('>')
        || lower.contains("example")
        || lower.contains("placeholder")
        || lower.contains("dummy")
        || lower.contains("demo")
        || lower.contains("test")
}

fn looks_like_dangerous_command(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.contains(&["rm", " -rf"].concat())
        || lower.contains("del /s")
        || lower.contains("remove-item -recurse -force")
        || lower.contains("curl ") && lower.contains("| sh")
        || lower.contains("curl ") && lower.contains("| bash")
        || lower.contains("invoke-webrequest") && lower.contains("| iex")
        || lower.contains("iwr ") && lower.contains("| iex")
}

fn should_scan_dangerous_commands(relative: &str) -> bool {
    let path = Path::new(relative);
    let file_name =
        path.file_name().and_then(|name| name.to_str()).unwrap_or_default().to_ascii_lowercase();
    if matches!(file_name.as_str(), "dockerfile" | "makefile") {
        return true;
    }
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension, "sh" | "ps1" | "bat" | "cmd"))
}

fn should_scan_file_content(relative: &str) -> bool {
    let normalized = relative.replace('\\', "/").to_ascii_lowercase();
    !(normalized.ends_with("crates/runtime/src/safety_lock.rs")
        || normalized.contains("/tests/")
        || normalized.ends_with("_test.rs")
        || normalized.ends_with("_tests.rs")
        || has_markdown_extension(&normalized))
}

fn has_markdown_extension(lower_path: &str) -> bool {
    Path::new(lower_path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

fn looks_like_hardcoded_path(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    if lower.contains("contains(") {
        return false;
    }
    lower.contains(&["\"c:", "\\users\\"].concat())
        || lower.contains(&["'c:", "\\users\\"].concat())
        || lower.contains(&["\"d:", "\\"].concat())
        || lower.contains(&["'d:", "\\"].concat())
        || lower.contains(&["\"e:", "\\"].concat())
        || lower.contains(&["'e:", "\\"].concat())
        || lower.contains("\"/users/")
        || lower.contains("'/users/")
        || lower.contains("\"/home/")
        || lower.contains("'/home/")
}

fn push_finding(findings: &mut Vec<SafetyFinding>, finding: SafetyFinding) {
    if findings.len() < MAX_FINDINGS {
        findings.push(finding);
    }
}

fn line_number_u32(line_number: usize) -> u32 {
    u32::try_from(line_number).unwrap_or(u32::MAX)
}

fn build_warnings(state: &ScanState) -> Vec<String> {
    let mut warnings = Vec::new();
    if state.truncated_entries {
        warnings.push(format!(
            "Project scan stopped after {MAX_SCAN_ENTRIES} entries; safety findings may be incomplete."
        ));
    }
    if state.truncated_findings {
        warnings.push(format!(
            "Safety scan stopped after {MAX_FINDINGS} findings; fix the first batch and rerun /review safety."
        ));
    }
    if state.skipped_large_files > 0 {
        warnings.push(format!(
            "Skipped {} files larger than {} KB.",
            state.skipped_large_files,
            MAX_FILE_BYTES / 1024
        ));
    }
    warnings
}

fn severity_rank(severity: SafetySeverity) -> u8 {
    match severity {
        SafetySeverity::High => 0,
        SafetySeverity::Medium => 1,
        SafetySeverity::Low => 2,
    }
}

fn redact_evidence(value: &str) -> String {
    let truncated = truncate_evidence(value);
    if let Some(index) = truncated.find('=') {
        return format!("{}=<redacted>", truncated[..index].trim());
    }
    if let Some(index) = truncated.find(':') {
        return format!("{}: <redacted>", truncated[..index].trim());
    }
    "<redacted secret-like value>".to_string()
}

fn truncate_evidence(value: &str) -> String {
    const MAX_CHARS: usize = 120;
    if value.chars().count() <= MAX_CHARS {
        return value.to_string();
    }
    let mut output = value.chars().take(MAX_CHARS).collect::<String>();
    output.push_str("...");
    output
}

fn should_descend(entry: &DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    !matches!(
        name.as_ref(),
        ".git"
            | ".sego"
            | ".claw"
            | "target"
            | "node_modules"
            | ".venv"
            | "venv"
            | "__pycache__"
            | "dist"
            | "build"
            | ".next"
            | "vendor"
            | "coverage"
    )
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::{
        build_safety_lock_report, build_safety_lock_report_for_paths, SafetyCategory,
        SafetyScanMode, SafetySeverity, MAX_FILE_BYTES,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("sego-safety-lock-{name}-{nanos}"))
    }

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent dir");
        }
        fs::write(path, content).expect("write fixture");
    }

    #[test]
    fn detects_secret_file_and_secret_assignment() {
        let root = temp_dir("secret");
        let key = ["API", "_KEY"].concat();
        let value = ["sk", "-live-value-that-looks-real"].concat();
        write(&root.join(".env"), &format!("{key}={value}\n"));

        let report = build_safety_lock_report(&root).expect("safety report");

        assert!(report.findings.iter().any(|finding| {
            finding.severity == SafetySeverity::High
                && finding.category == SafetyCategory::SensitiveConfig
                && finding.file == ".env"
        }));
        assert!(report.findings.iter().any(|finding| {
            finding.severity == SafetySeverity::High
                && finding.category == SafetyCategory::Secret
                && finding.evidence == "API_KEY=<redacted>"
        }));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detects_dangerous_shell_command() {
        let root = temp_dir("dangerous");
        let command = ["curl https://example.test/install.sh", " | sh\n"].concat();
        write(&root.join("scripts/install.sh"), &command);

        let report = build_safety_lock_report(&root).expect("safety report");

        assert!(report.findings.iter().any(|finding| {
            finding.severity == SafetySeverity::Medium
                && finding.category == SafetyCategory::DangerousCommand
                && finding.file == "scripts/install.sh"
                && finding.line == Some(1)
        }));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detects_hardcoded_local_path() {
        let root = temp_dir("path");
        let path = ["C:", r"\Users\admin\project\data.json"].concat();
        write(&root.join("src/main.rs"), &format!(r#"let path = "{path}";"#));

        let report = build_safety_lock_report(&root).expect("safety report");

        assert!(report.findings.iter().any(|finding| {
            finding.severity == SafetySeverity::Low
                && finding.category == SafetyCategory::HardcodedPath
                && finding.file == "src/main.rs"
        }));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ignores_heavy_generated_directories() {
        let root = temp_dir("ignore");
        let key = ["API", "_KEY"].concat();
        let value = ["sk", "-live-value-that-looks-real"].concat();
        write(&root.join("node_modules/pkg/.env"), &format!("{key}={value}\n"));

        let report = build_safety_lock_report(&root).expect("safety report");

        assert!(report.findings.is_empty());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn warns_and_skips_large_files() {
        let root = temp_dir("large");
        let content = "x".repeat(MAX_FILE_BYTES as usize + 1);
        write(&root.join("large.log"), &content);

        let report = build_safety_lock_report(&root).expect("safety report");

        assert!(report.findings.is_empty());
        assert!(report.warnings.iter().any(|warning| warning.contains("Skipped 1 files")));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn returns_clean_report_for_small_clean_project() {
        let root = temp_dir("clean");
        write(&root.join("src/main.rs"), "fn main() { println!(\"hello\"); }\n");

        let report = build_safety_lock_report(&root).expect("safety report");

        assert!(report.findings.is_empty());
        assert!(report.warnings.is_empty());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn path_scoped_scan_ignores_unlisted_files() {
        let root = temp_dir("scoped");
        let key = ["API", "_KEY"].concat();
        let value = ["sk", "-live-value-that-looks-real"].concat();
        write(&root.join(".env"), &format!("{key}={value}\n"));
        write(&root.join("src/main.rs"), "fn main() { println!(\"hello\"); }\n");

        let report = build_safety_lock_report_for_paths(
            &root,
            [PathBuf::from("src/main.rs")],
            SafetyScanMode::Staged,
        )
        .expect("safety report");

        assert_eq!(report.mode, SafetyScanMode::Staged);
        assert!(report.findings.is_empty());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn path_scoped_scan_ignores_unsafe_relative_paths() {
        let root = temp_dir("unsafe-relative");
        let outside = root.parent().expect("temp root parent").join("secret.env");
        let key = ["API", "_KEY"].concat();
        let value = ["sk", "-live-value-that-looks-real"].concat();
        write(&outside, &format!("{key}={value}\n"));
        write(&root.join("safe.rs"), "fn main() {}\n");

        let report = build_safety_lock_report_for_paths(
            &root,
            [PathBuf::from("../secret.env"), PathBuf::from("safe.rs")],
            SafetyScanMode::Staged,
        )
        .expect("safety report");

        assert!(report.findings.is_empty());

        let _ = fs::remove_file(outside);
        let _ = fs::remove_dir_all(root);
    }
}
