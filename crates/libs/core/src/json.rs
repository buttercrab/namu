//! JSON at the edges: the IR keeps its literals as JSON text, and a run's inputs arrive as JSON
//! through the API. Both become values here, as MessagePack, the way every runtime decodes them.
//! [`check`] says whether a JSON value is one of a type, before anything runs with it.

use std::fmt::Write;

use anyhow::Context;
use serde_json::Value as Json;

use crate::types::{Def, Defs, Type};
use crate::value::Value;

/// The value written as JSON `text`.
pub fn decode(text: &str) -> anyhow::Result<Value> {
    let mut json = serde_json::Deserializer::from_str(text);
    transcode(&mut json).with_context(|| format!("{text:?} is not JSON"))
}

/// The value `json` is.
pub fn to_value(json: &Json) -> anyhow::Result<Value> {
    transcode(json)
}

fn transcode<'de>(json: impl serde::Deserializer<'de>) -> anyhow::Result<Value> {
    let mut bytes = Vec::new();
    let mut msgpack = rmp_serde::Serializer::new(&mut bytes).with_struct_map();
    serde_transcode::transcode(json, &mut msgpack)?;
    Ok(Value::encoded(bytes))
}

/// Whether `json` is a `ty`, as serde writes one in JSON by default: a struct as an object, a
/// newtype as what it wraps, a tuple struct as an array, an enum's unit variant as its name and
/// any other variant as `{"Variant": ..}`. A missing `Option` field is `None`. A named type whose
/// shape is unknown takes anything. The error says where in `json` it went wrong, from `path`.
pub fn check(json: &Json, ty: &Type, defs: &Defs, path: &str) -> Result<(), String> {
    let mismatch = || Err(format!("`{path}`: expected {ty}, got {}", kind(json)));
    match (ty, json) {
        (Type::Unit, Json::Null)
        | (Type::Bool, Json::Bool(_))
        | (Type::String, Json::String(_)) => Ok(()),
        (Type::Char, Json::String(s)) if s.chars().count() == 1 => Ok(()),
        (Type::Int { bits, signed }, Json::Number(n)) => {
            let n = n
                .as_i64()
                .map(i128::from)
                .or_else(|| n.as_u64().map(i128::from));
            // JSON numbers stop at 64 bits, so a 128-bit type takes any of them.
            let (min, max) = match (signed, *bits) {
                (_, 128) => (i128::MIN, i128::MAX),
                (true, bits) => (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1),
                (false, bits) => (0, (1i128 << bits) - 1),
            };
            match n {
                Some(n) if (min..=max).contains(&n) => Ok(()),
                Some(_) => Err(format!("`{path}`: {json} does not fit in {ty}")),
                None => mismatch(),
            }
        }
        (Type::Float { .. }, Json::Number(_)) => Ok(()),
        (Type::Bytes, Json::Array(items)) => {
            match items.iter().all(|b| b.as_u64().is_some_and(|b| b <= 255)) {
                true => Ok(()),
                false => Err(format!(
                    "`{path}`: expected bytes, an array of numbers 0 to 255"
                )),
            }
        }
        (Type::Option(_), Json::Null) => Ok(()),
        (Type::Option(inner), _) => check(json, inner, defs, path),
        (Type::List(item), Json::Array(items)) => items
            .iter()
            .enumerate()
            .try_for_each(|(i, json)| check(json, item, defs, &format!("{path}[{i}]"))),
        (Type::Map(key, value), Json::Object(entries)) => match **key {
            Type::String => entries
                .iter()
                .try_for_each(|(k, json)| check(json, value, defs, &format!("{path}.{k}"))),
            _ => Err(format!(
                "`{path}`: a map from JSON has string keys, not {key}"
            )),
        },
        (Type::Tuple(types), Json::Array(items)) if types.len() == items.len() => types
            .iter()
            .zip(items)
            .enumerate()
            .try_for_each(|(i, (ty, json))| check(json, ty, defs, &format!("{path}.{i}"))),
        (Type::Named(name), _) => match defs.get(name) {
            None => Ok(()),
            Some(Def::Struct(fields)) => check_fields(json, fields, defs, path, ty),
            Some(Def::Enum(variants)) => {
                let (variant, json) = match json {
                    Json::String(variant) => (variant, None),
                    Json::Object(entries) if entries.len() == 1 => {
                        let (variant, json) = entries.iter().next().expect("one entry");
                        (variant, Some(json))
                    }
                    _ => return mismatch(),
                };
                let Some((_, fields)) = variants.iter().find(|(name, _)| name == variant) else {
                    let names = variants.iter().map(|(name, _)| name.as_str());
                    return Err(format!(
                        "`{path}`: {ty} has no variant {variant:?}; it has {}",
                        names.collect::<Vec<_>>().join(", ")
                    ));
                };
                match json {
                    None if fields.is_empty() => Ok(()),
                    None => Err(format!(
                        "`{path}`: {variant} has fields: {{\"{variant}\": ..}}"
                    )),
                    Some(json) => {
                        check_fields(json, fields, defs, &format!("{path}.{variant}"), ty)
                    }
                }
            }
        },
        _ => mismatch(),
    }
}

