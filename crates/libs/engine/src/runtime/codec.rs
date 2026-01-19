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
        let val = match raw {
            "true" => Value::new(true),
            "false" => Value::new(false),
            "()" => Value::new(()),
            _ => {
                if let Ok(n) = raw.parse::<i32>() {
                    Value::new(n)
                } else {
                    Value::new(raw.trim_matches('"').to_string())
                }
            }
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
        let val = match raw {
            "true" => serde_json::Value::Bool(true),
            "false" => serde_json::Value::Bool(false),
            "()" => serde_json::Value::Null,
            _ => {
                if let Ok(n) = raw.parse::<i32>() {
                    serde_json::Value::Number(n.into())
                } else {
                    serde_json::Value::String(raw.trim_matches('"').to_string())
                }
            }
        };
        Ok(val)
    }

    fn as_bool(&self, value: &Self::Value) -> anyhow::Result<bool> {
        value
            .as_bool()
            .ok_or_else(|| anyhow::anyhow!("branch value not bool"))
    }
}
