//! Built-in workflow recording and analysis system.
//!
//! Every session automatically records structured Lane Events.
//! At session end, a SessionReport is generated with failure
//! analysis, recovery tracking, and efficiency scoring.

mod report;
mod store;

pub use report::{EfficiencyTrend, SessionReport, SessionSummary};
pub use store::{WorkflowStore, WorkflowStoreError};

use crate::green_contract::GreenLevel;
use crate::lane_events::{LaneEvent, LaneEventName};
use crate::recovery_recipes::{RecoveryEvent, RecoveryResult};

/// Aggregated statistics for one lane event type.
#[derive(Debug, Clone, Default)]
pub struct LaneEventStats {
    pub count: u32,
    pub last_emitted_at: Option<String>,
}

/// Aggregated statistics for recovery activity in a session.
#[derive(Debug, Clone, Default)]
pub struct RecoveryStats {
    pub attempts: u32,
    pub successes: u32,
    pub partial: u32,
    pub escalations: u32,
    pub scenarios_seen: Vec<String>,
}

/// Complete workflow snapshot for the current session.
#[derive(Debug, Clone)]
pub struct WorkflowSnapshot {
    pub session_id: String,
    pub task_description: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub lane_events: Vec<LaneEvent>,
    pub lane_event_stats: std::collections::BTreeMap<String, LaneEventStats>,
    pub recovery_stats: RecoveryStats,
    pub green_level: Option<GreenLevel>,
    pub failure_count: u32,
    pub efficiency_score: Option<f64>,
}

impl WorkflowSnapshot {
    #[must_use]
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            task_description: None,
            started_at: None,
            finished_at: None,
            lane_events: Vec::new(),
            lane_event_stats: std::collections::BTreeMap::new(),
            recovery_stats: RecoveryStats::default(),
            green_level: None,
            failure_count: 0,
            efficiency_score: None,
        }
    }

    /// Record a lane event and update statistics.
    pub fn record_event(&mut self, event: LaneEvent) {
        if event.event == LaneEventName::Started {
            self.started_at = Some(event.emitted_at.clone());
        }
        if event.event == LaneEventName::Finished {
            self.finished_at = Some(event.emitted_at.clone());
        }
        if let Some(failure_class) = &event.failure_class {
            self.failure_count += 1;
            let key = format!("failure_{failure_class:?}");
            let entry = self.lane_event_stats.entry(key).or_default();
            entry.count += 1;
        }

        let key = format!("{:?}", event.event);
        let entry = self.lane_event_stats.entry(key).or_default();
        entry.count += 1;
        entry.last_emitted_at = Some(event.emitted_at.clone());

        self.lane_events.push(event);
    }

    /// Record a recovery event.
    pub fn record_recovery(&mut self, event: &RecoveryEvent) {
        match event {
            RecoveryEvent::RecoveryAttempted { scenario, result, .. } => {
                self.recovery_stats.attempts += 1;
                let scenario_name = scenario.to_string();
                if !self.recovery_stats.scenarios_seen.contains(&scenario_name) {
                    self.recovery_stats.scenarios_seen.push(scenario_name);
                }
                match result {
                    RecoveryResult::Recovered { .. } => {
                        self.recovery_stats.successes += 1;
                    }
                    RecoveryResult::PartialRecovery { .. } => {
                        self.recovery_stats.partial += 1;
                    }
                    RecoveryResult::EscalationRequired { .. } => {
                        self.recovery_stats.escalations += 1;
                    }
                }
            }
            RecoveryEvent::RecoverySucceeded => {}
            RecoveryEvent::RecoveryFailed => {}
            RecoveryEvent::Escalated => {}
        }
    }

    /// Compute an efficiency score (0.0–100.0).
    pub fn compute_efficiency(&mut self) -> f64 {
        let base = 100.0;
        let failure_penalty = f64::from(self.failure_count) * 5.0;
        let recovery_bonus = f64::from(self.recovery_stats.successes) * 3.0;
        let escalation_penalty = f64::from(self.recovery_stats.escalations) * 10.0;

        let score =
            (base - failure_penalty + recovery_bonus - escalation_penalty).clamp(0.0, 100.0);
        self.efficiency_score = Some(score);
        score
    }
}
