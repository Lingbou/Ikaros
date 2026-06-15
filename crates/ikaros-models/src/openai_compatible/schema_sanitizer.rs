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
        let value = match key.as_str() {
            "properties" | "patternProperties" | "$defs" | "definitions" => {
                if let Value::Object(sub) = value {
                    Value::Object(
                        sub.into_iter()
                            .map(|(sub_key, sub_value)| (sub_key, repair_schema(sub_value, true)))
                            .collect(),
                    )
                } else {
                    value
                }
            }
            "anyOf" | "oneOf" | "allOf" | "prefixItems" => {
                if let Value::Array(items) = value {
                    Value::Array(
                        items
                            .into_iter()
                            .map(|item| repair_schema(item, true))
                            .collect(),
                    )
                } else {
                    value
                }
            }
            "items" | "contains" | "not" | "additionalProperties" | "propertyNames" => {
                if value.is_object() {
                    repair_schema(value, true)
                } else {
                    value
                }
            }
            _ => value,
        };
        repaired.insert(key, value);
    }

    if !is_schema {
        return Value::Object(repaired);
    }

    if let Some(Value::Array(any_of)) = repaired.remove("anyOf") {
        repaired.remove("type");
        let non_null = any_of
            .into_iter()
            .filter(|branch| branch.get("type").and_then(Value::as_str) != Some("null"))
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
                return Value::Object(repaired);
            }
        }
    }

    repaired.remove("nullable");
    if !repaired.contains_key("$ref") {
        fill_missing_type(&mut repaired);
    }
    clean_scalar_enum(&mut repaired);
    Value::Object(repaired)
}

fn fill_missing_type(map: &mut Map<String, Value>) {
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

fn clean_scalar_enum(map: &mut Map<String, Value>) {
    let Some(kind) = map.get("type").and_then(Value::as_str) else {
        return;
    };
    if !matches!(kind, "string" | "integer" | "number" | "boolean") {
        return;
    }
    let Some(Value::Array(values)) = map.get_mut("enum") else {
        return;
    };
    values.retain(|value| !value.is_null() && value.as_str() != Some(""));
    if values.is_empty() {
        map.remove("enum");
    }
}
