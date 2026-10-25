//! Formatting for client-owned disk documents; external JSON contracts stay separate.
use serde::{Serialize, Serializer, ser::SerializeMap};
use serde_json::Value;

pub(crate) fn pretty(value: &impl Serialize) -> Result<Vec<u8>, serde_json::Error> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Order dynamic document headers without renaming or interpreting external fields.
pub(crate) struct Ordered<'a>(pub &'a Value);

impl Serialize for Ordered<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Value::Object(object) => {
                let mut map = serializer.serialize_map(Some(object.len()))?;
                for key in ["schema", "type", "id"] {
                    if let Some(value) = object.get(key) {
                        map.serialize_entry(key, &Ordered(value))?;
                    }
                }
                for (key, value) in object {
                    if !matches!(key.as_str(), "schema" | "type" | "id") {
                        map.serialize_entry(key, &Ordered(value))?;
                    }
                }
                map.end()
            }
            Value::Array(values) => serializer.collect_seq(values.iter().map(Ordered)),
            value => value.serialize(serializer),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_headers_are_first_without_changing_external_field_names() {
        let value = serde_json::json!({"active":true,"type":"repair","schema":3,"id":"example",
            "external":{"schema_version":7,"kind":"external"}});
        let bytes = pretty(&Ordered(&value)).unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.starts_with("{\n  \"schema\": 3,\n  \"type\": \"repair\",\n  \"id\""));
        assert!(text.ends_with('\n'));
        assert_eq!(serde_json::from_slice::<Value>(&bytes).ok(), Some(value));
    }
}
