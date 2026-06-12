use super::ReviewSeverity;

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewFinding {
    pub severity: ReviewSeverity,
    pub file: String,
    pub line: Option<u32>,
    pub title: String,
    pub evidence: String,
    pub risk: String,
    pub suggestion: String,
    pub confidence: f32,
    pub verification_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewReport {
    pub findings: Vec<ReviewFinding>,
    pub raw_text: String,
}

impl ReviewReport {
    #[must_use]
    pub fn no_findings(raw_text: impl Into<String>) -> Self {
        Self { findings: Vec::new(), raw_text: raw_text.into() }
    }
}
