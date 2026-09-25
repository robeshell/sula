//! Errors returned by the core API. Each variant keeps the message text the UI
//! already understands, and serializes to exactly that string, so shells can
//! pass it through unchanged while native code can match on the kind.

use serde::{Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CoreError {
    /// A library or media item that no longer exists.
    #[error("{kind} not found: {id}")]
    NotFound { kind: &'static str, id: String },
    /// The request itself can't be carried out (empty name, nothing selected …).
    #[error("{0}")]
    Invalid(String),
    /// Something else changed at the same time; retrying may succeed.
    #[error("{0}")]
    Busy(String),
    /// Anything else, with the underlying message.
    #[error("{0}")]
    Failed(String),
}

impl CoreError {
    pub(crate) fn not_found(kind: &'static str, id: &str) -> Self {
        Self::NotFound { kind, id: id.to_string() }
    }

    pub(crate) fn invalid(message: &str) -> Self {
        Self::Invalid(message.to_string())
    }
}

impl From<String> for CoreError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

impl From<CoreError> for String {
    fn from(error: CoreError) -> Self {
        error.to_string()
    }
}

impl Serialize for CoreError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

pub type CoreResult<T> = Result<T, CoreError>;

pub(crate) fn failed(error: impl ToString) -> CoreError {
    CoreError::Failed(error.to_string())
}

/// Filesystem or SQLite work on the blocking pool (see `app::blocking`).
pub(crate) async fn blocking<T: Send + 'static>(work: impl FnOnce() -> CoreResult<T> + Send + 'static) -> CoreResult<T> {
    tokio::task::spawn_blocking(work).await.map_err(failed)?
}

#[cfg(test)]
mod tests {
    use super::CoreError;

    #[test]
    fn messages_and_json_match_the_previous_strings() {
        let missing = CoreError::not_found("media item", "abc");
        assert_eq!(missing.to_string(), "media item not found: abc");
        assert_eq!(serde_json::to_string(&missing).unwrap(), "\"media item not found: abc\"");
        assert_eq!(CoreError::invalid("no items selected").to_string(), "no items selected");
    }
}
