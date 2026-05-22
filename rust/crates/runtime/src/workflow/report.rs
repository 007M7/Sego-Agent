//! Session report generation from workflow data.
//!
//! Produces human-readable reports and machine-readable JSON
//! from the workflow snapshot collected during a session.

use std::fmt::{Display, Formatter};

use super::WorkflowSnapshot;
use crate::green_contract::GreenLevel;

/// A summary of the session's key metrics.
#[derive(Debug, Clone, Default)]
pub struct SessionSummary {
    pub duration_display: String,
    pub total_events: u32,
    pub event_types: Vec<String>,
    pub failures: u32,
    pub recoveries: u32,
    pub green_level: Option<String>,
}

/// Describes the efficiency trend compared to history.
#[derive(Debug, Clone)]
pub enum EfficiencyTrend {
    Improving { delta_pct: f64 },
    Stable,
    Declining { delta_pct: f64 },
}

impl Display for EfficiencyTrend {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Improving { delta_pct } => {
                write!(f, "↑ improving ({delta_pct:.0}% above avg)")
            }
            Self::Stable => write!(f, "→ stable"),
            Self::Declining { delta_pct } => {
                write!(f, "↓ declining ({delta_pct:.0}% below avg)")
            }
        }
    }
}

/// A complete session report with metrics and suggestions.
#[derive(Debug, Clone)]
pub struct SessionReport {
    pub session_id: String,
    pub session_summary: SessionSummary,
    pub efficiency_score: f64,
    pub failure_count: u32,
    pub recovery_attempts: u32,
    pub recovery_successes: u32,
    pub green_level: Option<GreenLevel>,
    pub suggestions: Vec<String>,
}

impl SessionReport {
    /// Generate a report from a completed workflow snapshot.
    #[must_use]
    pub fn from_snapshot(
        snapshot: &WorkflowSnapshot,
        historical_avg_efficiency: Option<f64>,
    ) -> Self {
        let mut suggestions = Vec::new();

        let event_types: Vec<String> = snapshot.lane_event_stats.keys().cloned().collect();

        let summary = SessionSummary {
            duration_display: compute_duration_display(
                snapshot.started_at.as_deref(),
                snapshot.finished_at.as_deref(),
            ),
            total_events: snapshot.lane_events.len() as u32,
            event_types,
            failures: snapshot.failure_count,
            recoveries: snapshot.recovery_stats.successes,
            green_level: snapshot.green_level.map(|g| g.to_string()),
        };

        let mut report = Self {
            session_id: snapshot.session_id.clone(),
            session_summary: summary,
            efficiency_score: snapshot.efficiency_score.unwrap_or(0.0),
            failure_count: snapshot.failure_count,
            recovery_attempts: snapshot.recovery_stats.attempts,
            recovery_successes: snapshot.recovery_stats.successes,
            green_level: snapshot.green_level,
            suggestions: Vec::new(),
        };

        // Generate suggestions
        if let Some(avg) = historical_avg_efficiency {
            if report.efficiency_score < avg {
                suggestions.push(format!(
                    "Efficiency {:.0}% is below your average ({:.0}%). Review failure patterns.",
                    report.efficiency_score, avg
                ));
            } else if report.efficiency_score > avg + 5.0 {
                suggestions.push(
                    "Above-average efficiency! Current practices are working well.".to_string(),
                );
            }
        }

        if report.failure_count == 0 && report.recovery_attempts == 0 {
            suggestions.push(
                "Zero failures and zero recoveries — session ran clean. Consider raising your Green Contract level.".to_string(),
            );
        }

        if report.recovery_attempts > 0 && report.recovery_successes == report.recovery_attempts {
            suggestions.push(
                "All recovery attempts succeeded — recovery recipes are working effectively."
                    .to_string(),
            );
        }

        if report.recovery_attempts > report.recovery_successes && report.recovery_attempts > 0 {
            suggestions.push(format!(
                "{} of {} recovery attempts failed. Consider reviewing recovery recipes for the affected scenarios.",
                report.recovery_attempts - report.recovery_successes,
                report.recovery_attempts
            ));
        }

        if snapshot.recovery_stats.escalations > 0 {
            suggestions.push(format!(
                "{} recovery escalation(s) occurred — these required human intervention. Review escalation scenarios.",
                snapshot.recovery_stats.escalations
            ));
        }

        if let Some(ref green_level) = report.green_level {
            if *green_level < GreenLevel::Workspace {
                suggestions.push(format!(
                    "Green level is {green_level}. Consider upgrading to workspace-level testing for stronger quality guarantees."
                ));
            }
        }

        report.suggestions = suggestions;
        report
    }

