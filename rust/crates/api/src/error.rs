use std::env::VarError;
use std::fmt::{Display, Formatter};
use std::time::Duration;

#[derive(Debug)]
pub enum ApiError {
    MissingCredentials {
        provider: &'static str,
        env_vars: &'static [&'static str],
    },
    ExpiredOAuthToken,
    Auth(String),
    InvalidApiKeyEnv(VarError),
    Http(reqwest::Error),
    Io(std::io::Error),
    Json(serde_json::Error),
    Api {
        status: reqwest::StatusCode,
        error_type: Option<String>,
        message: Option<String>,
        body: String,
        retryable: bool,
    },
    RetriesExhausted {
        attempts: u32,
        last_error: Box<ApiError>,
    },
    InvalidSseFrame(&'static str),
    BackoffOverflow {
        attempt: u32,
        base_delay: Duration,
    },
}

impl ApiError {
    #[must_use]
    pub const fn missing_credentials(
        provider: &'static str,
        env_vars: &'static [&'static str],
    ) -> Self {
        Self::MissingCredentials { provider, env_vars }
    }

    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Http(error) => error.is_connect() || error.is_timeout() || error.is_request(),
            Self::Api { retryable, .. } => *retryable,
            Self::RetriesExhausted { last_error, .. } => last_error.is_retryable(),
            Self::MissingCredentials { .. }
            | Self::ExpiredOAuthToken
            | Self::Auth(_)
            | Self::InvalidApiKeyEnv(_)
            | Self::Io(_)
            | Self::Json(_)
            | Self::InvalidSseFrame(_)
            | Self::BackoffOverflow { .. } => false,
        }
    }
}

impl Display for ApiError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingCredentials { provider, env_vars } => write!(
                f,
                "missing {provider} credentials; set {} before calling the {provider} API.\n\n{}",
                env_vars.join(" or "),
                credential_setup_hint(provider, env_vars),
            ),
            Self::ExpiredOAuthToken => {
                write!(f, "saved OAuth token is expired and no refresh token is available")
            }
            Self::Auth(message) => write!(f, "auth error: {message}"),
            Self::InvalidApiKeyEnv(error) => {
                write!(f, "failed to read credential environment variable: {error}")
            }
            Self::Http(error) => write!(f, "http error: {error}"),
            Self::Io(error) => write!(f, "io error: {error}"),
            Self::Json(error) => write!(f, "json error: {error}"),
            Self::Api { status, error_type, message, body, .. } => {
                let auth_hint = authentication_error_hint(*status, error_type, message, body);
                match (error_type, message) {
                    (Some(error_type), Some(message)) => {
                        write!(f, "api returned {status} ({error_type}): {message}{auth_hint}")
                    }
                    _ => write!(f, "api returned {status}: {body}{auth_hint}"),
                }
            }
            Self::RetriesExhausted { attempts, last_error } => {
                write!(f, "api failed after {attempts} attempts: {last_error}")
            }
            Self::InvalidSseFrame(message) => write!(f, "invalid sse frame: {message}"),
            Self::BackoffOverflow { attempt, base_delay } => write!(
                f,
                "retry backoff overflowed on attempt {attempt} with base delay {base_delay:?}"
            ),
        }
    }
}

fn credential_setup_hint(provider: &str, env_vars: &[&str]) -> String {
    if env_vars.contains(&"DEEPSEEK_API_KEY") || provider.eq_ignore_ascii_case("deepseek") {
        "DeepSeek setup:\n  PowerShell: setx DEEPSEEK_API_KEY \"sk-your-deepseek-key\"\n  Then close and reopen the terminal before running sego again.".to_string()
    } else {
        format!(
            "Set the required environment variable, then close and reopen the terminal before running sego again.\nRequired: {}",
            env_vars.join(" or ")
        )
    }
}

fn authentication_error_hint(
    status: reqwest::StatusCode,
    error_type: &Option<String>,
    message: &Option<String>,
    body: &str,
) -> &'static str {
    let status_is_auth = matches!(status.as_u16(), 401 | 403);
    let haystack = format!(
        "{} {} {}",
        error_type.as_deref().unwrap_or_default(),
        message.as_deref().unwrap_or_default(),
        body
    )
    .to_ascii_lowercase();
    let looks_like_auth_error = status_is_auth
        || haystack.contains("authentication")
        || haystack.contains("unauthorized")
        || haystack.contains("invalid api key")
        || haystack.contains("api key")
        || haystack.contains("auth");

    if looks_like_auth_error {
        "\n\nAuthentication failed. If you are using the default DeepSeek model, check DEEPSEEK_API_KEY.\nPowerShell setup:\n  setx DEEPSEEK_API_KEY \"sk-your-deepseek-key\"\nThen close and reopen the terminal before running sego again.\nYou can also run `sego doctor` to inspect local setup."
    } else {
        ""
    }
}

impl std::error::Error for ApiError {}

impl From<reqwest::Error> for ApiError {
    fn from(value: reqwest::Error) -> Self {
        Self::Http(value)
    }
}

impl From<std::io::Error> for ApiError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<VarError> for ApiError {
    fn from(value: VarError) -> Self {
        Self::InvalidApiKeyEnv(value)
    }
}

#[cfg(test)]
mod tests {
    use super::ApiError;

    #[test]
    fn missing_deepseek_credentials_show_setup_hint() {
        let message = ApiError::missing_credentials("DeepSeek", &["DEEPSEEK_API_KEY"]).to_string();

        assert!(message.contains("missing DeepSeek credentials"));
        assert!(message.contains("setx DEEPSEEK_API_KEY"));
        assert!(message.contains("close and reopen the terminal"));
    }

    #[test]
    fn deepseek_authentication_errors_show_setup_hint() {
        let message = ApiError::Api {
            status: reqwest::StatusCode::UNAUTHORIZED,
            error_type: Some("authentication_error".to_string()),
            message: Some("Authentication Fails, Your api key is invalid".to_string()),
            body: String::new(),
            retryable: false,
        }
        .to_string();

        assert!(message.contains("api returned 401 Unauthorized"));
        assert!(message.contains("Authentication failed"));
        assert!(message.contains("setx DEEPSEEK_API_KEY"));
        assert!(message.contains("sego doctor"));
    }
}
