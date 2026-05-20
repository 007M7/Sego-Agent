//! Anonymous community learning telemetry — opt-in data sharing for
//! improving Sego's recovery recipes, efficiency benchmarks, and API
//! compatibility.
//!
//! ## What is collected (when opted in)
//! - Anonymized device ID (random UUID, not linked to identity)
//! - Session efficiency score and failure counts
//! - Failure type distribution (e.g., "compile: 3, test: 1")
//! - Recovery recipe success rates
//! - Green level achieved
//! - Model family (e.g., "deepseek-v4-pro") and API base domain only
//!
//! ## What is NEVER collected
//! - Conversation content or code
//! - API keys or tokens
//! - File paths or workspace names
//! - Personal identifiers

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::green_contract::GreenLevel;
use crate::workflow::{WorkflowSnapshot};
use crate::recovery_recipes::FailureScenario;

const TELEMETRY_CONFIG_FILE: &str = "telemetry.json";
const REPORT_ENDPOINT: &str = "https://sego-telemetry.example.com/api/v1/report";

/// Anonymous telemetry payload sent to community server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryReport {
    pub device_id: String,
    pub report_version: u32,
    pub session_count: u32,
    pub efficiency_score: f64,
    pub failure_count: u32,
    pub recovery_attempts: u32,
    pub recovery_successes: u32,
    pub recovery_escalations: u32,
    pub green_level: Option<String>,
    pub model_family: String,
    pub api_domain: String,
    pub failures_by_type: Vec<(String, u32)>,
    pub sego_version: String,
    pub platform: String,
    pub timestamp_secs: u64,
}

/// Configuration for community learning telemetry.
#[derive(Debug, Clone)]
pub struct CommunityLearning {
    enabled: bool,
    device_id: String,
    config_dir: PathBuf,
    session_count: u32,
    pending_reports: Vec<TelemetryReport>,
}

impl CommunityLearning {
    #[must_use]
    pub fn new(workspace_root: impl AsRef<Path>) -> Self {
        let config_dir = workspace_root.as_ref().join(".claw");
        let (enabled, device_id, session_count) = Self::load_config(&config_dir);

        Self {
            enabled,
            device_id,
            config_dir,
            session_count,
            pending_reports: Vec::new(),
        }
    }

    /// Enable telemetry for this workspace.
    pub fn enable(&mut self) {
        self.enabled = true;
        let _ = self.save_config();
    }

    /// Disable telemetry for this workspace.
    pub fn disable(&mut self) {
        self.enabled = false;
        let _ = self.save_config();
    }

    /// Check if telemetry is enabled.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Collect an anonymous report from a completed session snapshot.
    pub fn collect_report(
        &mut self,
        snapshot: &WorkflowSnapshot,
        model: &str,
        api_base_url: &str,
    ) -> Option<TelemetryReport> {
        if !self.enabled {
            return None;
        }

        self.session_count += 1;
        let _ = self.save_config();

        // Extract model family (strip version specifics)
        let model_family = model
            .split('/').last().unwrap_or(model)
            .split('@').next().unwrap_or(model)
            .to_string();

        // Extract only domain from API base URL (strip path and credentials)
        let api_domain = api_base_url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/').next().unwrap_or("unknown")
            .split('@').last().unwrap_or("unknown")
            .to_string();

        // Collect failure types
        let mut failures_by_type: Vec<(String, u32)> = Vec::new();
        for (key, stats) in &snapshot.lane_event_stats {
            if key.starts_with("failure_") {
                let failure_type = key.strip_prefix("failure_").unwrap_or(key).to_string();
                failures_by_type.push((failure_type, stats.count));
            }
        }

        let report = TelemetryReport {
            device_id: self.device_id.clone(),
            report_version: 1,
            session_count: self.session_count,
            efficiency_score: snapshot.efficiency_score.unwrap_or(0.0),
            failure_count: snapshot.failure_count,
            recovery_attempts: snapshot.recovery_stats.attempts,
            recovery_successes: snapshot.recovery_stats.successes,
            recovery_escalations: snapshot.recovery_stats.escalations,
            green_level: snapshot.green_level.map(|g| g.to_string()),
            model_family,
            api_domain,
            failures_by_type,
            sego_version: env!("CARGO_PKG_VERSION").to_string(),
            platform: std::env::consts::OS.to_string(),
            timestamp_secs: now_secs(),
        };

        self.pending_reports.push(report.clone());
        Some(report)
    }

