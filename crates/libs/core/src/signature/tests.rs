use super::*;

const I64: Type = Type::Int {
    bits: 64,
    signed: true,
};
const U8: Type = Type::Int {
    bits: 8,
    signed: false,
};
const F64: Type = Type::Float { bits: 64 };

fn named(name: &str) -> Type {
    Type::Named(name.into())
}

fn option(ty: Type) -> Type {
    Type::Option(Box::new(ty))
}

fn list(ty: Type) -> Type {
    Type::List(Box::new(ty))
}

fn fields(fields: &[(&str, Type)]) -> Vec<(String, Type)> {
    fields
        .iter()
        .map(|(name, ty)| (name.to_string(), ty.clone()))
        .collect()
}

fn structs(defs: &[(&str, &[(&str, Type)])]) -> Defs {
    defs.iter()
        .map(|(name, body)| (name.to_string(), Def::Struct(fields(body))))
        .collect()
}

/// Whether `given` (under `given_defs`) is read as `read` (under `read_defs`), as a message.
fn read(read: &Type, read_defs: &Defs, given: &Type, given_defs: &Defs) -> Result<(), String> {
    fits(Typed::new(read, read_defs), Typed::new(given, given_defs)).map_err(|err| err.to_string())
}

fn plain(reader: &Type, given: &Type) -> Result<(), String> {
    read(reader, &Defs::new(), given, &Defs::new())
}

#[test]
fn scalars_are_read_as_the_wire_allows() {
    assert_eq!(plain(&I64, &U8), Ok(()), "integers are one kind");
    assert_eq!(plain(&F64, &I64), Ok(()), "a float reads an integer");
    assert_eq!(plain(&Type::String, &Type::Char), Ok(()));
    assert_eq!(plain(&I64, &F64), Err("`f64` is read as `i64`".to_string()));
    assert_eq!(
        plain(&Type::String, &Type::Bool),
        Err("`bool` is read as `String`".to_string())
    );
}

#[test]
fn an_option_reads_its_value_and_none_but_is_not_read_as_one() {
    assert_eq!(plain(&option(I64), &I64), Ok(()));
    assert_eq!(plain(&option(I64), &Type::Unit), Ok(()));
    assert_eq!(plain(&option(I64), &option(U8)), Ok(()));
    assert_eq!(
        plain(&I64, &option(I64)),
        Err("`Option<i64>` may be none, and is read as `i64`".to_string())
    );
}

#[test]
fn a_struct_is_read_by_its_fields_names_not_its_name() {
    // The workflow's `Page` and the task's `Doc`: the task needs `url` and `words`, may do
    // without `lang`, and ignores `links`.
    let workflow = structs(&[(
        "wf::Page",
        &[
            ("url", Type::String),
            ("words", U8),
            ("links", list(Type::String)),
        ],
    )]);
    let task = structs(&[(
        "task::Doc",
        &[
            ("words", I64),
            ("url", Type::String),
            ("lang", option(Type::String)),
        ],
    )]);
    let (page, doc) = (named("wf::Page"), named("task::Doc"));
    assert_eq!(read(&doc, &task, &page, &workflow), Ok(()));

    let missing = structs(&[("task::Doc", &[("title", Type::String)])]);
    assert_eq!(
        read(&doc, &missing, &page, &workflow),
        Err("`Page` has no field `title`, which `Doc` needs".to_string())
    );
    let wrong = structs(&[("task::Doc", &[("words", Type::String)])]);
    assert_eq!(
        read(&list(doc), &wrong, &list(page), &workflow),
        Err("item: field `words`: `u8` is read as `String`".to_string())
    );
}

#[test]
fn a_struct_crosses_as_its_wire_shape() {
    let defs = structs(&[
        ("Id", &[("0", Type::String)]),
        ("Span", &[("0", I64), ("1", I64)]),
        ("Point", &[("x", F64), ("y", F64)]),
    ]);
    let ok = |reader: Type, given: Type| read(&reader, &defs, &given, &defs);
    // A newtype is its field; a tuple struct a tuple; a struct with names a map.
    assert_eq!(ok(Type::String, named("Id")), Ok(()));
    assert_eq!(ok(named("Id"), Type::String), Ok(()));
    assert_eq!(ok(Type::Tuple(vec![I64, I64]), named("Span")), Ok(()));
    assert_eq!(ok(list(I64), named("Span")), Ok(()));
    assert_eq!(
        ok(
            Type::Map(Box::new(Type::String), Box::new(F64)),
            named("Point")
        ),
        Ok(())
    );
    assert_eq!(
        ok(Type::Tuple(vec![I64, I64, I64]), named("Span")),
        Err("`Span` has 2 elements, and is read as `(i64, i64, i64)` with 3".to_string())
    );
}

