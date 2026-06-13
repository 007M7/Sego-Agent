//! Persistent active task state for crash recovery.
//! Writes .sego/runtime/active_task.json for new-agent recovery.
//! Core of Sego's recoverable runtime.
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const SEGO_DIR: &str = ".sego";
const RUNTIME_DIR: &str = "runtime";
const ACTIVE_TASK_FILE: &str = "active_task.json";
const RECOVERY_PROMPT_FILE: &str = "recovery_prompt.md";
const HEARTBEAT_FILE: &str = "heartbeat.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveTaskStatus {
    Created,
    Running,
    Completed,
    Failed,
    Interrupted,
}

impl Display for ActiveTaskStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Created => write!(f, "created"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Interrupted => write!(f, "interrupted"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedProcess {
    pub pid: u32,
    pub command: String,
    pub cwd: String,
    pub port: Option<u16>,
    pub start_time: String,
    pub status: String,
    pub purpose: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveTask {
    pub task_id: String,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub remote: Option<String>,
    pub status: ActiveTaskStatus,
    pub current_goal: String,
    pub last_action: String,
    pub last_verified: Option<String>,
    pub owned_files: Vec<String>,
    pub protected_files: Vec<String>,
    pub running_processes: Vec<TrackedProcess>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug)]
pub enum ActiveTaskError {
    Io(std::io::Error),
    Json(serde_json::Error),
    NoTask,
}

impl Display for ActiveTaskError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Json(e) => write!(f, "json error: {e}"),
            Self::NoTask => write!(f, "no active task found"),
        }
    }
}
impl std::error::Error for ActiveTaskError {}
impl From<std::io::Error> for ActiveTaskError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<serde_json::Error> for ActiveTaskError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

fn now_iso() -> String {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = ts.as_secs();
    let days_since_epoch = (secs / 86400) as i64;
    let day_secs = (secs % 86400) as u32;
    let hours = day_secs / 3600;
    let mins = (day_secs % 3600) / 60;
    let srem = day_secs % 60;
    let mut days = days_since_epoch;
    let mut year = 1970i64;
    loop {
        let yd = if is_leap(year) { 366 } else { 365 };
        if days < yd {
            break;
        }
        days -= yd;
        year += 1;
    }
    let md: [i64; 12] = if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 0i64;
    while month < 12 && days >= md[month as usize] {
        days -= md[month as usize];
        month += 1;
    }
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", year, month + 1, days + 1, hours, mins, srem)
}
fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

#[derive(Debug, Clone)]
pub struct ActiveTaskStore {
    #[allow(dead_code)]
    root: PathBuf,
    runtime_dir: PathBuf,
    task_path: PathBuf,
    recovery_path: PathBuf,
    heartbeat_path: PathBuf,
}

impl ActiveTaskStore {
    #[must_use]
    pub fn new(workspace_root: impl AsRef<Path>) -> Self {
        let root = workspace_root.as_ref().join(SEGO_DIR);
        let rtdir = root.join(RUNTIME_DIR);
        Self {
            root: root.clone(),
            runtime_dir: rtdir.clone(),
            task_path: rtdir.join(ACTIVE_TASK_FILE),
            recovery_path: rtdir.join(RECOVERY_PROMPT_FILE),
            heartbeat_path: rtdir.join(HEARTBEAT_FILE),
        }
    }
    pub fn init(&self) -> Result<(), ActiveTaskError> {
        fs::create_dir_all(&self.runtime_dir)?;
        Ok(())
    }

    pub fn start_task(
        &self,
        task_id: &str,
        goal: &str,
        repo: Option<&str>,
        branch: Option<&str>,
        commit: Option<&str>,
        protected_files: Vec<String>,
    ) -> Result<ActiveTask, ActiveTaskError> {
        self.init()?;
        let now = now_iso();
        let task = ActiveTask {
            task_id: task_id.to_string(),
            repo: repo.map(str::to_string),
            branch: branch.map(str::to_string),
            commit: commit.map(str::to_string),
            remote: None,
            status: ActiveTaskStatus::Running,
            current_goal: goal.to_string(),
            last_action: format!("Task started: {goal}"),
            last_verified: None,
            owned_files: Vec::new(),
            protected_files,
            running_processes: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
            completed_at: None,
        };
        let json = serde_json::to_string_pretty(&task)?;
        fs::write(&self.task_path, &json)?;
        Ok(task)
    }

