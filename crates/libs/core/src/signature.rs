//! Tasks' signatures, and the check of a workflow against them: whether what each call gives a
//! task can be read as what the task takes, and what the task gives back as what the workflow
//! takes from it. Values cross as MessagePack (a struct as a map of its fields, a tuple or a list
//! as an array), so what matters is their shapes, not their names: a workflow and a task written
//! in different crates, or different languages, agree when their shapes do.
//!
//! The check is as lenient as the wire. A reader ignores fields it does not know, and a field it
//! knows may be missing if it is an `Option`. Integers of any width are one kind, as they are in
//! MessagePack, and a float reads an integer: a value that does not fit fails its call when it
//! runs. A type whose shape is unknown is not checked.

use std::collections::{BTreeSet, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ir::{Action, Workflow};
use crate::types::{Def, Defs, Type};

/// What a task takes and gives, as a workflow sees one call of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    /// Its arguments as they cross: `()` for none, the argument for one, a tuple for several. A
    /// batch task's is one item's.
    pub input: Type,
    /// What a call gives: a stream's is the list of its items, a batch task's one item's.
    pub output: Type,
    /// What it fails with, if it says: its own error type, which an attempt of it catches. A
    /// task that does not say fails with a message, a `String`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Type>,
    /// The shapes of the named types in `input`, `output` and `error`.
    #[serde(default, skip_serializing_if = "Defs::is_empty")]
    pub defs: Defs,
}

impl Signature {
    /// A task taking `params` and giving `output`, failing with a message.
    pub fn new(params: Vec<Type>, output: Type, defs: Defs) -> Self {
        Self {
            input: pack(params),
            output,
            error: None,
            defs,
        }
    }

    /// The same task, failing with `error` instead (if it is not `None`).
    pub fn failing_with(mut self, error: Option<Type>) -> Self {
        self.error = error;
        self
    }
}

/// Arguments as they cross: `()` for none, the argument for one, a tuple for several.
pub fn pack(mut types: Vec<Type>) -> Type {
    match types.len() {
        0 => Type::Unit,
        1 => types.pop().expect("one"),
        _ => Type::Tuple(types),
    }
}

/// A type and where its named types are defined.
#[derive(Clone, Copy)]
pub struct Typed<'a> {
    pub ty: &'a Type,
    pub defs: &'a Defs,
}

impl<'a> Typed<'a> {
    pub fn new(ty: &'a Type, defs: &'a Defs) -> Self {
        Self { ty, defs }
    }
}

/// Why a value cannot be read as a type: where in it, and what is wrong there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    /// From the outside in: `argument 2`, `field \`url\``, `item`, ...
    pub path: Vec<String>,
    pub reason: String,
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for part in &self.path {
            write!(f, "{part}: ")?;
        }
        f.write_str(&self.reason)
    }
}

/// Whether a value of `given`'s type can be read as `read`.
pub fn fits(read: Typed<'_>, given: Typed<'_>) -> Result<(), Mismatch> {
    Fit::default().fits(read, given)
}

/// A workflow checked against its tasks' signatures.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// What cannot work, one line each.
    pub problems: Vec<String>,
    /// Tasks called without a signature to check against.
    pub unchecked: BTreeSet<String>,
}

/// Checks every call in `workflow` against the signature of its task, from `signature`. A
/// workflow that records no types is not checked.
pub fn check<'a>(workflow: &Workflow, signature: impl Fn(&str) -> Option<&'a Signature>) -> Report {
    let mut report = Report::default();
    let typed = !workflow.types.is_empty();
    let ty = |id: usize| workflow.types.get(id).unwrap_or(&UNKNOWN);
    for (op, operation) in workflow.operations.iter().enumerate() {
        let (task, args, results, caught): (&str, Vec<&Type>, Vec<&Type>, _) =
            match &operation.action {
                Some(Action::Call(call)) => (
                    &call.task_id,
                    call.inputs.iter().map(|&id| ty(id)).collect(),
                    call.outputs.iter().map(|&id| ty(id)).collect(),
                    // Only a typed catch reads the task's error; any other reads its message.
                    call.catch
                        .filter(|catch| catch.typed)
                        .map(|catch| ty(catch.error)),
                ),
                Some(Action::Collect(collect)) => match &collect.reduce {
                    Some(reduce) => (
                        &reduce.task_id,
                        vec![ty(reduce.init), ty(collect.input)],
                        vec![ty(collect.output)],
                        None,
                    ),
                    None => continue,
                },
                _ => continue,
            };
        let Some(signature) = signature(task).filter(|_| typed) else {
            report.unchecked.insert(task.to_string());
            continue;
        };
        let call = Call {
            workflow: &workflow.defs,
            task: signature,
        };
        let problems = call
            .arguments(&args)
            .err()
            .into_iter()
            .chain(call.results(&results).err())
            .chain(caught.and_then(|error| call.error(error).err()));
        for mismatch in problems {
            report
                .problems
                .push(format!("call to `{task}` (op {op}): {mismatch}"));
        }
    }
    report
}

