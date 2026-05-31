//! Tool-schema sanitization for MCP `tools/list` output.
//!
//! `schemars` renders arbitrary-JSON fields (`serde_json::Value`,
//! `Option<serde_json::Value>`) as the JSON Schema boolean `true`. That is a
//! technically valid schema (draft-06+: a boolean schema, where `true` means
//! "any value", equivalent to `{}`), but strict MCP clients — including the
//! Zod-based validator in the official clients — reject a property whose schema
//! is a bare boolean and fail the entire `tools/list` response with
//! `Invalid input`. The symptom is "Reconnected to <server>, but fetching tools
//! failed: tools[..].(input|output)Schema.properties.<field> Invalid input",
//! after which none of the server's tools are usable.
//!
//! See the upstream discussion of this exact incompatibility:
//! <https://github.com/PrefectHQ/fastmcp/issues/3783> (boolean property schemas)
//! and schemars' documented behaviour for `serde_json::Value`.
//!
//! Rather than annotate every `Value` field individually (fragile — the next
//! `Value` field reintroduces the bug), we normalize at the serving boundary:
//! every emitted tool schema is walked and any boolean found in a *schema
//! position that strict clients validate as a property schema* is replaced with
//! an explicit, fully permissive object schema. This is exhaustive over current
//! and future tools and is enforced by `schema_sanitize_tests`.
//!
//! Booleans in `additionalProperties` / `additionalItems` / `unevaluated*`
//! positions are intentionally preserved: a boolean there is meaningful and is
//! accepted by clients.

use std::sync::Arc;

use rmcp::model::Tool;
use serde_json::{Map, Value};

/// Keywords whose value is a *map* of subschemas. A boolean value of any member
/// is the client-rejected case and is rewritten.
const SCHEMA_MAP_KEYWORDS: &[&str] = &["properties", "patternProperties"];

/// Keywords whose value is an *array* of subschemas. A boolean element is
/// rewritten.
const SCHEMA_ARRAY_KEYWORDS: &[&str] = &["oneOf", "anyOf", "allOf", "prefixItems"];

/// Sanitizes every tool's input and output schema so no property/composition
/// subschema is a bare boolean. Returns tools safe to send over `tools/list`.
#[must_use]
pub fn sanitize_tools(tools: Vec<Tool>) -> Vec<Tool> {
    tools.into_iter().map(sanitize_tool).collect()
}

fn sanitize_tool(mut tool: Tool) -> Tool {
    tool.input_schema = sanitize_schema_object(&tool.input_schema);
    if let Some(output) = &tool.output_schema {
        tool.output_schema = Some(sanitize_schema_object(output));
    }
    tool
}

fn sanitize_schema_object(schema: &Arc<Map<String, Value>>) -> Arc<Map<String, Value>> {
    let mut cloned = (**schema).clone();
    rewrite_map(&mut cloned);
    Arc::new(cloned)
}

/// A fully permissive but explicit object schema used to replace a bare boolean
/// `true`. Every JSON value validates against it, and every strict client
/// accepts it because it is an object with a concrete `type` union.
fn permissive_schema() -> Value {
    Value::Object(Map::from_iter([(
        "type".to_owned(),
        Value::Array(vec![
            Value::String("object".to_owned()),
            Value::String("array".to_owned()),
            Value::String("string".to_owned()),
            Value::String("number".to_owned()),
            Value::String("boolean".to_owned()),
            Value::String("null".to_owned()),
        ]),
    )]))
}

/// A never-matching object schema used to replace a bare boolean `false`.
fn never_schema() -> Value {
    Value::Object(Map::from_iter([(
        "not".to_owned(),
        Value::Object(Map::new()),
    )]))
}

fn boolean_as_schema(b: bool) -> Value {
    if b {
        permissive_schema()
    } else {
        never_schema()
    }
}

fn rewrite_value(value: &mut Value) {
    match value {
        Value::Object(map) => rewrite_map(map),
        Value::Array(items) => {
            for item in items.iter_mut() {
                rewrite_value(item);
            }
        }
        _ => {}
    }
}