#[test]
fn every_variant_given_must_be_one_the_reader_knows() {
    let variants = |variants: &[(&str, &[(&str, Type)])]| -> Defs {
        let variants = variants
            .iter()
            .map(|(name, body)| (name.to_string(), fields(body)))
            .collect();
        [("Event".to_string(), Def::Enum(variants))].into()
    };
    let event = named("Event");
    let given = variants(&[("Start", &[]), ("Page", &[("0", Type::String)])]);
    let wider = variants(&[
        ("Start", &[]),
        ("Page", &[("0", Type::String)]),
        ("Stop", &[]),
    ]);
    assert_eq!(read(&event, &wider, &event, &given), Ok(()));
    assert_eq!(
        read(&event, &given, &event, &wider),
        Err("`Event` may be `Stop`, which `Event` has no variant for".to_string())
    );
    let other = variants(&[("Start", &[]), ("Page", &[("0", I64)])]);
    assert_eq!(
        read(&event, &other, &event, &given),
        Err("variant `Page`: `String` is read as `i64`".to_string())
    );
}

#[test]
fn a_type_that_contains_itself_is_compared_once() {
    let tree = |item: Type| -> Defs {
        structs(&[("Tree", &[("item", item), ("children", list(named("Tree")))])])
    };
    let tree_ty = named("Tree");
    assert_eq!(read(&tree_ty, &tree(I64), &tree_ty, &tree(U8)), Ok(()));
    assert_eq!(
        read(&tree_ty, &tree(I64), &tree_ty, &tree(Type::String)),
        Err("field `item`: `String` is read as `i64`".to_string())
    );
}

#[test]
fn what_is_unknown_is_not_checked() {
    assert_eq!(plain(&named("Opaque"), &I64), Ok(()));
    assert_eq!(plain(&list(I64), &named("Opaque")), Ok(()));
}

#[test]
fn a_typed_catch_reads_the_task_s_error_and_any_other_its_message() {
    use crate::ir::{Call as IrCall, Catch, Next, Operation};
    // `fetch(url)`, attempted: value 0 is the url, 1 the page, 2 the error.
    let workflow = |error: Type, typed: bool| {
        let call = IrCall {
            task_id: "fetch".into(),
            inputs: vec![0],
            outputs: vec![1],
            catch: Some(Catch {
                error: 2,
                next: 1,
                typed,
            }),
        };
        let mut workflow = Workflow::new(
            "wf".into(),
            vec![
                Operation::new(
                    Vec::new(),
                    Vec::new(),
                    Some(Action::Call(call)),
                    Next::jump(1),
                ),
                Operation::new(Vec::new(), Vec::new(), None, Next::return_unit()),
            ],
        );
        workflow.types = vec![Type::String, Type::String, error];
        workflow.defs = [(
            "Gone".to_string(),
            Def::Enum(vec![("gone".into(), Vec::new())]),
        )]
        .into();
        workflow
    };
    let fetch = |error: Option<Type>| {
        Signature::new(vec![Type::String], Type::String, Defs::new()).failing_with(error)
    };
    let problems = |workflow: Workflow, task: Signature| check(&workflow, |_| Some(&task)).problems;

    // A typed catch reads the task's own error type, by shape.
    assert!(problems(workflow(named("Gone"), true), fetch(Some(Type::String))).len() == 1);
    assert_eq!(
        problems(workflow(Type::Unit, true), fetch(None)),
        ["call to `fetch` (op 0): error: `String` is read as `()`"]
    );
    assert!(problems(workflow(Type::String, true), fetch(None)).is_empty());
    // A catch of messages reads a message, whatever the task's error type.
    assert!(problems(workflow(Type::String, false), fetch(Some(named("Gone")))).is_empty());
}