/// A type with nothing known of it, for a value the workflow records none for.
static UNKNOWN: Type = Type::Named(String::new());

/// One call: the workflow's side and the task's.
struct Call<'a> {
    workflow: &'a Defs,
    task: &'a Signature,
}

impl Call<'_> {
    /// The arguments the workflow gives, read as the task's input.
    fn arguments(&self, args: &[&Type]) -> Result<(), Mismatch> {
        let input = Typed::new(&self.task.input, &self.task.defs);
        let given = |ty| Typed::new(ty, self.workflow);
        match (args, shape(input)) {
            ([], _) => fits(input, given(&Type::Unit)).map_err(|err| err.at("arguments")),
            ([arg], _) => fits(input, given(arg)).map_err(|err| err.at("argument 1")),
            (_, Shape::Tuple(params)) if params.len() == args.len() => params
                .iter()
                .zip(args)
                .enumerate()
                .try_for_each(|(i, (param, arg))| {
                    fits(Typed::new(param, input.defs), given(arg))
                        .map_err(|err| err.at(format!("argument {}", i + 1)))
                }),
            (_, Shape::Tuple(params)) => Err(Mismatch::new(format!(
                "{} arguments are given; the task takes {}",
                args.len(),
                params.len()
            ))),
            _ => {
                let packed = pack(args.iter().map(|&ty| ty.clone()).collect());
                fits(input, given(&packed)).map_err(|err| err.at("arguments"))
            }
        }
    }

    /// What the task fails with, read as the error the workflow catches.
    fn error(&self, caught: &Type) -> Result<(), Mismatch> {
        let error = self.task.error.as_ref().unwrap_or(&Type::String);
        let error = Typed::new(error, &self.task.defs);
        fits(Typed::new(caught, self.workflow), error).map_err(|err| err.at("error"))
    }

    /// What the task gives, read as the results the workflow takes: none, one, or a tuple split
    /// into several.
    fn results(&self, results: &[&Type]) -> Result<(), Mismatch> {
        let output = Typed::new(&self.task.output, &self.task.defs);
        let read = |ty| Typed::new(ty, self.workflow);
        match (results, shape(output)) {
            // Nothing is read.
            ([], _) => Ok(()),
            ([result], _) => fits(read(result), output).map_err(|err| err.at("result")),
            (_, Shape::Tuple(parts)) if parts.len() == results.len() => parts
                .iter()
                .zip(results)
                .enumerate()
                .try_for_each(|(i, (part, result))| {
                    fits(read(result), Typed::new(part, output.defs))
                        .map_err(|err| err.at(format!("result {}", i + 1)))
                }),
            _ => {
                let packed = pack(results.iter().map(|&ty| ty.clone()).collect());
                fits(read(&packed), output).map_err(|err| err.at("results"))
            }
        }
    }
}

impl Mismatch {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            path: Vec::new(),
            reason: reason.into(),
        }
    }

    /// The same mismatch, inside `part`.
    fn at(mut self, part: impl Into<String>) -> Self {
        self.path.insert(0, part.into());
        self
    }
}

/// A type, or the fields of a struct or variant, as they cross.
#[derive(Clone, Copy)]
enum Part<'a> {
    Type(&'a Type),
    Fields(&'a [(String, Type)]),
}

impl<'a> Part<'a> {
    fn with(self, defs: &'a Defs) -> Side<'a> {
        Side { part: self, defs }
    }
}

/// One side of a comparison: a part and where its named types are defined.
#[derive(Clone, Copy)]
struct Side<'a> {
    part: Part<'a>,
    defs: &'a Defs,
}

impl<'a> From<Typed<'a>> for Side<'a> {
    fn from(typed: Typed<'a>) -> Self {
        Part::Type(typed.ty).with(typed.defs)
    }
}