    pub fn load_task(&self) -> Result<ActiveTask, ActiveTaskError> {
        if !self.task_path.exists() {
            return Err(ActiveTaskError::NoTask);
        }
        let data = fs::read_to_string(&self.task_path)?;
        Ok(serde_json::from_str(&data)?)
    }

    #[must_use]
    pub fn has_active_task(&self) -> bool {
        self.task_path.exists()
    }

    pub fn update_task(
        &self,
        updater: impl FnOnce(&mut ActiveTask),
    ) -> Result<(), ActiveTaskError> {
        let mut task = self.load_task()?;
        updater(&mut task);
        task.updated_at = now_iso();
        let json = serde_json::to_string_pretty(&task)?;
        fs::write(&self.task_path, &json)?;
        self.write_recovery_prompt(&task).ok();
        Ok(())
    }

    pub fn complete_task(&self) -> Result<(), ActiveTaskError> {
        self.update_task(|t| {
            t.status = ActiveTaskStatus::Completed;
            t.completed_at = Some(now_iso());
        })
    }
    pub fn fail_task(&self, reason: &str) -> Result<(), ActiveTaskError> {
        self.update_task(|t| {
            t.status = ActiveTaskStatus::Failed;
            t.last_action = format!("Task failed: {reason}");
            t.completed_at = Some(now_iso());
        })
    }
    pub fn interrupt_task(&self) -> Result<(), ActiveTaskError> {
        if self.has_active_task() {
            self.update_task(|t| {
                t.status = ActiveTaskStatus::Interrupted;
                t.last_action = format!("Interrupted at {}", now_iso());
            })?;
        }
        Ok(())
    }
    pub fn clear_task(&self) -> Result<(), ActiveTaskError> {
        if self.task_path.exists() {
            fs::remove_file(&self.task_path)?;
        }
        Ok(())
    }

    pub fn track_process(
        &self,
        pid: u32,
        command: &str,
        cwd: &str,
        port: Option<u16>,
        purpose: &str,
    ) -> Result<(), ActiveTaskError> {
        self.update_task(|t| {
            t.running_processes.push(TrackedProcess {
                pid,
                command: command.to_string(),
                cwd: cwd.to_string(),
                port,
                start_time: now_iso(),
                status: "running".to_string(),
                purpose: purpose.to_string(),
            });
        })
    }
    pub fn untrack_process(&self, pid: u32) -> Result<(), ActiveTaskError> {
        self.update_task(|t| {
            for p in &mut t.running_processes {
                if p.pid == pid {
                    p.status = "stopped".to_string();
                }
            }
        })
    }

    pub fn heartbeat(&self, last_tool: &str, last_message: &str) -> Result<(), ActiveTaskError> {
        self.init()?;
        let hb = serde_json::json!({ "last_heartbeat": now_iso(), "last_tool": last_tool, "last_message": last_message, "status": "running" });
        fs::write(&self.heartbeat_path, serde_json::to_string_pretty(&hb)?)?;
        Ok(())
    }
    #[must_use]
    pub fn read_heartbeat(&self) -> Option<serde_json::Value> {
        fs::read_to_string(&self.heartbeat_path).ok().and_then(|s| serde_json::from_str(&s).ok())
    }

    fn write_recovery_prompt(&self, task: &ActiveTask) -> Result<(), ActiveTaskError> {
        let procs: Vec<String> = task
            .running_processes
            .iter()
            .filter(|p| p.status == "running")
            .map(|p| {
                format!(
                    "- PID {}: {} (cwd: {}, port: {}) - {}",
                    p.pid,
                    p.command,
                    p.cwd,
                    p.port.map_or("none".to_string(), |x| x.to_string()),
                    p.purpose
                )
            })
            .collect();
        let owned = if task.owned_files.is_empty() {
            "none".to_string()
        } else {
            task.owned_files.join(", ")
        };
        let prot = if task.protected_files.is_empty() {
            "none".to_string()
        } else {
            task.protected_files.join(", ")
        };
        let content = format!(
            "# Recovery Prompt

Current goal: {}
Repo: {}
Branch: {}
Commit: {}
Status: {}
Last action: {}
Last verified: {}

Running processes:
{}

Files changed: {}
Protected files: {}

Next step: Resume from last action.
",
            task.current_goal,
            task.repo.as_deref().unwrap_or("unknown"),
            task.branch.as_deref().unwrap_or("unknown"),
            task.commit.as_deref().unwrap_or("unknown"),
            task.status,
            task.last_action,
            task.last_verified.as_deref().unwrap_or("none"),
            if procs.is_empty() {
                "none".to_string()
            } else {
                procs.join(
                    "
",
                )
            },
            owned,
            prot
        );
        fs::write(&self.recovery_path, &content)?;
        Ok(())
    }
    pub fn write_recovery_prompt_public(&self) -> Result<String, ActiveTaskError> {
        let task = self.load_task()?;
        self.write_recovery_prompt(&task)?;
        fs::read_to_string(&self.recovery_path).map_err(ActiveTaskError::Io)
    }

