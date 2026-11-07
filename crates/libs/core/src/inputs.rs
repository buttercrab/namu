//! A run's inputs: a value for each of its workflow's parameters, by name. They are given as Rust
//! values in process, or as JSON through the API, and checked against the parameters' types
//! before the run starts, so a wrong input is refused rather than failing a run halfway.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::Value as Json;

use crate::ir::Workflow;
use crate::types::{Defs, Type};
use crate::value::{Data, Value};
use crate::{ValueId, json};

#[derive(Clone, Default)]
pub struct Inputs(BTreeMap<String, Given>);

#[derive(Clone)]
enum Given {
    Value { value: Value, ty: Type },
    Json(Json),
}

impl Inputs {
    pub fn new() -> Self {
        Self::default()
    }

    /// With `value` for the parameter `name`.
    pub fn with<T: Data>(mut self, name: impl Into<String>, value: T) -> Self {
        let ty = T::describe(&mut Defs::new());
        let value = Value::new(value);
        self.0.insert(name.into(), Given::Value { value, ty });
        self
    }

    /// The inputs in a JSON object, by name.
    pub fn from_json(object: serde_json::Map<String, Json>) -> Self {
        Self(
            object
                .into_iter()
                .map(|(name, json)| (name, Given::Json(json)))
                .collect(),
        )
    }

    /// The values of `workflow`'s parameters, each bound to its value id. Every parameter needs a
    /// value of its type, and there is nothing else.
    pub fn bind(mut self, workflow: &Workflow) -> Result<Vec<(ValueId, Value)>, InputError> {
        let mut problems = Vec::new();
        let mut bound = Vec::with_capacity(workflow.inputs.len());
        for input in &workflow.inputs {
            let ty = workflow.types.get(input.output);
            let value = match (self.0.remove(&input.name), ty) {
                (None, _) => {
                    let ty = ty.map(|ty| format!(" ({ty})")).unwrap_or_default();
                    problems.push(format!("`{}`{ty} is missing", input.name));
                    continue;
                }
                (Some(Given::Value { ty: given, .. }), Some(ty)) if given != *ty => {
                    problems.push(format!("`{}`: expected {ty}, got {given}", input.name));
                    continue;
                }
                (Some(Given::Value { value, .. }), _) => value,
                (Some(Given::Json(json)), ty) => {
                    if let Some(ty) = ty
                        && let Err(problem) = json::check(&json, ty, &workflow.defs, &input.name)
                    {
                        problems.push(problem);
                        continue;
                    }
                    match json::to_value(&json) {
                        Ok(value) => value,
                        Err(err) => {
                            problems.push(format!("`{}`: {err:#}", input.name));
                            continue;
                        }
                    }
                }
            };
            bound.push((input.output, value));
        }
        problems.extend(
            self.0
                .into_keys()
                .map(|name| format!("`{name}` is not a parameter of {}", workflow.name)),
        );
        match problems.is_empty() {
            true => Ok(bound),
            false => Err(InputError(problems)),
        }
    }
}

/// What is wrong with a run's inputs: everything, not just the first thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputError(pub Vec<String>);

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid inputs: {}", self.0.join("; "))
    }
}

impl std::error::Error for InputError {}
