//! Simlink's source manifest, which every test reads its schemas from.

use std::sync::LazyLock;

use serde_json::{Map, Value, json};

pub static MANIFEST: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../../protocol/source-schema.json")).unwrap()
});

/// Everything `value` violates in `schema`.
pub fn issues(schema: &Value, value: &Value) -> Vec<String> {
    jsonschema::validator_for(schema)
        .unwrap()
        .iter_errors(value)
        .map(|e| e.to_string())
        .collect()
}

/// The most alternatives any `enum`, `oneOf`, `anyOf` or type list in `schema` offers, so samples for that many
/// branches reach every one.
pub fn branches(schema: &Value) -> usize {
    match schema {
        Value::Object(object) => {
            let own = ["enum", "oneOf", "anyOf", "type"]
                .iter()
                .filter_map(|key| object.get(*key)?.as_array())
                .map(Vec::len);
            own.chain(object.values().map(branches)).max().unwrap_or(1)
        }
        Value::Array(items) => items.iter().map(branches).max().unwrap_or(1),
        _ => 1,
    }
}

/// Builds a value that satisfies `schema`: only required properties, or all of them when `full`, taking the
/// `branch`th alternative of every `oneOf`/`anyOf`/type list, wrapping around.
pub fn sample(schema: &Value, root: &Value, full: bool, branch: usize) -> Value {
    let pick = |options: &Vec<Value>| options[branch % options.len()].clone();
    if let Some(target) = schema["$ref"].as_str().and_then(|r| r.strip_prefix('#')) {
        return sample(root.pointer(target).unwrap(), root, full, branch);
    }
    if let Some(value) = schema.get("const") {
        return value.clone();
    }
    if let Some(values) = schema["enum"].as_array() {
        return pick(values);
    }
    if let Some(options) = schema["oneOf"]
        .as_array()
        .or_else(|| schema["anyOf"].as_array())
    {
        return sample(&pick(options), root, full, branch);
    }

    let kind = match &schema["type"] {
        Value::Array(kinds) => pick(kinds),
        kind => kind.clone(),
    };
    match kind.as_str() {
        Some("object") => {
            let properties = schema["properties"]
                .as_object()
                .cloned()
                .unwrap_or_default();
            let required: Vec<&str> = schema["required"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let mut object: Map<String, Value> = properties
                .iter()
                // The SDK omits const-with-default properties, which Simlink fills in.
                .filter(|(name, property)| {
                    (full || required.contains(&name.as_str()))
                        && !(property.get("const").is_some() && property.get("default").is_some())
                })
                .map(|(name, property)| (name.clone(), sample(property, root, full, branch)))
                .collect();
            if let Some(values) = schema
                .get("additionalProperties")
                .filter(|values| values.is_object())
            {
                object.insert("key".into(), sample(values, root, full, branch));
            }
            Value::Object(object)
        }
        Some("array") => json!([sample(&schema["items"], root, full, branch)]),
        Some("string") => {
            let len = schema["minLength"].as_u64().unwrap_or(1).max(1);
            let len = len.min(schema["maxLength"].as_u64().unwrap_or(u64::MAX)) as usize;
            // The first candidate the schema's length, pattern and format accept.
            ["A", "a", "0"]
                .map(|c| json!(c.repeat(len)))
                .into_iter()
                .chain([json!("2026-01-01T00:00:00Z")])
                .find(|candidate| issues(schema, candidate).is_empty())
                .unwrap_or_else(|| panic!("teach sample() to satisfy {schema}"))
        }
        Some("integer") => schema.get("minimum").cloned().unwrap_or(json!(1)),
        Some("number") => json!(1.5),
        Some("boolean") => json!(full),
        _ => Value::Null,
    }
}
