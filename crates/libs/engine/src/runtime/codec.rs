use namu_core::Value;

pub trait ValueRuntime: Send + Sync + Clone + 'static {
    type Value: Clone + Send + Sync + 'static;

    fn parse_literal(&self, raw: &str) -> anyhow::Result<Self::Value>;
    fn as_bool(&self, value: &Self::Value) -> anyhow::Result<bool>;
}

#[derive(Clone, Default)]
pub struct CoreValueRuntime;

impl ValueRuntime for CoreValueRuntime {
    type Value = Value;

    fn parse_literal(&self, raw: &str) -> anyhow::Result<Self::Value> {
        // Literals are JSON (see `namu_flow::Builder::literal`); "()" is the unit placeholder
        // for empty blocks. Integers stay `i32` as before; strings are fully unescaped.
        if raw == "()" {
            return Ok(Value::new(()));
        }
        let val = match serde_json::from_str::<serde_json::Value>(raw) {
            Ok(serde_json::Value::Null) => Value::new(()),
            Ok(serde_json::Value::Bool(b)) => Value::new(b),
            Ok(serde_json::Value::String(s)) => Value::new(s),
            Ok(serde_json::Value::Number(n)) => match n.as_i64().map(i32::try_from) {
                Some(Ok(i)) => Value::new(i),
                Some(Err(_)) => Value::new(n.as_i64().unwrap_or_default()),
                None => Value::new(n.as_f64().unwrap_or_default()),
            },
            _ => Value::new(raw.to_string()),
        };
        Ok(val)
    }

    fn as_bool(&self, value: &Self::Value) -> anyhow::Result<bool> {
        value
            .downcast_ref::<bool>()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("branch value not bool"))
    }
}

#[derive(Clone, Default)]
pub struct JsonRuntime;

impl ValueRuntime for JsonRuntime {
    type Value = serde_json::Value;

    fn parse_literal(&self, raw: &str) -> anyhow::Result<Self::Value> {
        // Literals are JSON; "()" is the unit placeholder for empty blocks.
        if raw == "()" {
            return Ok(serde_json::Value::Null);
        }
        Ok(serde_json::from_str(raw)
            .unwrap_or_else(|_| serde_json::Value::String(raw.trim_matches('"').to_string())))
    }

    fn as_bool(&self, value: &Self::Value) -> anyhow::Result<bool> {
        value
            .as_bool()
            .ok_or_else(|| anyhow::anyhow!("branch value not bool"))
    }
}
