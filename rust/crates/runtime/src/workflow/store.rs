//! Persistent storage for workflow data across sessions.
//!
//! Stores per-session lane event logs and aggregated trend data
//! in `.claw/workflow/` under the current workspace.

use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{report::SessionReport, WorkflowSnapshot};

const WORKFLOW_DIR: &str = ".claw/workflow";
const SESSIONS_DIR: &str = "sessions";
const TRENDS_FILE: &str = "trends.json";

#[derive(Debug)]
pub enum WorkflowStoreError {
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl Display for WorkflowStoreError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Json(e) => write!(f, "json error: {e}"),
        }
    }
}

impl std::error::Error for WorkflowStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Json(e) => Some(e),
        }
    }
}

impl From<std::io::Error> for WorkflowStoreError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for WorkflowStoreError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

/// Aggregated trend data across multiple sessions.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WorkflowTrends {
    pub total_sessions: u32,
    pub average_efficiency: f64,
    pub average_duration_seconds: u64,
    pub total_failures: u32,
    pub total_recoveries: u32,
    pub most_common_failure: Option<String>,
    pub improvement_rate: f64,
    #[serde(default)]
    pub recent_session_ids: Vec<String>,
}

/// Persistent store for workflow data.
#[derive(Debug, Clone)]
pub struct WorkflowStore {
    #[allow(dead_code)]
    root: PathBuf,
    sessions_dir: PathBuf,
    trends_path: PathBuf,
}

impl WorkflowStore {
    #[must_use]
    pub fn new(workspace_root: impl AsRef<Path>) -> Self {
        let root = workspace_root.as_ref().join(WORKFLOW_DIR);
        let sessions_dir = root.join(SESSIONS_DIR);
        let trends_path = root.join(TRENDS_FILE);
        Self { root, sessions_dir, trends_path }
    }

    /// Ensure the workflow directories exist.
    pub fn init(&self) -> Result<(), WorkflowStoreError> {
        fs::create_dir_all(&self.sessions_dir)?;
        if !self.trends_path.exists() {
            self.save_trends(&WorkflowTrends::default())?;
        }
        Ok(())
    }

    /// Persist a completed session snapshot to disk.
    pub fn save_session(&self, snapshot: &WorkflowSnapshot) -> Result<PathBuf, WorkflowStoreError> {
        self.init()?;
        let mut record = BTreeMap::new();
        record.insert("session_id".to_string(), serde_json::json!(snapshot.session_id));
        record.insert("started_at".to_string(), serde_json::json!(snapshot.started_at));
        record.insert("finished_at".to_string(), serde_json::json!(snapshot.finished_at));
        record.insert("task_description".to_string(), serde_json::json!(snapshot.task_description));
        record.insert("failure_count".to_string(), serde_json::json!(snapshot.failure_count));
        record.insert("efficiency_score".to_string(), serde_json::json!(snapshot.efficiency_score));
        record.insert(
            "recovery_attempts".to_string(),
            serde_json::json!(snapshot.recovery_stats.attempts),
        );
        record.insert(
            "recovery_successes".to_string(),
            serde_json::json!(snapshot.recovery_stats.successes),
        );
        record.insert(
            "green_level".to_string(),
            serde_json::json!(snapshot
                .green_level
                .map(super::super::green_contract::GreenLevel::as_str)),
        );

        let timestamp = snapshot.started_at.as_deref().unwrap_or("unknown").replace(':', "-");
        let filename = format!("{timestamp}.json");
        let path = self.sessions_dir.join(&filename);

        let json = serde_json::to_string_pretty(&record)?;
        fs::write(&path, json)?;
        Ok(path)
    }

    /// Load all historical session data and generate a trend analysis.
    pub fn load_trends(&self) -> Result<WorkflowTrends, WorkflowStoreError> {
        if self.trends_path.exists() {
            let data = fs::read_to_string(&self.trends_path)?;
            Ok(serde_json::from_str(&data)?)
        } else {
            Ok(WorkflowTrends::default())
        }
    }

    /// Update trends with a new session report and persist.
    pub fn update_trends(
        &self,
        report: &SessionReport,
        duration_seconds: u64,
    ) -> Result<WorkflowTrends, WorkflowStoreError> {
        let mut trends = self.load_trends()?;
        let n = f64::from(trends.total_sessions);

        trends.total_sessions += 1;
        trends.average_efficiency =
            (trends.average_efficiency * n + report.efficiency_score) / (n + 1.0);
        trends.average_duration_seconds = ((trends.average_duration_seconds as f64 * n
            + duration_seconds as f64)
            / (n + 1.0)) as u64;
        trends.total_failures += report.failure_count;
        trends.total_recoveries += report.recovery_successes;
        trends.improvement_rate = if trends.total_sessions > 1 {
            (report.efficiency_score - trends.average_efficiency) / trends.average_efficiency
        } else {
            0.0
        };

        // Keep track of recent session IDs
        trends.recent_session_ids.push(report.session_id.clone());
        if trends.recent_session_ids.len() > 20 {
            trends.recent_session_ids.remove(0);
        }

        self.save_trends(&trends)?;
        Ok(trends)
    }

