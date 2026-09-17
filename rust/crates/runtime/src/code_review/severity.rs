use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

impl ReviewSeverity {
    /// Every label, in declaration order. Kept next to the enum so the
    /// contract conformance suite can pin it against the schema enums.
    pub const ALL_LABELS: [&str; 5] = ["critical", "high", "medium", "low", "info"];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Info => "info",
        }
    }
}
