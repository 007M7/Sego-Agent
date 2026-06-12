#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReviewSeverity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}
