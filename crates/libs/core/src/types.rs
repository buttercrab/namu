//! The types of a workflow's values, as the IR records them (design §5): structural for the
//! standard types, and by name for structs and enums, whose shapes are defined once, in [`Defs`].
//! A named type can refer to itself (`struct Tree { children: Vec<Tree> }`). The model is
//! language-neutral: it is what a UI shows, and what a task in another language is checked
//! against.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Type {
    Unit,
    Bool,
    Char,
    Int {
        bits: u8,
        signed: bool,
    },
    Float {
        bits: u8,
    },
    String,
    Bytes,
    Option(Box<Type>),
    List(Box<Type>),
    Map(Box<Type>, Box<Type>),
    Tuple(Vec<Type>),
    /// A struct or enum, defined in the workflow's [`Defs`] if its shape is known.
    Named(String),
}

/// The shapes of the named types a workflow uses, by name.
pub type Defs = BTreeMap<String, Def>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Def {
    /// Fields by name; a tuple struct's are named `0`, `1`, ...
    Struct(Vec<(String, Type)>),
    /// Variants by name, each with its fields (none for a unit variant).
    Enum(Vec<(String, Vec<(String, Type)>)>),
}

/// Describes a named type: defines it in `defs` the first time, with `define`, and refers to it
/// by name. The name is in `defs` before `define` runs, so a type that contains itself ends.
pub fn named(name: &str, defs: &mut Defs, define: impl FnOnce(&mut Defs) -> Def) -> Type {
    if !defs.contains_key(name) {
        defs.insert(name.to_string(), Def::Struct(Vec::new()));
        let def = define(defs);
        defs.insert(name.to_string(), def);
    }
    Type::Named(name.to_string())
}
