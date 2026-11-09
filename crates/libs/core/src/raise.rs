//! What a task fails with, as it crosses from the task to the engine: a message, and, for a task
//! that fails with a type of its own, that value as MessagePack, which an attempt of the call
//! catches.
//!
//! `anyhow::Error` is a message. Any type that `Describe`s itself and serializes is the task's own
//! error type; its `Display` is the message.

use std::fmt::Display;

use serde::{Deserialize, Serialize};

use crate::types::{Defs, Describe, Type};

/// A task's own failure, as it crosses: the same over every boundary (WASM, the native ABI, the
/// Python worker's frames).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Raised {
    pub message: String,
    /// The error value, as MessagePack, if the task fails with a type of its own.
    #[serde(default, with = "serde_bytes")]
    pub error: Option<Vec<u8>>,
}

impl From<String> for Raised {
    fn from(message: String) -> Self {
        Self {
            message,
            error: None,
        }
    }
}

/// An error a task can fail with.
pub trait Raise {
    /// The failure, as it crosses.
    fn raise(self) -> Raised;

    /// The type an attempt of the task catches, if it is one of its own (not a message).
    fn error_type(defs: &mut Defs) -> Option<Type>;
}

impl Raise for anyhow::Error {
    fn raise(self) -> Raised {
        format!("{self:#}").into()
    }

    fn error_type(_defs: &mut Defs) -> Option<Type> {
        None
    }
}

impl<E: Describe + Serialize + Display> Raise for E {
    fn raise(self) -> Raised {
        let message = self.to_string();
        match rmp_serde::to_vec_named(&self) {
            Ok(error) => Raised {
                message,
                error: Some(error),
            },
            Err(err) => format!("{message} (and its error value does not encode: {err})").into(),
        }
    }

    fn error_type(defs: &mut Defs) -> Option<Type> {
        Some(E::describe(defs))
    }
}