    /// List saved session files, ordered by filename (which is timestamp-based).
    pub fn list_sessions(&self) -> Result<Vec<PathBuf>, WorkflowStoreError> {
        if !self.sessions_dir.exists() {
            return Ok(Vec::new());
        }
        let mut paths: Vec<PathBuf> = fs::read_dir(&self.sessions_dir)?
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        paths.sort();
        Ok(paths)
    }

    /// Load a specific session by its file path.
    pub fn load_session(&self, path: &Path) -> Result<serde_json::Value, WorkflowStoreError> {
        let data = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&data)?)
    }

    /// Load the N most recent sessions.
    pub fn load_recent_sessions(
        &self,
        count: usize,
    ) -> Result<Vec<serde_json::Value>, WorkflowStoreError> {
        let paths = self.list_sessions()?;
        let recent: Vec<_> = paths.iter().rev().take(count).collect();
        let mut sessions = Vec::new();
        for path in recent.iter().rev() {
            sessions.push(self.load_session(path)?);
        }
        Ok(sessions)
    }

    fn save_trends(&self, trends: &WorkflowTrends) -> Result<(), WorkflowStoreError> {
        let json = serde_json::to_string_pretty(trends)?;
        fs::write(&self.trends_path, json)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::workflow::SessionSummary;

    #[test]
    fn initializes_workflow_directories() {
        let tmp = std::env::temp_dir().join(format!("claw-wf-test-{}", rand_id()));
        let store = WorkflowStore::new(&tmp);
        store.init().expect("init should succeed");
        assert!(store.sessions_dir.exists());
        assert!(store.trends_path.exists());
        fs::remove_dir_all(store.root).expect("cleanup should succeed");
    }

    #[test]
    fn saves_and_loads_session() {
        let tmp = std::env::temp_dir().join(format!("claw-wf-test-{}", rand_id()));
        let store = WorkflowStore::new(&tmp);

        let mut snap = WorkflowSnapshot::new("test-session-1");
        snap.started_at = Some("2026-05-19T14:30:00Z".to_string());
        snap.finished_at = Some("2026-05-19T14:52:00Z".to_string());
        snap.failure_count = 1;
        snap.compute_efficiency();

        let path = store.save_session(&snap).expect("save should succeed");
        assert!(path.exists());

        let loaded = store.load_session(&path).expect("load should succeed");
        assert_eq!(loaded["session_id"], "test-session-1");
        assert_eq!(loaded["failure_count"], 1);

        fs::remove_dir_all(store.root).expect("cleanup should succeed");
    }

    #[test]
    fn updates_trends_across_sessions() {
        let tmp = std::env::temp_dir().join(format!("claw-wf-test-{}", rand_id()));
        let store = WorkflowStore::new(&tmp);

        let report = SessionReport {
            session_id: "s1".to_string(),
            session_summary: SessionSummary::default(),
            efficiency_score: 85.0,
            failure_count: 1,
            recovery_attempts: 1,
            recovery_successes: 1,
            green_level: None,
            suggestions: vec![],
        };

        store.update_trends(&report, 1320).expect("first trend update should succeed");
        let trends = store.load_trends().expect("trends should load");
        assert_eq!(trends.total_sessions, 1);
        assert!((trends.average_efficiency - 85.0).abs() < 0.01);

        let report2 = SessionReport {
            session_id: "s2".to_string(),
            efficiency_score: 95.0,
            ..report.clone()
        };
        store.update_trends(&report2, 900).expect("second trend update should succeed");
        let trends = store.load_trends().expect("trends should load");
        assert_eq!(trends.total_sessions, 2);
        assert!((trends.average_efficiency - 90.0).abs() < 0.01);

        fs::remove_dir_all(store.root).expect("cleanup should succeed");
    }

    #[test]
    fn lists_and_loads_recent_sessions() {
        let tmp = std::env::temp_dir().join(format!("claw-wf-test-{}", rand_id()));
        let store = WorkflowStore::new(&tmp);

        for i in 1..=5 {
            let mut snap = WorkflowSnapshot::new(format!("session-{i}"));
            snap.started_at = Some(format!("2026-05-19T1{i}:00:00Z"));
            snap.compute_efficiency();
            store.save_session(&snap).expect("save should succeed");
        }

        let sessions = store.list_sessions().expect("list should succeed");
        assert_eq!(sessions.len(), 5);

        let recent = store.load_recent_sessions(3).expect("recent should load");
        assert_eq!(recent.len(), 3);

        fs::remove_dir_all(store.root).expect("cleanup should succeed");
    }

    fn rand_id() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0)
    }
}
