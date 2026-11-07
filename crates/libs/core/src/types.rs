//! The types of a workflow's values, as the IR records them (design §5): structural for the
//! standard types, and by name for structs and enums, whose shapes are defined once, in [`Defs`].
//! A named type can refer to itself (`struct Tree { children: Vec<Tree> }`). The model is
//! language-neutral: it is what a UI shows, and what a task in another language is checked
//! against.

use std::any::type_name;
use std::collections::{BTreeMap, HashMap};

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

/// A type that says what it is: how it crosses process boundaries, in the terms of [`Type`].
/// Derive it with `#[derive(Describe)]` (or `#[derive(Data)]`, which describes too); it is
/// implemented for the standard types. A type that does not say is named after its Rust type,
/// with its shape unknown, and is not checked.
pub trait Describe {
    /// The type; named types are defined in `defs`.
    fn describe(defs: &mut Defs) -> Type {
        let _ = defs;
        Type::Named(type_name::<Self>().to_string())
    }
}

macro_rules! scalars {
    ($($t:ty => $type:expr),* $(,)?) => {$(
        impl Describe for $t {
            fn describe(_: &mut Defs) -> Type {
                $type
            }
        }
    )*};
}

const fn int(bits: u8, signed: bool) -> Type {
    Type::Int { bits, signed }
}

scalars!(
    () => Type::Unit,
    bool => Type::Bool,
    char => Type::Char,
    i8 => int(8, true),
    i16 => int(16, true),
    i32 => int(32, true),
    i64 => int(64, true),
    i128 => int(128, true),
    isize => int(64, true),
    u8 => int(8, false),
    u16 => int(16, false),
    u32 => int(32, false),
    u64 => int(64, false),
    u128 => int(128, false),
    usize => int(64, false),
    f32 => Type::Float { bits: 32 },
    f64 => Type::Float { bits: 64 },
    String => Type::String,
);

impl<T: Describe> Describe for Option<T> {
    fn describe(defs: &mut Defs) -> Type {
        Type::Option(Box::new(T::describe(defs)))
    }
}

impl<T: Describe + ?Sized> Describe for Box<T> {
    fn describe(defs: &mut Defs) -> Type {
        T::describe(defs)
    }
}

impl<T: Describe> Describe for Vec<T> {
    fn describe(defs: &mut Defs) -> Type {
        Type::List(Box::new(T::describe(defs)))
    }
}

impl<K: Describe, V: Describe> Describe for BTreeMap<K, V> {
    fn describe(defs: &mut Defs) -> Type {
        Type::Map(Box::new(K::describe(defs)), Box::new(V::describe(defs)))
    }
}

impl<K: Describe, V: Describe, S> Describe for HashMap<K, V, S> {
    fn describe(defs: &mut Defs) -> Type {
        Type::Map(Box::new(K::describe(defs)), Box::new(V::describe(defs)))
    }
}

macro_rules! tuples {
    ($(($($name:ident),+))*) => {$(
        impl<$($name: Describe),+> Describe for ($($name,)+) {
            fn describe(defs: &mut Defs) -> Type {
                Type::Tuple(vec![$($name::describe(defs)),+])
            }
        }
    )*};
}

tuples! {
    (A)
    (A, B)
    (A, B, C)
    (A, B, C, D)
    (A, B, C, D, E)
    (A, B, C, D, E, F)
    (A, B, C, D, E, F, G)
    (A, B, C, D, E, F, G, H)
    (A, B, C, D, E, F, G, H, I)
    (A, B, C, D, E, F, G, H, I, J)
    (A, B, C, D, E, F, G, H, I, J, K)
    (A, B, C, D, E, F, G, H, I, J, K, L)
}
