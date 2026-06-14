use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

const MAX_SCAN_ENTRIES: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProjectLanguage {
    Rust,
    Python,
    Go,
    JavaScriptTypeScript,
}

impl ProjectLanguage {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Python => "Python",
            Self::Go => "Go",
            Self::JavaScriptTypeScript => "JavaScript/TypeScript",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectSignal {
    pub language: ProjectLanguage,
    pub confidence: u8,
    pub evidence_paths: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolCheckRisk {
    Low,
    Medium,
}

impl ToolCheckRisk {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCheckPlan {
    pub language: ProjectLanguage,
    pub tool: String,
    pub command: String,
    pub purpose: String,
    pub risk: ToolCheckRisk,
    pub execution_mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolProbeReport {
    pub root: String,
    pub signals: Vec<ProjectSignal>,
    pub checks: Vec<ToolCheckPlan>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub enum ToolProbeError {
    NotDirectory(PathBuf),
    Walk(walkdir::Error),
    Io(std::io::Error),
}

impl fmt::Display for ToolProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotDirectory(path) => write!(formatter, "not a directory: {}", path.display()),
            Self::Walk(error) => write!(formatter, "failed to scan project: {error}"),
            Self::Io(error) => write!(formatter, "failed to read project metadata: {error}"),
        }
    }
}

impl std::error::Error for ToolProbeError {}

impl From<walkdir::Error> for ToolProbeError {
    fn from(value: walkdir::Error) -> Self {
        Self::Walk(value)
    }
}

impl From<std::io::Error> for ToolProbeError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Debug, Default)]
struct ProjectMarkers {
    rust: Vec<String>,
    python: Vec<String>,
    go: Vec<String>,
    js_ts: Vec<String>,
    scanned_entries: usize,
    truncated: bool,
}

pub fn build_tool_probe_report(root: &Path) -> Result<ToolProbeReport, ToolProbeError> {
    if !root.is_dir() {
        return Err(ToolProbeError::NotDirectory(root.to_path_buf()));
    }

    let markers = collect_project_markers(root)?;
    let signals = build_project_signals(&markers);
    let checks =
        signals.iter().flat_map(|signal| checks_for_language(signal.language)).collect::<Vec<_>>();
    let mut warnings = Vec::new();
    if markers.truncated {
        warnings.push(format!(
            "Project scan stopped after {MAX_SCAN_ENTRIES} entries; tool plan may be incomplete."
        ));
    }
    if signals.is_empty() {
        warnings.push(
            "No supported project markers were detected yet. Add a manifest or run from the project root."
                .to_string(),
        );
    }

    Ok(ToolProbeReport { root: root.display().to_string(), signals, checks, warnings })
}

fn collect_project_markers(root: &Path) -> Result<ProjectMarkers, ToolProbeError> {
    let mut markers = ProjectMarkers::default();
    let walker = WalkDir::new(root).follow_links(false).into_iter();
    for entry in walker.filter_entry(should_descend) {
        if markers.scanned_entries >= MAX_SCAN_ENTRIES {
            markers.truncated = true;
            break;
        }
        let entry = entry?;
        markers.scanned_entries += 1;
        if entry.file_type().is_dir() {
            continue;
        }
        record_marker(root, &entry, &mut markers);
    }
    sort_and_dedup(&mut markers.rust);
    sort_and_dedup(&mut markers.python);
    sort_and_dedup(&mut markers.go);
    sort_and_dedup(&mut markers.js_ts);
    Ok(markers)
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
    )
}

fn record_marker(root: &Path, entry: &DirEntry, markers: &mut ProjectMarkers) {
    let path = entry.path();
    let relative = relative_path(root, path);
    let file_name = entry.file_name().to_string_lossy();
    let extension = path.extension().and_then(|value| value.to_str()).unwrap_or_default();

    match file_name.as_ref() {
        "Cargo.toml" => markers.rust.push(relative),
        "pyproject.toml" | "requirements.txt" | "setup.py" | "Pipfile" => {
            markers.python.push(relative);
        }
        "go.mod" => markers.go.push(relative),
        "package.json" => {
            markers.js_ts.push(relative.clone());
            if package_json_mentions_typescript(path) {
                markers.js_ts.push(format!("{relative}:typescript"));
            }
        }
        _ => match extension {
            "rs" => push_limited(&mut markers.rust, relative),
            "py" => push_limited(&mut markers.python, relative),
            "go" => push_limited(&mut markers.go, relative),
            "js" | "jsx" | "ts" | "tsx" => push_limited(&mut markers.js_ts, relative),
            _ => {}
        },
    }
}

fn push_limited(values: &mut Vec<String>, value: String) {
    if values.len() < 8 {
        values.push(value);
    }
}

