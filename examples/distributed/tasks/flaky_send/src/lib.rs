//! Sends a document to a flaky service, which turns down the first `id % 4` attempts for it
//! (deterministically, so a run's retries are known). Returns a receipt: the id. A send that is
//! turned down fails with a `SendError`, which a workflow that attempts it gets as a value.

use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, namu_guest::Describe)]
pub enum SendError {
    /// The service is busy: try again.
    Unavailable { attempt: i64 },
    /// The service will not take the document.
    Refused(String),
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::Unavailable { attempt } => write!(f, "service unavailable (attempt {attempt})"),
            SendError::Refused(reason) => write!(f, "refused: {reason}"),
        }
    }
}

namu_guest::task! {
    fn flaky_send(id: i64, attempt: i64) -> Result<i64, SendError> {
        if id < 0 {
            return Err(SendError::Refused(format!("no document {id}")));
        }
        if attempt < id % 4 {
            return Err(SendError::Unavailable { attempt });
        }
        Ok(id)
    }
}
