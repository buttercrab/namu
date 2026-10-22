//! Every value in a workflow's IR has a type; structs and enums are defined once, by shape.

use namu::prelude::*;
use namu_core::types::{Def, Type};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, namu::Data)]
pub struct Page {
    pub url: String,
    pub links: Vec<Link>,
    pub status: Status,
}

#[derive(Clone, Serialize, Deserialize, namu::Data)]
pub struct Link(pub String, pub Option<u16>);

/// A tree: a type that contains itself.
#[derive(Clone, Serialize, Deserialize, namu::Data)]
pub struct Outline {
    pub title: String,
    pub children: Vec<Outline>,
}

#[derive(Clone, Serialize, Deserialize, namu::Data)]
pub enum Status {
    Ok,
    Moved { to: String },
    Failed(u16),
}

#[task]
fn fetch(url: String) -> Result<Page>;

#[task]
fn outline(page: Page) -> Result<Outline>;

fn name(ty: &Type) -> &str {
    match ty {
        Type::Named(name) => name,
        other => panic!("{other:?} is not named"),
    }
}

#[test]
fn values_are_typed_and_named_types_defined_by_shape() {
    #[workflow]
    fn wf() -> Outline {
        let page = fetch("https://example.com");
        outline(page)
    }

    let ir = wf().to_serializable("typed".into());
    // The literal, the page, the outline.
    assert_eq!(ir.types.len(), 3);
    assert_eq!(ir.types[0], Type::String);
    let (page, outline) = (name(&ir.types[1]), name(&ir.types[2]));
    assert!(page.ends_with("::Page"), "{page}");

    let Def::Struct(fields) = &ir.defs[page] else {
        panic!("Page is a struct")
    };
    let link = format!("{}::Link", page.rsplit_once("::").unwrap().0);
    let status = format!("{}::Status", page.rsplit_once("::").unwrap().0);
    assert_eq!(
        fields,
        &[
            ("url".to_string(), Type::String),
            (
                "links".to_string(),
                Type::List(Box::new(Type::Named(link.clone())))
            ),
            ("status".to_string(), Type::Named(status.clone())),
        ]
    );
    // A tuple struct's fields are named by position.
    assert_eq!(
        ir.defs[&link],
        Def::Struct(vec![
            ("0".to_string(), Type::String),
            (
                "1".to_string(),
                Type::Option(Box::new(Type::Int {
                    bits: 16,
                    signed: false
                }))
            ),
        ])
    );
    assert_eq!(
        ir.defs[&status],
        Def::Enum(vec![
            ("Ok".to_string(), vec![]),
            ("Moved".to_string(), vec![("to".to_string(), Type::String)]),
            (
                "Failed".to_string(),
                vec![(
                    "0".to_string(),
                    Type::Int {
                        bits: 16,
                        signed: false
                    }
                )]
            ),
        ])
    );
    // A type that contains itself refers to itself by name.
    assert_eq!(
        ir.defs[outline],
        Def::Struct(vec![
            ("title".to_string(), Type::String),
            (
                "children".to_string(),
                Type::List(Box::new(Type::Named(outline.to_string())))
            ),
        ])
    );
    // And it all survives the IR's JSON.
    let json = serde_json::to_string(&ir).unwrap();
    assert_eq!(
        serde_json::from_str::<namu_core::ir::Workflow>(&json).unwrap(),
        ir
    );
}

#[test]
fn fan_out_items_collected_lists_and_errors_are_typed() {
    #[task]
    fn urls() -> Result<Vec<String>>;

    #[task]
    fn count(pages: Vec<Page>) -> Result<usize>;

    #[workflow]
    fn wf() -> usize {
        let url = urls();
        let page = match attempt(fetch(url)) {
            Ok(page) => page,
            Err(message) => fetch(message),
        };
        count(page)
    }

    let ir = wf().to_serializable("fan".into());
    let list_of = |ty: Type| Type::List(Box::new(ty));
    let page = ir
        .types
        .iter()
        .find_map(|ty| {
            matches!(ty, Type::Named(name) if name.ends_with("::Page")).then(|| ty.clone())
        })
        .expect("a page");
    for expected in [
        list_of(Type::String), // urls()
        Type::String,          // the fanned-out url, and the caught error
        list_of(page.clone()), // the collected pages
        Type::Int {
            bits: 64,
            signed: false,
        }, // count
    ] {
        assert!(
            ir.types.contains(&expected),
            "no {expected:?} in {:?}",
            ir.types
        );
    }
}