fn package_json_mentions_typescript(path: &Path) -> bool {
    fs::read_to_string(path)
        .is_ok_and(|content| content.contains("\"typescript\"") || content.contains("\"tsc\""))
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn sort_and_dedup(values: &mut Vec<String>) {
    values.sort();
    values.dedup();
}

fn build_project_signals(markers: &ProjectMarkers) -> Vec<ProjectSignal> {
    let mut signals = Vec::new();
    push_signal(&mut signals, ProjectLanguage::Rust, &markers.rust, 95);
    push_signal(&mut signals, ProjectLanguage::Python, &markers.python, 90);
    push_signal(&mut signals, ProjectLanguage::Go, &markers.go, 95);
    push_signal(&mut signals, ProjectLanguage::JavaScriptTypeScript, &markers.js_ts, 90);
    signals
}

fn push_signal(
    signals: &mut Vec<ProjectSignal>,
    language: ProjectLanguage,
    evidence: &[String],
    confidence: u8,
) {
    if !evidence.is_empty() {
        signals.push(ProjectSignal {
            language,
            confidence,
            evidence_paths: evidence.iter().take(8).cloned().collect(),
        });
    }
}

fn checks_for_language(language: ProjectLanguage) -> Vec<ToolCheckPlan> {
    match language {
        ProjectLanguage::Rust => vec![
            check(
                language,
                "cargo fmt",
                "cargo fmt --all --check",
                "Check Rust formatting.",
                ToolCheckRisk::Low,
            ),
            check(
                language,
                "cargo test",
                "cargo test --workspace",
                "Run Rust workspace tests.",
                ToolCheckRisk::Medium,
            ),
            check(
                language,
                "cargo clippy",
                "cargo clippy --workspace --all-targets",
                "Run Rust lint checks.",
                ToolCheckRisk::Medium,
            ),
        ],
        ProjectLanguage::Python => vec![
            check(
                language,
                "ruff/flake8",
                "ruff check .",
                "Check Python style and common errors.",
                ToolCheckRisk::Low,
            ),
            check(language, "pytest", "pytest", "Run Python tests.", ToolCheckRisk::Medium),
            check(
                language,
                "bandit",
                "bandit -r .",
                "Scan Python security issues.",
                ToolCheckRisk::Medium,
            ),
        ],
        ProjectLanguage::Go => vec![
            check(
                language,
                "gofmt",
                "gofmt -l <files>",
                "List Go files that need formatting.",
                ToolCheckRisk::Low,
            ),
            check(language, "go test", "go test ./...", "Run Go tests.", ToolCheckRisk::Medium),
            check(language, "go vet", "go vet ./...", "Run Go static checks.", ToolCheckRisk::Low),
        ],
        ProjectLanguage::JavaScriptTypeScript => vec![
            check(
                language,
                "npm test",
                "npm test",
                "Run project test script.",
                ToolCheckRisk::Medium,
            ),
            check(
                language,
                "eslint",
                "npm run lint",
                "Run JavaScript/TypeScript lint script.",
                ToolCheckRisk::Low,
            ),
            check(
                language,
                "tsc",
                "npm run typecheck",
                "Run TypeScript type checks when configured.",
                ToolCheckRisk::Low,
            ),
        ],
    }
}

fn check(
    language: ProjectLanguage,
    tool: &str,
    command: &str,
    purpose: &str,
    risk: ToolCheckRisk,
) -> ToolCheckPlan {
    ToolCheckPlan {
        language,
        tool: tool.to_string(),
        command: command.to_string(),
        purpose: purpose.to_string(),
        risk,
        execution_mode: "suggest-only".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{build_tool_probe_report, ProjectLanguage};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("sego-tool-probe-{name}-{nanos}"))
    }

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent dir");
        }
        fs::write(path, content).expect("write fixture");
    }

    #[test]
    fn detects_rust_project_and_plans_cargo_checks() {
        let root = temp_dir("rust");
        write(&root.join("Cargo.toml"), "[workspace]\n");
        let report = build_tool_probe_report(&root).expect("probe report");

        assert_eq!(report.signals[0].language, ProjectLanguage::Rust);
        assert!(report.checks.iter().any(|check| check.command == "cargo test --workspace"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detects_mixed_python_and_typescript_project() {
        let root = temp_dir("mixed");
        write(&root.join("pyproject.toml"), "[project]\n");
        write(&root.join("src/app.py"), "print('hi')\n");
        write(
            &root.join("package.json"),
            r#"{"devDependencies":{"typescript":"latest"},"scripts":{"typecheck":"tsc"}}"#,
        );

        let report = build_tool_probe_report(&root).expect("probe report");
        let languages = report.signals.iter().map(|signal| signal.language).collect::<Vec<_>>();

        assert!(languages.contains(&ProjectLanguage::Python));
        assert!(languages.contains(&ProjectLanguage::JavaScriptTypeScript));
        assert!(report.checks.iter().any(|check| check.command == "ruff check ."));
        assert!(report.checks.iter().any(|check| check.command == "npm run typecheck"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn ignores_heavy_generated_directories() {
        let root = temp_dir("ignore");
        write(&root.join("node_modules/pkg/package.json"), "{}");

        let report = build_tool_probe_report(&root).expect("probe report");

        assert!(report.signals.is_empty());
        assert!(report.warnings.iter().any(|warning| warning.contains("No supported")));

        let _ = fs::remove_dir_all(root);
    }
}
