use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewScope {
    Workspace,
    Staged,
    Unstaged,
    Path(PathBuf),
}

impl ReviewScope {
    pub fn parse(input: Option<&str>) -> Result<Self, ReviewScopeParseError> {
        let Some(raw) = input.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(Self::Workspace);
        };

        match raw {
            "workspace" | "all" | "." => Ok(Self::Workspace),
            "staged" | "--staged" | "cached" | "--cached" => Ok(Self::Staged),
            "unstaged" | "--unstaged" | "working" | "worktree" => Ok(Self::Unstaged),
            value if value.starts_with('-') => {
                Err(ReviewScopeParseError::UnsupportedFlag { value: value.to_string() })
            }
            value => Ok(Self::Path(PathBuf::from(value))),
        }
    }

    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Workspace => "workspace".to_string(),
            Self::Staged => "staged".to_string(),
            Self::Unstaged => "unstaged".to_string(),
            Self::Path(path) => format!("path:{}", path.display()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewScopeParseError {
    UnsupportedFlag { value: String },
}

impl fmt::Display for ReviewScopeParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFlag { value } => write!(
                formatter,
                "unsupported /review scope flag `{value}` (expected staged, unstaged, workspace, or a file path)"
            ),
        }
    }
}

impl std::error::Error for ReviewScopeParseError {}

#[cfg(test)]
mod tests {
    use super::{ReviewScope, ReviewScopeParseError};

    #[test]
    fn parses_empty_scope_as_workspace() {
        assert_eq!(ReviewScope::parse(None), Ok(ReviewScope::Workspace));
        assert_eq!(ReviewScope::parse(Some("  ")), Ok(ReviewScope::Workspace));
    }

    #[test]
    fn parses_named_scopes() {
        assert_eq!(ReviewScope::parse(Some("workspace")), Ok(ReviewScope::Workspace));
        assert_eq!(ReviewScope::parse(Some("staged")), Ok(ReviewScope::Staged));
        assert_eq!(ReviewScope::parse(Some("--cached")), Ok(ReviewScope::Staged));
        assert_eq!(ReviewScope::parse(Some("unstaged")), Ok(ReviewScope::Unstaged));
        assert_eq!(ReviewScope::parse(Some("worktree")), Ok(ReviewScope::Unstaged));
    }

    #[test]
    fn parses_path_scope() {
        assert_eq!(
            ReviewScope::parse(Some("rust/crates/runtime/src/lib.rs")),
            Ok(ReviewScope::Path("rust/crates/runtime/src/lib.rs".into()))
        );
    }

    #[test]
    fn rejects_unknown_flags() {
        assert_eq!(
            ReviewScope::parse(Some("--json")),
            Err(ReviewScopeParseError::UnsupportedFlag { value: "--json".to_string() })
        );
    }
}
