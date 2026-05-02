use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("authentication error: {0}")]
    Auth(String),

    #[error("API error: {status} {message}")]
    Api { status: u16, message: String },

    #[error("not found: {0}")]
    NotFound(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("HTTP middleware error: {0}")]
    HttpMiddleware(#[from] reqwest_middleware::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML serialization error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("TOML deserialization error: {0}")]
    TomlDeserialize(#[from] toml::de::Error),

    #[error("invalid API response: {0}")]
    InvalidResponse(String),

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("template error: {0}")]
    Template(String),

    /// `atl jira issue check` found one or more `--require`d fields that are
    /// missing on the issue. The variant carries the field IDs (not display
    /// names) so callers can format a stable, machine-friendly summary.
    #[error("required fields missing: {}", .0.join(", "))]
    CheckFailed(Vec<String>),
}

pub type Result<T> = std::result::Result<T, Error>;

pub mod exit_code {
    pub const RUNTIME_ERROR: i32 = 1;
    pub const NOT_FOUND: i32 = 2;
    pub const CONFIG_ERROR: i32 = 3;
    pub const AUTH_ERROR: i32 = 4;
    pub const INPUT_ERROR: i32 = 5;
}

pub fn exit_code_for_error(err: &anyhow::Error) -> i32 {
    err.downcast_ref::<Error>()
        .map(|e| match e {
            Error::Config(_) => exit_code::CONFIG_ERROR,
            Error::Auth(_) => exit_code::AUTH_ERROR,
            Error::NotFound(_) => exit_code::NOT_FOUND,
            Error::InvalidInput(_) => exit_code::INPUT_ERROR,
            Error::CheckFailed(_) => exit_code::RUNTIME_ERROR,
            _ => exit_code::RUNTIME_ERROR,
        })
        .unwrap_or(exit_code::RUNTIME_ERROR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_config_error() {
        let err: anyhow::Error = Error::Config("bad config".into()).into();
        assert_eq!(exit_code_for_error(&err), exit_code::CONFIG_ERROR);
    }

    #[test]
    fn exit_code_auth_error() {
        let err: anyhow::Error = Error::Auth("unauthorized".into()).into();
        assert_eq!(exit_code_for_error(&err), exit_code::AUTH_ERROR);
    }

    #[test]
    fn exit_code_input_error() {
        let err: anyhow::Error = Error::InvalidInput("bad flag".into()).into();
        assert_eq!(exit_code_for_error(&err), exit_code::INPUT_ERROR);
    }

    #[test]
    fn exit_code_not_found() {
        let err: anyhow::Error = Error::NotFound("missing".into()).into();
        assert_eq!(exit_code_for_error(&err), exit_code::NOT_FOUND);
    }

    #[test]
    fn exit_code_api_error_is_runtime() {
        let err: anyhow::Error = Error::Api {
            status: 500,
            message: "server error".into(),
        }
        .into();
        assert_eq!(exit_code_for_error(&err), exit_code::RUNTIME_ERROR);
    }

    #[test]
    fn exit_code_unknown_error_is_runtime() {
        let err = anyhow::anyhow!("something unexpected");
        assert_eq!(exit_code_for_error(&err), exit_code::RUNTIME_ERROR);
    }

    #[test]
    fn exit_code_template_error_is_runtime() {
        let err: anyhow::Error = Error::Template("bad template".into()).into();
        assert_eq!(exit_code_for_error(&err), exit_code::RUNTIME_ERROR);
    }

    #[test]
    fn exit_code_check_failed_is_runtime() {
        let err: anyhow::Error = Error::CheckFailed(vec!["customfield_10035".into()]).into();
        assert_eq!(exit_code_for_error(&err), exit_code::RUNTIME_ERROR);
    }

    #[test]
    fn check_failed_message_lists_missing_ids() {
        let err = Error::CheckFailed(vec!["a".into(), "b".into()]);
        assert_eq!(err.to_string(), "required fields missing: a, b");
    }
}