    /// Export pending reports as JSON (for offline submission).
    #[must_use]
    pub fn export_pending(&self) -> String {
        serde_json::to_string_pretty(&self.pending_reports).unwrap_or_default()
    }

    /// Clear pending reports.
    pub fn clear_pending(&mut self) {
        self.pending_reports.clear();
    }

    fn load_config(config_dir: &Path) -> (bool, String, u32) {
        let path = config_dir.join(TELEMETRY_CONFIG_FILE);
        match fs::read_to_string(&path) {
            Ok(content) => {
                let parsed: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
                let enabled = parsed["enabled"].as_bool().unwrap_or(false);
                let device_id = parsed["device_id"].as_str().unwrap_or("").to_string();
                let device_id = if device_id.is_empty() {
                    generate_device_id()
                } else {
                    device_id
                };
                let session_count = parsed["session_count"].as_u64().unwrap_or(0) as u32;
                (enabled, device_id, session_count)
            }
            Err(_) => (false, generate_device_id(), 0),
        }
    }

    fn save_config(&self) -> Result<(), std::io::Error> {
        fs::create_dir_all(&self.config_dir)?;
        let path = self.config_dir.join(TELEMETRY_CONFIG_FILE);
        let content = serde_json::json!({
            "enabled": self.enabled,
            "device_id": self.device_id,
            "session_count": self.session_count,
        });
        fs::write(&path, serde_json::to_string_pretty(&content)?)?;
        Ok(())
    }
}

fn generate_device_id() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    nanos.hash(&mut hasher);
    format!("sego-{:016x}", hasher.finish())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn disabled_by_default() {
        let tmp = std::env::temp_dir().join(format!("sego-tel-{}", rand_id()));
        let cl = CommunityLearning::new(&tmp);
        assert!(!cl.is_enabled());
        fs::remove_dir_all(tmp.join(".claw")).ok();
    }

    #[test]
    fn enable_and_disable() {
        let tmp = std::env::temp_dir().join(format!("sego-tel-{}", rand_id()));
        let mut cl = CommunityLearning::new(&tmp);
        assert!(!cl.is_enabled());

        cl.enable();
        assert!(cl.is_enabled());

        // Reload from config
        let cl2 = CommunityLearning::new(&tmp);
        assert!(cl2.is_enabled());

        cl.disable();
        assert!(!cl.is_enabled());

        fs::remove_dir_all(tmp.join(".claw")).ok();
    }

    #[test]
    fn collects_anonymous_report() {
        let tmp = std::env::temp_dir().join(format!("sego-tel-{}", rand_id()));
        let mut cl = CommunityLearning::new(&tmp);
        cl.enable();

        let mut snap = WorkflowSnapshot::new("test-session");
        snap.failure_count = 2;
        snap.recovery_stats.attempts = 2;
        snap.recovery_stats.successes = 1;
        snap.green_level = Some(GreenLevel::Package);
        snap.compute_efficiency();

        let report = cl.collect_report(&snap, "deepseek-v4-pro", "https://api.deepseek.com/anthropic")
            .expect("should generate report");

        assert_eq!(report.failure_count, 2);
        assert_eq!(report.recovery_successes, 1);
        assert_eq!(report.model_family, "deepseek-v4-pro");
        assert_eq!(report.api_domain, "api.deepseek.com");
        assert!(!report.device_id.is_empty());
        assert_eq!(report.platform, std::env::consts::OS);

        fs::remove_dir_all(tmp.join(".claw")).ok();
    }

    #[test]
    fn api_domain_strips_credentials() {
        let tmp = std::env::temp_dir().join(format!("sego-tel-{}", rand_id()));
        let mut cl = CommunityLearning::new(&tmp);
        cl.enable();

        let snap = WorkflowSnapshot::new("test");
        let report = cl.collect_report(&snap, "claude-opus-4-7", "https://user:pass@proxy.example.com/v1/chat")
            .expect("should generate report");

        assert_eq!(report.api_domain, "proxy.example.com");
        fs::remove_dir_all(tmp.join(".claw")).ok();
    }

    #[test]
    fn does_not_collect_when_disabled() {
        let tmp = std::env::temp_dir().join(format!("sego-tel-{}", rand_id()));
        let mut cl = CommunityLearning::new(&tmp);
        let snap = WorkflowSnapshot::new("test");
        assert!(cl.collect_report(&snap, "model", "url").is_none());
    }

    fn rand_id() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }
}
