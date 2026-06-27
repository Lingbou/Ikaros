// SPDX-License-Identifier: GPL-3.0-only

use crate::types::ModelToolDefinition;
use serde_json::{Map, Value};

pub(super) fn sanitize_moonshot_tool_definitions(
    tools: Vec<ModelToolDefinition>,
) -> Vec<ModelToolDefinition> {
    tools
        .into_iter()
        .map(|mut tool| {
            tool.input_schema = sanitize_moonshot_tool_parameters(tool.input_schema);
            tool
        })
        .collect()
}

pub(crate) fn sanitize_moonshot_tool_parameters(parameters: Value) -> Value {
    let mut repaired = repair_schema(parameters, true);
    let Some(map) = repaired.as_object_mut() else {
        return serde_json::json!({"type": "object", "properties": {}});
    };
    if map.get("type").and_then(Value::as_str) != Some("object") {
        map.insert("type".into(), Value::String("object".into()));
    }
    map.entry("properties")
        .or_insert_with(|| Value::Object(Map::new()));
    repaired
}

fn repair_schema(node: Value, is_schema: bool) -> Value {
    match node {
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| repair_schema(item, true))
                .collect(),
        ),
        Value::Object(map) => repair_schema_object(map, is_schema),
        other => other,
    }
}

fn repair_schema_object(map: Map<String, Value>, is_schema: bool) -> Value {
    let mut repaired = Map::new();
    for (key, value) in map {
        match key.as_str() {
            "properties" => {
                if let Value::Object(sub) = value {
                    repaired.insert(
                        key,
                        Value::Object(
                            sub.into_iter()
                                .map(|(sub_key, sub_value)| {
                                    (
                                        sub_key,
                                        ensure_schema_object(repair_schema(sub_value, true)),
                                    )
                                })
                                .collect(),
                        ),
                    );
                }
            }
            "anyOf" | "oneOf" => {
                if let Value::Array(items) = value {
                    let repaired_items = items
                        .into_iter()
                        .map(|item| ensure_schema_object(repair_schema(item, true)))
                        .collect::<Vec<_>>();
                    repaired
                        .entry("anyOf")
                        .and_modify(|existing| {
                            if let Value::Array(existing_items) = existing {
                                existing_items.extend(repaired_items.clone());
                            }
                        })
                        .or_insert_with(|| Value::Array(repaired_items));
                }
            }
            "items" => {
                if value.is_object() {
                    repaired.insert(key, repair_schema(value, true));
                }
            }
            "additionalProperties" => {
                let value = if value.is_object() {
                    repair_schema(value, true)
                } else {
                    value
                };
                if matches!(value, Value::Object(_) | Value::Bool(_)) {
                    repaired.insert(key, value);
                }
            }
            "type" | "description" | "default" | "enum" | "required" => {
                repaired.insert(key, value);
            }
            _ => {}
        }
    }

    if !is_schema {
        return Value::Object(repaired);
    }

    if let Some(Value::Array(any_of)) = repaired.remove("anyOf") {
        repaired.remove("type");
        let non_null = any_of
            .into_iter()
            .filter(|branch| branch.get("type").and_then(Value::as_str) != Some("null"))
            .map(ensure_schema_object)
            .collect::<Vec<_>>();
        match non_null.as_slice() {
            [single] => {
                if let Some(single) = single.as_object() {
                    repaired.extend(single.clone());
                }
            }
            [] => {}
            _ => {
                repaired.insert("anyOf".into(), Value::Array(non_null));
            }
        }
    }

    repaired.remove("nullable");
    fill_missing_type(&mut repaired);
    clean_required(&mut repaired);
    clean_scalar_enum(&mut repaired);
    retain_supported_schema_keys(&mut repaired);
    Value::Object(repaired)
}

fn ensure_schema_object(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(map),
        Value::Bool(_) | Value::Null => serde_json::json!({"type": "object", "properties": {}}),
        Value::Array(_) => serde_json::json!({"type": "array"}),
        Value::Number(number) if number.is_i64() || number.is_u64() => {
            serde_json::json!({"type": "integer"})
        }
        Value::Number(_) => serde_json::json!({"type": "number"}),
        Value::String(_) => serde_json::json!({"type": "string"}),
    }
}

fn retain_supported_schema_keys(map: &mut Map<String, Value>) {
    const SUPPORTED: &[&str] = &[
        "type",
        "description",
        "default",
        "anyOf",
        "properties",
        "additionalProperties",
        "items",
        "enum",
        "required",
    ];
    map.retain(|key, _| SUPPORTED.contains(&key.as_str()));
}

fn fill_missing_type(map: &mut Map<String, Value>) {
    if map.contains_key("anyOf") {
        return;
    }
    match map.get("type") {
        Some(Value::String(value)) if !value.trim().is_empty() => return,
        Some(Value::Array(values)) => {
            if let Some(Value::String(concrete)) = values.iter().find(|value| {
                value
                    .as_str()
                    .is_some_and(|kind| kind != "null" && !kind.is_empty())
            }) {
                map.insert("type".into(), Value::String(concrete.clone()));
                return;
            }
        }
        _ => {}
    }
    let inferred = if map.contains_key("properties")
        || map.contains_key("required")
        || map.contains_key("additionalProperties")
    {
        "object"
    } else if map.contains_key("items") || map.contains_key("prefixItems") {
        "array"
    } else if let Some(Value::Array(values)) = map.get("enum") {
        match values.first() {
            Some(Value::Bool(_)) => "boolean",
            Some(Value::Number(number)) if number.is_i64() || number.is_u64() => "integer",
            Some(Value::Number(_)) => "number",
            _ => "string",
        }
    } else {
        "string"
    };
    map.insert("type".into(), Value::String(inferred.into()));
}

fn clean_required(map: &mut Map<String, Value>) {
    let Some(Value::Array(values)) = map.get_mut("required") else {
        return;
    };
    values.retain(|value| value.as_str().is_some_and(|item| !item.trim().is_empty()));
    if values.is_empty() {
        map.remove("required");
    }
}

fn clean_scalar_enum(map: &mut Map<String, Value>) {
    let Some(kind) = map.get("type").and_then(Value::as_str).map(str::to_owned) else {
        return;
    };
    if !matches!(kind.as_str(), "string" | "integer" | "number" | "boolean") {
        map.remove("enum");
        return;
    }
    let Some(Value::Array(values)) = map.get_mut("enum") else {
        return;
    };
    values.retain(|value| match kind.as_str() {
        "string" => value.as_str().is_some_and(|text| !text.is_empty()),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.as_f64().is_some(),
        "boolean" => value.as_bool().is_some(),
        _ => false,
    });
    if values.is_empty() {
        map.remove("enum");
    }
}