    pub fn write_state_md(&self, project_root: &Path) -> Result<String, ActiveTaskError> {
        let sd = project_root.join(SEGO_DIR);
        fs::create_dir_all(&sd)?;
        let sp = sd.join("STATE.md");
        let task = self.load_task().unwrap_or_else(|_| ActiveTask {
            task_id: "N/A".to_string(),
            repo: None,
            branch: None,
            commit: None,
            remote: None,
            status: ActiveTaskStatus::Created,
            current_goal: "No active task".to_string(),
            last_action: "N/A".to_string(),
            last_verified: None,
            owned_files: Vec::new(),
            protected_files: Vec::new(),
            running_processes: Vec::new(),
            created_at: now_iso(),
            updated_at: now_iso(),
            completed_at: None,
        });
        let prot = if task.protected_files.is_empty() {
            "none".to_string()
        } else {
            task.protected_files.join(", ")
        };
        let c = format!(
            "# Project State

canonical_repo: {}
current_commit: {}
current_branch: {}
remote: {}
last_tests: {}
protected_files: {}
open_risks: {}
next_task: {}
last_action: {}
",
            project_root.display(),
            task.commit.as_deref().unwrap_or("unknown"),
            task.branch.as_deref().unwrap_or("unknown"),
            task.remote.as_deref().unwrap_or("unknown"),
            task.last_verified.as_deref().unwrap_or("not yet"),
            prot,
            task.status,
            task.current_goal,
            task.last_action
        );
        fs::write(&sp, &c)?;
        Ok(c)
    }

    pub fn write_handoff_latest(&self, project_root: &Path) -> Result<String, ActiveTaskError> {
        let sd = project_root.join(SEGO_DIR);
        fs::create_dir_all(&sd)?;
        let hp = sd.join("HANDOFF_LATEST.md");
        let task = match self.load_task() {
            Ok(t) => t,
            Err(ActiveTaskError::NoTask) => {
                let c = "# Latest Handoff

No active task.
";
                fs::write(&hp, c)?;
                return Ok(c.to_string());
            }
            Err(e) => return Err(e),
        };
        let procs: Vec<String> = task
            .running_processes
            .iter()
            .filter(|p| p.status == "running")
            .map(|p| format!("- PID {}: {} ({})", p.pid, p.command, p.purpose))
            .collect();
        let owned = if task.owned_files.is_empty() {
            "none".to_string()
        } else {
            task.owned_files.join(
                "
",
            )
        };
        let prot = if task.protected_files.is_empty() {
            "none".to_string()
        } else {
            task.protected_files.join(", ")
        };
        let c = format!(
            "# Latest Handoff

Current goal: {}
Canonical repo: {}
Current commit: {}
Current branch: {}
Last action: {}
Last verification: {}
Status: {}

Running processes:
{}

Files changed:
{}

Protected files: {}

Open risks: Check above
Next step: Resume from recovery_prompt.md
",
            task.current_goal,
            task.repo.as_deref().unwrap_or("unknown"),
            task.commit.as_deref().unwrap_or("unknown"),
            task.branch.as_deref().unwrap_or("unknown"),
            task.last_action,
            task.last_verified.as_deref().unwrap_or("none"),
            task.status,
            if procs.is_empty() {
                "none".to_string()
            } else {
                procs.join(
                    "
",
                )
            },
            owned,
            prot
        );
        fs::write(&hp, &c)?;
        Ok(c)
    }
}

