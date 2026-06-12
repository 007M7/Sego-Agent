use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationScope {
    Auto,
    Fast,
    Full,
}

impl VerificationScope {
    pub fn parse(input: Option<&str>) -> Result<Self, VerificationScopeParseError> {
        let Some(raw) = input.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(Self::Auto);
        };

        match raw {
            "auto" | "default" => Ok(Self::Auto),
            "fast" | "quick" | "targeted" => Ok(Self::Fast),
            "full" | "workspace" | "all" => Ok(Self::Full),
            value if value.starts_with('-') => Err(VerificationScopeParseError::UnsupportedFlag {
                value: value.to_string(),
            }),
            value => Err(VerificationScopeParseError::UnknownScope {
                value: value.to_string(),
            }),
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Fast => "fast",
            Self::Full => "full",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationScopeParseError {
    UnsupportedFlag { value: String },
    UnknownScope { value: String },
}

impl fmt::Display for VerificationScopeParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFlag { value } => write!(
                formatter,
                "unsupported /verify scope flag `{value}` (expected auto, fast, or full)"
            ),
            Self::UnknownScope { value } => write!(
                formatter,
                "unknown /verify scope `{value}` (expected auto, fast, or full)"
            ),
        }
    }
}

impl std::error::Error for VerificationScopeParseError {}

#[cfg(test)]
mod tests {
    use super::{VerificationScope, VerificationScopeParseError};

    #[test]
    fn parses_empty_scope_as_auto() {
        assert_eq!(VerificationScope::parse(None), Ok(VerificationScope::Auto));
        assert_eq!(VerificationScope::parse(Some("  ")), Ok(VerificationScope::Auto));
    }

    #[test]
    fn parses_named_scopes() {
        assert_eq!(VerificationScope::parse(Some("auto")), Ok(VerificationScope::Auto));
        assert_eq!(VerificationScope::parse(Some("quick")), Ok(VerificationScope::Fast));
        assert_eq!(VerificationScope::parse(Some("workspace")), Ok(VerificationScope::Full));
    }

    #[test]
    fn rejects_unknown_scopes_and_flags() {
        assert_eq!(
            VerificationScope::parse(Some("src/lib.rs")),
            Err(VerificationScopeParseError::UnknownScope {
                value: "src/lib.rs".to_string(),
            })
        );
        assert_eq!(
            VerificationScope::parse(Some("--json")),
            Err(VerificationScopeParseError::UnsupportedFlag {
                value: "--json".to_string(),
            })
        );
    }
}
