use namu_core::{registry::TypeEntry};
use serde_json::{Deserializer as JsonDeserializer, Serializer as JsonSerializer};
use serde::{Deserialize, Serialize};

#[namu_macros::r#type]
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MyType {
    pub x: i32,
    pub y: String,
}

#[test]
fn type_macro_inventory_and_deserialize() {
    let entry = inventory::iter::<TypeEntry>
        .into_iter()
        .find(|e| e.name == "MyType")
        .expect("TypeEntry not found");

    let json = r#"{ "x": 42, "y": "hello" }"#;
    let mut de = JsonDeserializer::from_str(json);
    let mut erased = <dyn erased_serde::Deserializer>::erase(&mut de);

    let value = (entry.deserialize)(&mut erased).expect("deserialize failed");

    let my: &MyType = value.downcast_ref().expect("downcast failed");
    assert_eq!(my.x, 42);
    assert_eq!(my.y, "hello");
}

#[test]
fn type_macro_value_serialize_to_json() {
    let my = MyType { x: 42, y: "hello".to_string() };

    let value = namu_core::Value::new(my.clone());
    let mut json = Vec::new();
    let mut se = JsonSerializer::new(&mut json);
    value.serialize(&mut se).expect("serialize failed");

    let json = String::from_utf8(json).expect("UTF-8 conversion failed");
    let expected = serde_json::json!({"x": 42, "y": "hello"}).to_string();
    assert_eq!(json, expected);
}