#[must_use]
pub fn generate_task_id(prefix: &str) -> String {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    format!("{}-{:08x}-{:04x}", prefix, ts.as_secs(), ts.subsec_nanos() >> 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rand_id() -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0)
    }

    #[test]
    fn starts_and_loads_task() {
        let tmp = std::env::temp_dir().join(format!("sego-at-{}", rand_id()));
        let store = ActiveTaskStore::new(&tmp);
        let task = store
            .start_task(
                "test-1",
                "test goal",
                Some("/repo"),
                Some("main"),
                Some("abc"),
                vec!["s.py".to_string()],
            )
            .unwrap();
        assert_eq!(task.task_id, "test-1");
        assert!(store.has_active_task());
        let lo = store.load_task().unwrap();
        assert_eq!(lo.protected_files, vec!["s.py"]);
        fs::remove_dir_all(tmp.join(".sego")).ok();
    }

    #[test]
    fn completes_task() {
        let tmp = std::env::temp_dir().join(format!("sego-at-{}", rand_id()));
        let store = ActiveTaskStore::new(&tmp);
        store.start_task("t2", "g", None, None, None, vec![]).unwrap();
        store.complete_task().unwrap();
        assert_eq!(store.load_task().unwrap().status, ActiveTaskStatus::Completed);
        fs::remove_dir_all(tmp.join(".sego")).ok();
    }

    #[test]
    fn writes_recovery_prompt() {
        let tmp = std::env::temp_dir().join(format!("sego-at-{}", rand_id()));
        let store = ActiveTaskStore::new(&tmp);
        store.start_task("rp", "build", Some("/r"), None, None, vec![]).unwrap();
        let rp = store.write_recovery_prompt_public().unwrap();
        assert!(rp.contains("build"));
        assert!(rp.contains("/r"));
        fs::remove_dir_all(tmp.join(".sego")).ok();
    }

    #[test]
    fn tracks_processes() {
        let tmp = std::env::temp_dir().join(format!("sego-at-{}", rand_id()));
        let store = ActiveTaskStore::new(&tmp);
        store.start_task("p", "g", None, None, None, vec![]).unwrap();
        store.track_process(1234, "py srv", "/a", Some(8080), "be").unwrap();
        let t = store.load_task().unwrap();
        assert_eq!(t.running_processes.len(), 1);
        assert_eq!(t.running_processes[0].port, Some(8080));
        store.untrack_process(1234).unwrap();
        assert_eq!(store.load_task().unwrap().running_processes[0].status, "stopped");
        fs::remove_dir_all(tmp.join(".sego")).ok();
    }

    #[test]
    fn writes_state_and_handoff() {
        let tmp = std::env::temp_dir().join(format!("sego-at-{}", rand_id()));
        let store = ActiveTaskStore::new(&tmp);
        store
            .start_task(
                "sh",
                "wsf",
                Some("/prj"),
                Some("fx"),
                Some("d45"),
                vec!["c.py".to_string()],
            )
            .unwrap();
        let s = store.write_state_md(&tmp).unwrap();
        // The project_root.display() gives OS-native path; just check it contains the expected content
        assert!(s.contains("wsf") || s.contains("prj"));
        let h = store.write_handoff_latest(&tmp).unwrap();
        assert!(h.contains("wsf"));
        assert!(tmp.join(".sego").join("STATE.md").exists());
        assert!(tmp.join(".sego").join("HANDOFF_LATEST.md").exists());
        fs::remove_dir_all(tmp.join(".sego")).ok();
    }

    #[test]
    fn heartbeat_works() {
        let tmp = std::env::temp_dir().join(format!("sego-at-{}", rand_id()));
        let store = ActiveTaskStore::new(&tmp);
        store.heartbeat("shell", "checking").unwrap();
        let hb = store.read_heartbeat().unwrap();
        assert_eq!(hb["last_tool"], "shell");
        fs::remove_dir_all(tmp.join(".sego")).ok();
    }

    #[test]
    fn no_task_errs() {
        let tmp = std::env::temp_dir().join(format!("sego-at-{}", rand_id()));
        let store = ActiveTaskStore::new(&tmp);
        assert!(!store.has_active_task());
        assert!(store.load_task().is_err());
    }

    #[test]
    fn generates_task_ids() {
        let id1 = generate_task_id("sego");
        let id2 = generate_task_id("sego");
        assert!(id1.starts_with("sego-"));
        assert!(id2.starts_with("sego-"));
        assert_eq!(id1.split('-').count(), 3);
        assert_eq!(id2.split('-').count(), 3);
    }
}