/// Checks a struct's or a variant's `fields`, written as serde writes them.
fn check_fields(
    json: &Json,
    fields: &[(String, Type)],
    defs: &Defs,
    path: &str,
    ty: &Type,
) -> Result<(), String> {
    let positional = !fields.is_empty()
        && fields
            .iter()
            .enumerate()
            .all(|(i, (name, _))| *name == i.to_string());
    match (fields, json) {
        ([], Json::Null) => Ok(()),
        ([(_, inner)], _) if positional => check(json, inner, defs, path),
        (_, Json::Array(items)) if positional && items.len() == fields.len() => fields
            .iter()
            .zip(items)
            .try_for_each(|((name, ty), json)| check(json, ty, defs, &format!("{path}.{name}"))),
        (_, Json::Object(entries)) if !positional => {
            if let Some(unknown) = entries
                .keys()
                .find(|key| !fields.iter().any(|(name, _)| name == *key))
            {
                return Err(format!("`{path}`: {ty} has no field {unknown:?}"));
            }
            fields
                .iter()
                .try_for_each(|(name, ty)| match (entries.get(name), ty) {
                    (Some(json), _) => check(json, ty, defs, &format!("{path}.{name}")),
                    (None, Type::Option(_)) => Ok(()),
                    (None, _) => Err(format!("`{path}`: missing field {name:?}")),
                })
        }
        _ => Err(format!("`{path}`: expected {ty}, got {}", kind(json))),
    }
}