fn rewrite_map(map: &mut Map<String, Value>) {
    for (key, child) in map.iter_mut() {
        if SCHEMA_MAP_KEYWORDS.contains(&key.as_str()) {
            if let Value::Object(members) = child {
                for member in members.values_mut() {
                    if let Value::Bool(b) = member {
                        *member = boolean_as_schema(*b);
                    } else {
                        rewrite_value(member);
                    }
                }
            } else {
                rewrite_value(child);
            }
        } else if SCHEMA_ARRAY_KEYWORDS.contains(&key.as_str()) {
            if let Value::Array(elements) = child {
                for element in elements.iter_mut() {
                    if let Value::Bool(b) = element {
                        *element = boolean_as_schema(*b);
                    } else {
                        rewrite_value(element);
                    }
                }
            } else {
                rewrite_value(child);
            }
        } else {
            // `$defs`, `definitions`, `items`, `not`, `additionalProperties`
            // (object form), etc. Recurse to reach nested `properties`, but do
            // not rewrite a boolean that legitimately lives in
            // `additionalProperties`/`additionalItems`/`unevaluated*`.
            rewrite_value(child);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// Returns every JSON-pointer-ish path at which a *boolean* appears in a
    /// client-validated schema position (a `properties`/`patternProperties`
    /// member, or a `oneOf`/`anyOf`/`allOf`/`prefixItems` element). These are
    /// exactly the positions strict MCP clients reject.
    fn bare_boolean_schema_paths(value: &Value, path: &str, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    let child_path = format!("{path}.{key}");
                    if SCHEMA_MAP_KEYWORDS.contains(&key.as_str()) {
                        if let Value::Object(members) = child {
                            for (mk, mv) in members {
                                if mv.is_boolean() {
                                    out.push(format!("{child_path}.{mk}"));
                                } else {
                                    bare_boolean_schema_paths(
                                        mv,
                                        &format!("{child_path}.{mk}"),
                                        out,
                                    );
                                }
                            }
                            continue;
                        }
                    } else if SCHEMA_ARRAY_KEYWORDS.contains(&key.as_str())
                        && let Value::Array(elements) = child
                    {
                        for (i, ev) in elements.iter().enumerate() {
                            if ev.is_boolean() {
                                out.push(format!("{child_path}[{i}]"));
                            } else {
                                bare_boolean_schema_paths(ev, &format!("{child_path}[{i}]"), out);
                            }
                        }
                        continue;
                    }
                    bare_boolean_schema_paths(child, &child_path, out);
                }
            }
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    bare_boolean_schema_paths(item, &format!("{path}[{i}]"), out);
                }
            }
            _ => {}
        }
    }

    /// Full real tool surface, sanitized, must contain ZERO bare-boolean schema
    /// positions. This is the regression gate that keeps any current or future
    /// `serde_json::Value` tool field from breaking strict MCP clients.
    #[test]
    fn real_tool_schemas_have_no_bare_boolean_property_schemas_after_sanitize() {
        let tools = sanitize_tools(super::super::SynapseService::tool_router().list_all());
        let mut offenders = Vec::new();
        for tool in &tools {
            let input = Value::Object((*tool.input_schema).clone());
            bare_boolean_schema_paths(
                &input,
                &format!("{}.inputSchema", tool.name),
                &mut offenders,
            );
            if let Some(output) = &tool.output_schema {
                let output = Value::Object((**output).clone());
                bare_boolean_schema_paths(
                    &output,
                    &format!("{}.outputSchema", tool.name),
                    &mut offenders,
                );
            }
        }
        assert!(
            offenders.is_empty(),
            "sanitized tool schemas still contain bare boolean schemas (strict MCP clients reject these): {offenders:#?}"
        );
    }

    /// The raw (un-sanitized) surface is expected to contain bare booleans
    /// (schemars emits them for `serde_json::Value`). If this ever becomes
    /// empty the sanitizer is no longer load-bearing, which is fine — but we
    /// assert it here so the gate above can never pass vacuously.
    #[test]
    fn raw_tool_schemas_do_contain_bare_booleans() {
        let tools = super::super::SynapseService::tool_router().list_all();
        let mut offenders = Vec::new();
        for tool in &tools {
            let input = Value::Object((*tool.input_schema).clone());
            bare_boolean_schema_paths(&input, "in", &mut offenders);
            if let Some(output) = &tool.output_schema {
                let output = Value::Object((**output).clone());
                bare_boolean_schema_paths(&output, "out", &mut offenders);
            }
        }
        assert!(
            !offenders.is_empty(),
            "expected schemars to emit at least one bare boolean schema for a serde_json::Value field"
        );
    }

    #[test]
    fn rewrite_converts_property_booleans_and_preserves_additional_properties() {
        let mut schema = json!({
            "type": "object",
            "properties": {
                "payload": true,
                "nested": {
                    "type": "object",
                    "properties": { "inner": true }
                }
            },
            "oneOf": [ true, { "type": "string" } ],
            "additionalProperties": false
        });
        rewrite_value(&mut schema);

        // properties.payload boolean -> permissive object schema
        assert!(schema["properties"]["payload"].is_object());
        assert!(schema["properties"]["payload"]["type"].is_array());
        // deeply nested properties boolean rewritten too
        assert!(schema["properties"]["nested"]["properties"]["inner"].is_object());
        // oneOf boolean element rewritten
        assert!(schema["oneOf"][0].is_object());
        // additionalProperties boolean preserved (meaningful, accepted by clients)
        assert_eq!(schema["additionalProperties"], Value::Bool(false));
    }
}
