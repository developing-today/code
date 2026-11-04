//! Error type for atlas-core. Messages that correspond to the
//! Python's `die()` texts keep the Python's wording, so CLI surfaces
//! (step 3) print what the Python prints today.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A user-facing fatal, mirroring a Python `die()` message.
    #[error("{0}")]
    Fatal(String),
    /// A backend/transport/SQL failure.
    #[error("{0}")]
    Backend(String),
    /// Query embedding failure (including atlas-embed's).
    #[error("{0}")]
    Embed(String),
}

impl Error {
    pub fn fatal(msg: impl Into<String>) -> Error {
        Error::Fatal(msg.into())
    }
    pub fn backend(msg: impl Into<String>) -> Error {
        Error::Backend(msg.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
