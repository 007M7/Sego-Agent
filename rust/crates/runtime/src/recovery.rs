use std::fmt::{Display, Formatter, Write as _};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const RECOVERY_SCHEMA_VERSION: u32 = 1;
pub const RECOVERY_DIR: &str = ".sego/recovery";
pub const LATEST_SESSION_FILE: &str = "latest-session.json";
pub const EXIT_STATE_FILE: &str = "exit-state.json";
pub const RECOVERY_SUMMARY_FILE: &str = "recovery-summary.md";

#[derive(Debug)]
pub enum RecoveryError {
    Io { path: PathBuf, source: io::Error },
    Json { path: PathBuf, source: serde_json::Error },
    InvalidRecord { path: PathBuf, message: String },
}

impl Display for RecoveryError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "{}: {source}", path.display())
            }
            Self::Json { path, source } => {
                write!(f, "invalid recovery JSON at {}: {source}", path.display())
            }
            Self::InvalidRecord { path, message } => {
                write!(f, "invalid recovery record at {}: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for RecoveryError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoverySessionPathKind {
    Absolute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryExitState {
    Active,
    Graceful,
    Interrupted,
    Crashed,
    Unknown,
}

impl RecoveryExitState {
    #[must_use]
    pub const fn is_potentially_recoverable(self) -> bool {
        !matches!(self, Self::Graceful)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAvailability {
    NoState,
    CleanExit,
    Recoverable,
    MissingSession,
    UnreadableState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatestSessionRecord {
    pub schema_version: u32,
    pub session_id: String,
    pub session_path: PathBuf,
    pub session_path_kind: RecoverySessionPathKind,
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub updated_at_epoch_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitStateRecord {
    pub schema_version: u32,
    pub session_id: String,
    pub state: RecoveryExitState,
    pub last_seen_at_epoch_seconds: u64,
    pub last_user_goal: Option<String>,
    pub last_error: Option<String>,
    pub session_path: PathBuf,
    pub session_path_kind: RecoverySessionPathKind,
    pub cwd: PathBuf,
    /// C20.6-B R2 UX-E: artifact path (e.g. `.sego/reviews/<id>.md`) if one
    /// was produced before the failure. Empty/None for non-artifact sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryStateUpdate {
    pub session_id: String,
    pub session_path: PathBuf,
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub state: RecoveryExitState,
    pub last_user_goal: Option<String>,
    pub last_error: Option<String>,
    pub observed_at_epoch_seconds: u64,
    /// C20.6-B R2 UX-E: optional artifact path produced before failure.
    pub artifact_path: Option<PathBuf>,
}

impl RecoveryStateUpdate {
    #[must_use]
    pub fn new(
        session_id: impl Into<String>,
        session_path: impl Into<PathBuf>,
        cwd: impl Into<PathBuf>,
        state: RecoveryExitState,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            session_path: session_path.into(),
            cwd: cwd.into(),
            model: None,
            state,
            last_user_goal: None,
            last_error: None,
            observed_at_epoch_seconds: current_epoch_seconds(),
            artifact_path: None,
        }
    }

    #[must_use]
    pub fn with_model(mut self, model: impl Into<Option<String>>) -> Self {
        self.model = normalize_optional_string(model.into());
        self
    }

    #[must_use]
    pub fn with_last_user_goal(mut self, goal: impl Into<Option<String>>) -> Self {
        self.last_user_goal = normalize_optional_string(goal.into());
        self
    }

    #[must_use]
    pub fn with_last_error(mut self, error: impl Into<Option<String>>) -> Self {
        self.last_error = normalize_optional_string(error.into());
        self
    }

    #[must_use]
    pub fn with_artifact_path(mut self, artifact_path: impl Into<Option<PathBuf>>) -> Self {
        self.artifact_path = artifact_path.into();
        self
    }

    #[must_use]
    pub const fn with_observed_at_epoch_seconds(mut self, observed_at_epoch_seconds: u64) -> Self {
        self.observed_at_epoch_seconds = observed_at_epoch_seconds;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedRecoveryState {
    pub latest_session: LatestSessionRecord,
    pub exit_state: ExitStateRecord,
    pub latest_session_path: PathBuf,
    pub exit_state_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryAssessment {
    pub availability: RecoveryAvailability,
    pub latest_session: Option<LatestSessionRecord>,
    pub exit_state: Option<ExitStateRecord>,
    pub message: String,
}

#[must_use]
pub fn recovery_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(RECOVERY_DIR)
}

#[must_use]
pub fn latest_session_record_path(workspace_root: &Path) -> PathBuf {
    recovery_dir(workspace_root).join(LATEST_SESSION_FILE)
}

#[must_use]
pub fn exit_state_record_path(workspace_root: &Path) -> PathBuf {
    recovery_dir(workspace_root).join(EXIT_STATE_FILE)
}

#[must_use]
pub fn recovery_summary_path(workspace_root: &Path) -> PathBuf {
    recovery_dir(workspace_root).join(RECOVERY_SUMMARY_FILE)
}

pub fn persist_recovery_state(
    workspace_root: &Path,
    update: RecoveryStateUpdate,
) -> Result<PersistedRecoveryState, RecoveryError> {
    let session_path = absolute_path(workspace_root, update.session_path);
    let cwd = absolute_path(workspace_root, update.cwd);

    let latest_session = LatestSessionRecord {
        schema_version: RECOVERY_SCHEMA_VERSION,
        session_id: update.session_id.clone(),
        session_path: session_path.clone(),
        session_path_kind: RecoverySessionPathKind::Absolute,
        cwd: cwd.clone(),
        model: update.model,
        updated_at_epoch_seconds: update.observed_at_epoch_seconds,
    };
    validate_latest_session(&latest_session, &latest_session_record_path(workspace_root))?;

    let exit_state = ExitStateRecord {
        schema_version: RECOVERY_SCHEMA_VERSION,
        session_id: update.session_id,
        state: update.state,
        last_seen_at_epoch_seconds: update.observed_at_epoch_seconds,
        last_user_goal: update.last_user_goal,
        last_error: update.last_error,
        session_path,
        session_path_kind: RecoverySessionPathKind::Absolute,
        cwd,
        artifact_path: update.artifact_path,
    };
    validate_exit_state(&exit_state, &exit_state_record_path(workspace_root))?;

    let latest_session_path = latest_session_record_path(workspace_root);
    let exit_state_path = exit_state_record_path(workspace_root);
    write_json(&latest_session_path, &latest_session)?;
    write_json(&exit_state_path, &exit_state)?;

    Ok(PersistedRecoveryState { latest_session, exit_state, latest_session_path, exit_state_path })
}

pub fn read_latest_session(
    workspace_root: &Path,
) -> Result<Option<LatestSessionRecord>, RecoveryError> {
    let path = latest_session_record_path(workspace_root);
    let Some(record) = read_json::<LatestSessionRecord>(&path)? else {
        return Ok(None);
    };
    validate_latest_session(&record, &path)?;
    Ok(Some(record))
}

pub fn read_exit_state(workspace_root: &Path) -> Result<Option<ExitStateRecord>, RecoveryError> {
    let path = exit_state_record_path(workspace_root);
    let Some(record) = read_json::<ExitStateRecord>(&path)? else {
        return Ok(None);
    };
    validate_exit_state(&record, &path)?;
    Ok(Some(record))
}

#[must_use]
pub fn assess_recovery_state(workspace_root: &Path) -> RecoveryAssessment {
    let latest_session = match read_latest_session(workspace_root) {
        Ok(record) => record,
        Err(error) => return unreadable_assessment(error.to_string()),
    };

    let exit_state = match read_exit_state(workspace_root) {
        Ok(record) => record,
        Err(error) => return unreadable_assessment(error.to_string()),
    };

    let Some(exit_state) = exit_state else {
        return RecoveryAssessment {
            availability: RecoveryAvailability::NoState,
            latest_session,
            exit_state: None,
            message: "no recovery state found".to_string(),
        };
    };

    let availability = if !exit_state.state.is_potentially_recoverable() {
        RecoveryAvailability::CleanExit
    } else if exit_state.session_path.exists() {
        RecoveryAvailability::Recoverable
    } else {
        RecoveryAvailability::MissingSession
    };

    let message = match availability {
        RecoveryAvailability::CleanExit => "previous session exited cleanly".to_string(),
        RecoveryAvailability::Recoverable => {
            "previous session may not have ended normally".to_string()
        }
        RecoveryAvailability::MissingSession => {
            "previous session may be recoverable, but its session file is missing".to_string()
        }
        RecoveryAvailability::NoState | RecoveryAvailability::UnreadableState => {
            "no recovery state found".to_string()
        }
    };

    RecoveryAssessment { availability, latest_session, exit_state: Some(exit_state), message }
}

#[must_use]
pub fn render_recovery_summary(assessment: &RecoveryAssessment) -> String {
    let mut summary = String::new();
    let _ = writeln!(summary, "# Sego Recovery Summary");
    let _ = writeln!(summary);
    let _ = writeln!(summary, "- availability: {:?}", assessment.availability);
    let _ = writeln!(summary, "- message: {}", assessment.message);

    if let Some(exit_state) = &assessment.exit_state {
        let _ = writeln!(summary, "- session_id: {}", exit_state.session_id);
        let _ = writeln!(summary, "- state: {:?}", exit_state.state);
        let _ = writeln!(
            summary,
            "- last_seen_at_epoch_seconds: {}",
            exit_state.last_seen_at_epoch_seconds
        );
        let _ = writeln!(summary, "- session_path: {}", exit_state.session_path.display());
        let _ = writeln!(summary, "- cwd: {}", exit_state.cwd.display());
        if let Some(goal) = &exit_state.last_user_goal {
            let _ = writeln!(summary, "- last_user_goal: {goal}");
        }
        if let Some(error) = &exit_state.last_error {
            let _ = writeln!(summary, "- last_error: {error}");
        }
        if let Some(artifact) = &exit_state.artifact_path {
            let _ = writeln!(summary, "- artifact_path: {}", artifact.display());
        }
    }

    if let Some(latest_session) = &assessment.latest_session {
        let _ = writeln!(summary, "- model: {}", latest_session.model.as_deref().unwrap_or("n/a"));
        let _ = writeln!(
            summary,
            "- updated_at_epoch_seconds: {}",
            latest_session.updated_at_epoch_seconds
        );
    }

    let _ = writeln!(summary);
    let _ = writeln!(summary, "## Suggested Commands");
    let _ = writeln!(summary);
    let _ = writeln!(summary, "```powershell");
    let _ = writeln!(summary, "# Resume the previous session");
    let _ = writeln!(summary, "sego --resume latest");
    let _ = writeln!(summary);
    let _ = writeln!(summary, "# Inspect session status");
    let _ = writeln!(summary, "sego --resume latest /status");
    let _ = writeln!(summary);
    let _ = writeln!(summary, "# Export session to a file");
    let _ = writeln!(summary, "sego --resume latest /recovery-export");
    let _ = writeln!(summary, "```");

    summary
}

pub fn write_recovery_summary(
    workspace_root: &Path,
    assessment: &RecoveryAssessment,
) -> Result<PathBuf, RecoveryError> {
    let path = recovery_summary_path(workspace_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|source| RecoveryError::Io { path: parent.to_path_buf(), source })?;
    }
    fs::write(&path, render_recovery_summary(assessment))
        .map_err(|source| RecoveryError::Io { path: path.clone(), source })?;
    Ok(path)
}

fn unreadable_assessment(message: String) -> RecoveryAssessment {
    RecoveryAssessment {
        availability: RecoveryAvailability::UnreadableState,
        latest_session: None,
        exit_state: None,
        message,
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), RecoveryError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|source| RecoveryError::Io { path: parent.to_path_buf(), source })?;
    }
    let mut body = serde_json::to_string_pretty(value)
        .map_err(|source| RecoveryError::Json { path: path.to_path_buf(), source })?;
    body.push('\n');
    fs::write(path, body).map_err(|source| RecoveryError::Io { path: path.to_path_buf(), source })
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, RecoveryError> {
    if !path.exists() {
        return Ok(None);
    }
    let body = fs::read_to_string(path)
        .map_err(|source| RecoveryError::Io { path: path.to_path_buf(), source })?;
    let record = serde_json::from_str(&body)
        .map_err(|source| RecoveryError::Json { path: path.to_path_buf(), source })?;
    Ok(Some(record))
}

fn validate_latest_session(record: &LatestSessionRecord, path: &Path) -> Result<(), RecoveryError> {
    validate_schema_version(record.schema_version, path)?;
    validate_absolute_path("session_path", &record.session_path, path)?;
    validate_absolute_path("cwd", &record.cwd, path)?;
    if record.session_path_kind != RecoverySessionPathKind::Absolute {
        return Err(invalid_record(path, "session_path_kind must be absolute"));
    }
    Ok(())
}

fn validate_exit_state(record: &ExitStateRecord, path: &Path) -> Result<(), RecoveryError> {
    validate_schema_version(record.schema_version, path)?;
    validate_absolute_path("session_path", &record.session_path, path)?;
    validate_absolute_path("cwd", &record.cwd, path)?;
    if record.session_path_kind != RecoverySessionPathKind::Absolute {
        return Err(invalid_record(path, "session_path_kind must be absolute"));
    }
    Ok(())
}

fn validate_schema_version(version: u32, path: &Path) -> Result<(), RecoveryError> {
    if version != RECOVERY_SCHEMA_VERSION {
        return Err(invalid_record(
            path,
            format!("unsupported schema_version {version}; expected {RECOVERY_SCHEMA_VERSION}"),
        ));
    }
    Ok(())
}

fn validate_absolute_path(
    field: &'static str,
    value: &Path,
    record_path: &Path,
) -> Result<(), RecoveryError> {
    if !value.is_absolute() {
        return Err(invalid_record(
            record_path,
            format!("{field} must be an absolute path: {}", value.display()),
        ));
    }
    Ok(())
}

fn invalid_record(path: &Path, message: impl Into<String>) -> RecoveryError {
    RecoveryError::InvalidRecord { path: path.to_path_buf(), message: message.into() }
}

fn absolute_path(base: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn current_epoch_seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_persists_absolute_session_paths() {
        let root = temp_dir("absolute-paths");
        let relative_session = PathBuf::from(".claw").join("sessions").join("session.jsonl");
        let absolute_session = root.join(&relative_session);
        fs::create_dir_all(absolute_session.parent().expect("session parent"))
            .expect("create session dir");
        fs::write(&absolute_session, "{}\n").expect("write session");

        let persisted = persist_recovery_state(
            &root,
            RecoveryStateUpdate::new(
                "session-1",
                relative_session,
                root.clone(),
                RecoveryExitState::Active,
            )
            .with_model(Some("deepseek-v4-flash".to_string()))
            .with_last_user_goal(Some("finish crash recovery".to_string()))
            .with_observed_at_epoch_seconds(42),
        )
        .expect("persist recovery state");

        assert_eq!(persisted.latest_session.session_path, absolute_session);
        assert_eq!(persisted.exit_state.session_path, absolute_session);
        assert_eq!(persisted.latest_session.session_path_kind, RecoverySessionPathKind::Absolute);
        assert_eq!(persisted.exit_state.session_path_kind, RecoverySessionPathKind::Absolute);

        let latest = read_latest_session(&root).expect("read latest").expect("latest exists");
        let exit = read_exit_state(&root).expect("read exit").expect("exit exists");
        assert_eq!(latest.session_path, absolute_session);
        assert_eq!(exit.session_path, absolute_session);
        assert_eq!(exit.last_user_goal.as_deref(), Some("finish crash recovery"));
        cleanup_temp_dir(root);
    }

    #[test]
    fn recovery_assesses_active_session_as_recoverable() {
        let root = temp_dir("recoverable");
        let session_path = root.join(".claw").join("sessions").join("session.jsonl");
        fs::create_dir_all(session_path.parent().expect("session parent"))
            .expect("create session dir");
        fs::write(&session_path, "{}\n").expect("write session");

        persist_recovery_state(
            &root,
            RecoveryStateUpdate::new(
                "session-2",
                session_path.clone(),
                root.clone(),
                RecoveryExitState::Active,
            ),
        )
        .expect("persist recovery state");

        let assessment = assess_recovery_state(&root);
        assert_eq!(assessment.availability, RecoveryAvailability::Recoverable);
        assert_eq!(assessment.exit_state.expect("exit state").session_path, session_path);
        cleanup_temp_dir(root);
    }

    #[test]
    fn recovery_assesses_graceful_session_as_clean_exit() {
        let root = temp_dir("graceful");
        let session_path = root.join(".claw").join("sessions").join("session.jsonl");
        fs::create_dir_all(session_path.parent().expect("session parent"))
            .expect("create session dir");
        fs::write(&session_path, "{}\n").expect("write session");

        persist_recovery_state(
            &root,
            RecoveryStateUpdate::new(
                "session-3",
                session_path,
                root.clone(),
                RecoveryExitState::Graceful,
            ),
        )
        .expect("persist recovery state");

        let assessment = assess_recovery_state(&root);
        assert_eq!(assessment.availability, RecoveryAvailability::CleanExit);
        cleanup_temp_dir(root);
    }

    #[test]
    fn recovery_assesses_missing_session_file_without_panicking() {
        let root = temp_dir("missing-session");
        let session_path = root.join(".claw").join("sessions").join("missing.jsonl");

        persist_recovery_state(
            &root,
            RecoveryStateUpdate::new(
                "session-4",
                session_path,
                root.clone(),
                RecoveryExitState::Interrupted,
            ),
        )
        .expect("persist recovery state");

        let assessment = assess_recovery_state(&root);
        assert_eq!(assessment.availability, RecoveryAvailability::MissingSession);
        cleanup_temp_dir(root);
    }

    #[test]
    fn recovery_reports_invalid_json_without_panicking() {
        let root = temp_dir("invalid-json");
        let recovery_dir = recovery_dir(&root);
        fs::create_dir_all(&recovery_dir).expect("create recovery dir");
        fs::write(exit_state_record_path(&root), "{not-json").expect("write invalid json");

        let assessment = assess_recovery_state(&root);
        assert_eq!(assessment.availability, RecoveryAvailability::UnreadableState);
        assert!(assessment.message.contains("invalid recovery JSON"));
        cleanup_temp_dir(root);
    }

    #[test]
    fn recovery_summary_includes_resume_commands_and_absolute_session_path() {
        let root = temp_dir("summary");
        let session_path = root.join(".claw").join("sessions").join("session.jsonl");
        fs::create_dir_all(session_path.parent().expect("session parent"))
            .expect("create session dir");
        fs::write(&session_path, "{}\n").expect("write session");

        persist_recovery_state(
            &root,
            RecoveryStateUpdate::new(
                "session-5",
                session_path.clone(),
                root.clone(),
                RecoveryExitState::Crashed,
            )
            .with_last_error(Some("window closed".to_string())),
        )
        .expect("persist recovery state");

        let assessment = assess_recovery_state(&root);
        let summary = render_recovery_summary(&assessment);
        assert!(summary.contains("sego --resume latest /status"));
        assert!(summary.contains("sego --resume latest /recovery-export"));
        assert!(summary.contains(&session_path.display().to_string()));

        let summary_path = write_recovery_summary(&root, &assessment).expect("write summary");
        assert_eq!(summary_path, recovery_summary_path(&root));
        cleanup_temp_dir(root);
    }

    #[test]
    fn recovery_summary_renders_artifact_path_when_present() {
        // R2 UX-E: artifact_path field flows through recovery summary.
        let root = temp_dir("artifact-path");
        let session_path = root.join(".claw").join("sessions").join("session.jsonl");
        fs::create_dir_all(session_path.parent().expect("parent")).expect("create session dir");
        fs::write(&session_path, "{}\n").expect("write session");
        let artifact = root.join(".sego").join("reviews").join("review-test.md");

        persist_recovery_state(
            &root,
            RecoveryStateUpdate::new(
                "session-artifact",
                session_path,
                root.clone(),
                RecoveryExitState::Crashed,
            )
            .with_artifact_path(Some(artifact.clone())),
        )
        .expect("persist");

        let assessment = assess_recovery_state(&root);
        let summary = render_recovery_summary(&assessment);
        assert!(summary.contains("artifact_path:"));
        assert!(summary.contains(&artifact.display().to_string()));
        cleanup_temp_dir(root);
    }

    #[test]
    fn old_exit_state_without_artifact_path_still_loads() {
        // R2 UX-E: backward compatibility — old JSON missing artifact_path must deserialize.
        let json = r#"{
            "schema_version": 1,
            "session_id": "old-session",
            "state": "crashed",
            "last_seen_at_epoch_seconds": 1700000000,
            "last_user_goal": null,
            "last_error": "boom",
            "session_path": "C:/tmp/old-session.jsonl",
            "session_path_kind": "absolute",
            "cwd": "C:/tmp"
        }"#;
        let parsed: ExitStateRecord =
            serde_json::from_str(json).expect("deserialize old exit state");
        assert_eq!(parsed.session_id, "old-session");
        assert!(parsed.artifact_path.is_none());
    }

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).expect("system time").as_nanos();
        let path = std::env::temp_dir().join(format!("sego-recovery-{name}-{unique}"));
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    fn cleanup_temp_dir(path: PathBuf) {
        if let Err(error) = fs::remove_dir_all(&path) {
            if error.kind() != io::ErrorKind::NotFound {
                panic!("failed to cleanup {}: {error}", path.display());
            }
        }
    }
}