impl<'a> Side<'a> {
    /// A newtype variant's fields as its one field, which is how it crosses.
    fn newtype_field(self) -> Self {
        match self.part {
            Part::Fields([(name, inner)]) if name == "0" => Part::Type(inner).with(self.defs),
            _ => self,
        }
    }
}

/// What a value looks like on the wire.
enum Shape<'a> {
    Unknown,
    Unit,
    Bool,
    Char,
    Int,
    Float,
    String,
    Bytes,
    Option(&'a Type),
    List(&'a Type),
    Map(&'a Type, &'a Type),
    Tuple(Vec<&'a Type>),
    Struct(&'a [(String, Type)]),
    Enum(&'a [(String, Vec<(String, Type)>)]),
}

fn shape(typed: Typed<'_>) -> Shape<'_> {
    shape_of(typed.into())
}

fn shape_of(side: Side<'_>) -> Shape<'_> {
    let ty = match side.part {
        Part::Type(ty) => ty,
        Part::Fields(fields) => return fields_shape(fields, side.defs),
    };
    match ty {
        Type::Unit => Shape::Unit,
        Type::Bool => Shape::Bool,
        Type::Char => Shape::Char,
        Type::Int { .. } => Shape::Int,
        Type::Float { .. } => Shape::Float,
        Type::String => Shape::String,
        Type::Bytes => Shape::Bytes,
        Type::Option(inner) => Shape::Option(inner),
        Type::List(item) => Shape::List(item),
        Type::Map(key, value) => Shape::Map(key, value),
        Type::Tuple(types) => Shape::Tuple(types.iter().collect()),
        Type::Named(name) => match side.defs.get(name) {
            None => Shape::Unknown,
            Some(Def::Struct(fields)) => fields_shape(fields, side.defs),
            Some(Def::Enum(variants)) => Shape::Enum(variants),
        },
    }
}

/// Fields as serde writes them: none as nothing, one positional as the field itself, several
/// positional as a tuple, and named as a map.
fn fields_shape<'a>(fields: &'a [(String, Type)], defs: &'a Defs) -> Shape<'a> {
    match fields {
        [] => Shape::Unit,
        [(name, inner)] if name == "0" => shape_of(Part::Type(inner).with(defs)),
        _ if positional(fields) => Shape::Tuple(fields.iter().map(|(_, ty)| ty).collect()),
        _ => Shape::Struct(fields),
    }
}

fn positional(fields: &[(String, Type)]) -> bool {
    fields
        .iter()
        .enumerate()
        .all(|(i, (name, _))| *name == i.to_string())
}

/// The comparisons under way, so a type that contains itself is compared once.
#[derive(Default)]
struct Fit<'a> {
    under_way: HashSet<(&'a str, &'a str)>,
}

impl<'a> Fit<'a> {
    fn fits(&mut self, read: Typed<'a>, given: Typed<'a>) -> Result<(), Mismatch> {
        self.sides(read.into(), given.into())
    }

    fn sides(&mut self, read: Side<'a>, given: Side<'a>) -> Result<(), Mismatch> {
        let (read, given) = (read.newtype_field(), given.newtype_field());
        if let (Part::Type(Type::Named(a)), Part::Type(Type::Named(b))) = (read.part, given.part)
            && !self.under_way.insert((a, b))
        {
            return Ok(());
        }
        let ty = |side: Side<'a>, ty: &'a Type| Part::Type(ty).with(side.defs);
        let mismatch = || {
            Err(Mismatch::new(format!(
                "{} is read as {}",
                name(given),
                name(read)
            )))
        };
        match (shape_of(read), shape_of(given)) {
            (Shape::Unknown, _) | (_, Shape::Unknown) => Ok(()),
            (Shape::Option(read_inner), Shape::Option(given_inner)) => {
                self.sides(ty(read, read_inner), ty(given, given_inner))
            }
            (Shape::Option(_), Shape::Unit) => Ok(()),
            (Shape::Option(read_inner), _) => self.sides(ty(read, read_inner), given),
            (_, Shape::Option(_)) => Err(Mismatch::new(format!(
                "{} may be none, and is read as {}",
                name(given),
                name(read)
            ))),
            (Shape::Unit, Shape::Unit)
            | (Shape::Bool, Shape::Bool)
            | (Shape::Char, Shape::Char)
            | (Shape::String, Shape::String | Shape::Char)
            | (Shape::Int, Shape::Int)
            | (Shape::Float, Shape::Float | Shape::Int)
            | (Shape::Bytes, Shape::Bytes) => Ok(()),
            (Shape::List(read_item), Shape::List(given_item)) => self
                .sides(ty(read, read_item), ty(given, given_item))
                .map_err(|err| err.at("item")),
            (Shape::List(read_item), Shape::Tuple(parts)) => {
                parts.into_iter().enumerate().try_for_each(|(i, part)| {
                    self.sides(ty(read, read_item), ty(given, part))
                        .map_err(|err| err.at(format!("element {i}")))
                })
            }
            (Shape::Tuple(read_parts), Shape::Tuple(given_parts)) => {
                if read_parts.len() != given_parts.len() {
                    return Err(Mismatch::new(format!(
                        "{} has {} elements, and is read as {} with {}",
                        name(given),
                        given_parts.len(),
                        name(read),
                        read_parts.len()
                    )));
                }
                read_parts
                    .into_iter()
                    .zip(given_parts)
                    .enumerate()
                    .try_for_each(|(i, (read_part, given_part))| {
                        self.sides(ty(read, read_part), ty(given, given_part))
                            .map_err(|err| err.at(format!("element {i}")))
                    })
            }
            (Shape::Map(read_key, read_value), Shape::Map(given_key, given_value)) => {
                self.sides(ty(read, read_key), ty(given, given_key))
                    .map_err(|err| err.at("key"))?;
                self.sides(ty(read, read_value), ty(given, given_value))
                    .map_err(|err| err.at("value"))
            }
            // A struct crosses as a map from its field names.
            (Shape::Map(read_key, read_value), Shape::Struct(fields)) => {
                self.sides(ty(read, read_key), ty(given, &Type::String))
                    .map_err(|err| err.at("key"))?;
                fields.iter().try_for_each(|(field, field_ty)| {
                    self.sides(ty(read, read_value), ty(given, field_ty))
                        .map_err(|err| err.at(format!("field `{field}`")))
                })
            }
            (Shape::Struct(read_fields), Shape::Struct(given_fields)) => {
                read_fields.iter().try_for_each(|(field, read_ty)| {
                    match given_fields.iter().find(|(name, _)| name == field) {
                        Some((_, given_ty)) => self
                            .sides(ty(read, read_ty), ty(given, given_ty))
                            .map_err(|err| err.at(format!("field `{field}`"))),
                        None if optional(ty(read, read_ty)) => Ok(()),
                        None => Err(Mismatch::new(format!(
                            "{} has no field `{field}`, which {} needs",
                            name(given),
                            name(read)
                        ))),
                    }
                })
            }
            (Shape::Enum(read_variants), Shape::Enum(given_variants)) => given_variants
                .iter()
                .try_for_each(|(variant, given_fields)| {
                    let Some((_, read_fields)) =
                        read_variants.iter().find(|(name, _)| name == variant)
                    else {
                        return Err(Mismatch::new(format!(
                            "{} may be `{variant}`, which {} has no variant for",
                            name(given),
                            name(read)
                        )));
                    };
                    self.sides(
                        Part::Fields(read_fields).with(read.defs),
                        Part::Fields(given_fields).with(given.defs),
                    )
                    .map_err(|err| err.at(format!("variant `{variant}`")))
                }),
            _ => mismatch(),
        }
    }
}

/// Whether a reader can do without the value: an `Option`, or something unknown.
fn optional(side: Side<'_>) -> bool {
    matches!(shape_of(side), Shape::Option(_) | Shape::Unknown)
}

/// A side's type, for messages.
fn name(side: Side<'_>) -> String {
    match side.part {
        Part::Type(ty) => format!("`{ty}`"),
        Part::Fields([]) => "a unit variant".to_string(),
        Part::Fields(_) => "the variant's fields".to_string(),
    }
}

#[cfg(test)]
mod tests;