    /// Render the report as a human-readable string.
    #[must_use]
    pub fn render(&self) -> String {
        let mut lines = Vec::new();
        lines.push("╔══════════════════════════════════════════════════╗".to_string());
        lines.push("║        🦞 claw Session Report                   ║".to_string());
        lines.push("╠══════════════════════════════════════════════════╣".to_string());
        lines.push(format!("║ Session:   {}", self.session_id));
        lines.push(format!("║ Duration:  {}", self.session_summary.duration_display));
        lines.push("║──────────────────────────────────────────────────║".to_string());
        lines.push(format!(
            "║ Events:       {:>3} │ Efficiency:    {:>5.0}%",
            self.session_summary.total_events, self.efficiency_score
        ));
        lines.push(format!(
            "║ Failures:     {:>3} │ Recoveries:    {:>5}",
            self.failure_count, self.recovery_successes
        ));
        if let Some(ref level) = self.green_level {
            lines.push(format!("║ Green Level:  {:>3} │", level.to_string()));
        }
        lines.push("║──────────────────────────────────────────────────║".to_string());
        if !self.suggestions.is_empty() {
            lines.push("║ 💡 Suggestions:                                  ║".to_string());
            for suggestion in &self.suggestions {
                for chunk in wrap_text(suggestion, 48) {
                    lines.push(format!("║   • {chunk}"));
                }
            }
        }
        lines.push("╚══════════════════════════════════════════════════╝".to_string());
        lines.join("\n")
    }

    /// Export the report as a JSON value.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "session_id": self.session_id,
            "efficiency_score": self.efficiency_score,
            "failure_count": self.failure_count,
            "recovery_attempts": self.recovery_attempts,
            "recovery_successes": self.recovery_successes,
            "green_level": self.green_level.map(|g| g.as_str()),
            "duration_display": self.session_summary.duration_display,
            "total_events": self.session_summary.total_events,
            "suggestions": self.suggestions,
        })
    }
}

fn compute_duration_display(started_at: Option<&str>, finished_at: Option<&str>) -> String {
    match (started_at, finished_at) {
        (Some(_start), Some(_end)) => {
            // Simple display: use the timestamps directly
            // For a full implementation, parse ISO8601 and compute delta
            // For now, show the time range
            format!("{_start} → {_end}")
        }
        (Some(start), None) => format!("started at {start} (ongoing)"),
        _ => "unknown".to_string(),
    }
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut remaining = text;
    while remaining.len() > width {
        let split_at = remaining[..width].rfind(' ').unwrap_or(width);
        result.push(remaining[..split_at].to_string());
        remaining = remaining[split_at..].trim_start();
    }
    if !remaining.is_empty() {
        result.push(remaining.to_string());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_report_from_snapshot() {
        let mut snap = WorkflowSnapshot::new("test-session-1");
        snap.started_at = Some("2026-05-19T14:30:00Z".to_string());
        snap.finished_at = Some("2026-05-19T14:52:00Z".to_string());
        snap.failure_count = 0;
        snap.compute_efficiency();

        let report = SessionReport::from_snapshot(&snap, Some(78.0));
        assert!(report.efficiency_score > 90.0);
        assert!(!report.suggestions.is_empty());
        assert!(report.suggestions.iter().any(|s| s.contains("Zero failures")));
    }

    #[test]
    fn suggests_improvement_when_below_average() {
        let mut snap = WorkflowSnapshot::new("test-session-2");
        snap.failure_count = 3;
        snap.efficiency_score = Some(50.0);

        let report = SessionReport::from_snapshot(&snap, Some(85.0));
        assert!(report.suggestions.iter().any(|s| s.contains("below your average")));
    }

    #[test]
    fn flags_escalations() {
        let mut snap = WorkflowSnapshot::new("test-session-3");
        snap.recovery_stats.escalations = 2;
        snap.compute_efficiency();

        let report = SessionReport::from_snapshot(&snap, None);
        assert!(report.suggestions.iter().any(|s| s.contains("escalation")));
    }

    #[test]
    fn renders_full_report_without_panicking() {
        let mut snap = WorkflowSnapshot::new("render-test");
        snap.started_at = Some("2026-05-19T14:30:00Z".to_string());
        snap.finished_at = Some("2026-05-19T14:52:00Z".to_string());
        snap.failure_count = 1;
        snap.recovery_stats.attempts = 1;
        snap.recovery_stats.successes = 1;
        snap.green_level = Some(GreenLevel::Workspace);
        snap.compute_efficiency();

        let report = SessionReport::from_snapshot(&snap, Some(85.0));
        let rendered = report.render();
        assert!(rendered.contains("🦞 claw Session Report"));
        assert!(rendered.contains("workspace"));
    }

    #[test]
    fn exports_json_report() {
        let mut snap = WorkflowSnapshot::new("json-test");
        snap.started_at = Some("2026-05-19T14:30:00Z".to_string());
        snap.compute_efficiency();

        let report = SessionReport::from_snapshot(&snap, None);
        let json = report.to_json();
        assert_eq!(json["session_id"], "json-test");
        assert!(json["efficiency_score"].as_f64().is_some());
    }
}
