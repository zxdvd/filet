use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Error {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
}
pub type Result<T> = std::result::Result<T, Error>;
impl Error {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            rule_id: None,
        }
    }
    pub fn rule(mut self, id: &str) -> Self {
        self.rule_id = Some(id.into());
        self
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::new("IO_ERROR", e.to_string())
    }
}
impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Self::new("STATE_ERROR", e.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::new("INVALID_DATA", e.to_string())
    }
}