/// What kind of JSON value `json` is, for messages.
fn kind(json: &Json) -> String {
    match json {
        Json::Null => "null".into(),
        Json::Bool(b) => format!("{b}"),
        Json::Number(n) => format!("{n}"),
        Json::String(_) => "a string".into(),
        Json::Array(_) => "an array".into(),
        Json::Object(_) => "an object".into(),
    }
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Unit => f.write_str("()"),
            Type::Bool => f.write_str("bool"),
            Type::Char => f.write_str("char"),
            Type::Int { bits, signed } => write!(f, "{}{bits}", if *signed { 'i' } else { 'u' }),
            Type::Float { bits } => write!(f, "f{bits}"),
            Type::String => f.write_str("String"),
            Type::Bytes => f.write_str("bytes"),
            Type::Option(inner) => write!(f, "Option<{inner}>"),
            Type::List(item) => write!(f, "Vec<{item}>"),
            Type::Map(key, value) => write!(f, "Map<{key}, {value}>"),
            Type::Tuple(types) => {
                f.write_char('(')?;
                for (i, ty) in types.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{ty}")?;
                }
                if types.len() == 1 {
                    f.write_char(',')?;
                }
                f.write_char(')')
            }
            Type::Named(name) => f.write_str(name.rsplit("::").next().unwrap_or(name)),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const I32: Type = Type::Int {
        bits: 32,
        signed: true,
    };
    const U8: Type = Type::Int {
        bits: 8,
        signed: false,
    };

    fn defs() -> Defs {
        let named = |name: &str| Type::Named(name.into());
        let field = |name: &str, ty: Type| (name.to_string(), ty);
        Defs::from([
            (
                "Doc".into(),
                Def::Struct(vec![
                    field("title", Type::String),
                    field("tags", Type::List(Box::new(Type::String))),
                    field("lang", Type::Option(Box::new(Type::String))),
                ]),
            ),
            ("Id".into(), Def::Struct(vec![field("0", Type::String)])),
            (
                "Point".into(),
                Def::Struct(vec![field("0", I32), field("1", I32)]),
            ),
            ("Marker".into(), Def::Struct(vec![])),
            (
                "Source".into(),
                Def::Enum(vec![
                    ("Seed".into(), vec![]),
                    ("Url".into(), vec![field("0", Type::String)]),
                    (
                        "Crawl".into(),
                        vec![field("depth", U8), field("from", named("Id"))],
                    ),
                ]),
            ),
        ])
    }

    fn ok(json: Json, ty: &Type) {
        if let Err(problem) = check(&json, ty, &defs(), "x") {
            panic!("{json} is a {ty}, but: {problem}");
        }
    }

    fn refused(json: Json, ty: &Type) -> String {
        check(&json, ty, &defs(), "x").expect_err(&format!("{json} is not a {ty}"))
    }

    #[test]
    fn scalars() {
        ok(json!(-7), &I32);
        ok(json!(255), &U8);
        assert_eq!(refused(json!(256), &U8), "`x`: 256 does not fit in u8");
        assert_eq!(refused(json!(-1), &U8), "`x`: -1 does not fit in u8");
        assert_eq!(refused(json!(1.5), &I32), "`x`: expected i32, got 1.5");
        ok(
            json!(u64::MAX),
            &Type::Int {
                bits: 128,
                signed: true,
            },
        );
        ok(json!(2), &Type::Float { bits: 64 });
        ok(json!("é"), &Type::Char);
        refused(json!("ab"), &Type::Char);
        ok(json!(null), &Type::Unit);
        ok(json!([0, 255]), &Type::Bytes);
        refused(json!([256]), &Type::Bytes);
    }

    #[test]
    fn containers_say_where_they_went_wrong() {
        let list = Type::List(Box::new(I32));
        ok(json!([1, 2]), &list);
        assert_eq!(
            refused(json!([1, "2"]), &list),
            "`x[1]`: expected i32, got a string"
        );

        let option = Type::Option(Box::new(I32));
        ok(json!(null), &option);
        ok(json!(3), &option);

        let map = Type::Map(Box::new(Type::String), Box::new(I32));
        ok(json!({"a": 1}), &map);
        assert_eq!(
            refused(json!({"a": true}), &map),
            "`x.a`: expected i32, got true"
        );
        let by_int = Type::Map(Box::new(I32), Box::new(I32));
        refused(json!({"1": 1}), &by_int);

        let pair = Type::Tuple(vec![I32, Type::String]);
        ok(json!([1, "a"]), &pair);
        refused(json!([1]), &pair);
    }

    #[test]
    fn structs_as_serde_writes_them() {
        let doc = Type::Named("Doc".into());
        ok(json!({"title": "t", "tags": [], "lang": "en"}), &doc);
        // A missing `Option` field is `None`.
        ok(json!({"title": "t", "tags": []}), &doc);
        assert_eq!(
            refused(json!({"title": "t"}), &doc),
            "`x`: missing field \"tags\""
        );
        assert_eq!(
            refused(json!({"title": "t", "tags": [], "tilte": 1}), &doc),
            "`x`: Doc has no field \"tilte\""
        );
        // A newtype is what it wraps, a tuple struct an array, a unit struct null.
        ok(json!("a"), &Type::Named("Id".into()));
        ok(json!([1, 2]), &Type::Named("Point".into()));
        refused(json!({"0": 1, "1": 2}), &Type::Named("Point".into()));
        ok(json!(null), &Type::Named("Marker".into()));
        // A type whose shape is unknown takes anything.
        ok(json!({"any": "thing"}), &Type::Named("Opaque".into()));
    }

    #[test]
    fn enums_as_serde_writes_them() {
        let source = Type::Named("Source".into());
        ok(json!("Seed"), &source);
        ok(json!({"Url": "https://example.com"}), &source);
        ok(json!({"Crawl": {"depth": 2, "from": "a"}}), &source);
        assert_eq!(
            refused(json!({"Crawl": {"depth": 300, "from": "a"}}), &source),
            "`x.Crawl.depth`: 300 does not fit in u8"
        );
        assert_eq!(
            refused(json!("Ftp"), &source),
            "`x`: Source has no variant \"Ftp\"; it has Seed, Url, Crawl"
        );
        refused(json!("Url"), &source);
    }

    #[test]
    fn json_becomes_a_value_tasks_decode() {
        assert_eq!(to_value(&json!(5)).unwrap().to::<i32>().unwrap(), 5);
        let words = to_value(&json!(["a", "b"])).unwrap();
        assert_eq!(words.to::<Vec<String>>().unwrap(), ["a", "b"]);
        assert_eq!(decode("\"hi\"").unwrap().to::<String>().unwrap(), "hi");
        assert!(decode("{").is_err());
    }
}
