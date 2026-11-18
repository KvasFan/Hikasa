//! A small JSON Schema (draft 2020-12) validator for the subset `docs/schemas/table_compare.v1.json` uses: `type` (a
//! name or a list of names), `properties`, `required`, `additionalProperties: false`, `items`, `enum`, `const`,
//! `minimum`, `anyOf` and `$ref` to `#/$defs/<name>`. Keywords it does not know are an error, so the schema cannot rely
//! on a check that is silently skipped. The workspace has no JSON Schema crate and the lock must not grow.

use serde_json::{Map, Value};

/// The keywords this validator knows (annotations included).
const KNOWN: &[&str] = &[
    "$schema",
    "$id",
    "$defs",
    "$ref",
    "$comment",
    "title",
    "description",
    "type",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "enum",
    "const",
    "minimum",
    "pattern",
    "maximum",
    "minItems",
    "maxItems",
    "anyOf",
];

/// Validates `value` against `schema` (the root, which holds `$defs`): every violation as `<path>: <what>`.
pub fn validate(schema: &Value, value: &Value) -> Vec<String> {
    let mut errors = Vec::new();
    check(schema, schema, value, "$", &mut errors);
    errors
}

fn type_ok(name: &str, v: &Value) -> bool {
    match name {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "integer" => v.is_u64() || v.is_i64(),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        _ => false,
    }
}

/// The patterns the schemas use (no regex crate here): lower-case hex bytes, decimal integers; `None` for another.
fn pattern_matches(pattern: &str, text: &str) -> Option<bool> {
    let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    Some(match pattern {
        "^([0-9a-f]{2})*$" => {
            text.len().is_multiple_of(2)
                && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        }
        "^[0-9]+$" => digits(text),
        "^-?[0-9]+$" => digits(text.strip_prefix('-').unwrap_or(text)),
        _ => return None,
    })
}

fn resolve<'a>(root: &'a Value, reference: &str) -> Option<&'a Value> {
    let name = reference.strip_prefix("#/$defs/")?;
    root.get("$defs")?.get(name)
}

fn check(root: &Value, schema: &Value, v: &Value, path: &str, errors: &mut Vec<String>) {
    let Some(s) = schema.as_object() else {
        if schema != &Value::Bool(true) {
            errors.push(format!("{path}: the schema is not an object"));
        }
        return;
    };
    for key in s.keys() {
        if !KNOWN.contains(&key.as_str()) {
            errors.push(format!("{path}: unknown keyword {key}"));
        }
    }
    if let Some(r) = s.get("$ref").and_then(Value::as_str) {
        match resolve(root, r) {
            Some(target) => check(root, target, v, path, errors),
            None => errors.push(format!("{path}: unresolved $ref {r}")),
        }
    }
    if let Some(any) = s.get("anyOf").and_then(Value::as_array) {
        let ok = any.iter().any(|alt| {
            let mut sub = Vec::new();
            check(root, alt, v, path, &mut sub);
            sub.is_empty()
        });
        if !ok {
            errors.push(format!("{path}: matches no alternative of anyOf"));
        }
    }
    if let Some(t) = s.get("type") {
        let names: Vec<&str> = match t {
            Value::String(n) => vec![n.as_str()],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        if !names.iter().any(|n| type_ok(n, v)) {
            errors.push(format!("{path}: expected {names:?}, got {v}"));
            return;
        }
    }
    if let Some(values) = s.get("enum").and_then(Value::as_array)
        && !values.contains(v)
    {
        errors.push(format!("{path}: {v} is not one of {values:?}"));
    }
    if let Some(c) = s.get("const")
        && c != v
    {
        errors.push(format!("{path}: {v} is not {c}"));
    }
    if let (Some(min), Some(n)) = (s.get("minimum").and_then(Value::as_f64), v.as_f64())
        && n < min
    {
        errors.push(format!("{path}: {n} is below {min}"));
    }
    if let (Some(max), Some(n)) = (s.get("maximum").and_then(Value::as_f64), v.as_f64())
        && n > max
    {
        errors.push(format!("{path}: {n} is above {max}"));
    }
    if let (Some(pattern), Some(text)) = (s.get("pattern").and_then(Value::as_str), v.as_str()) {
        match pattern_matches(pattern, text) {
            Some(true) => {}
            Some(false) => errors.push(format!("{path}: {text:?} does not match {pattern}")),
            None => errors.push(format!("{path}: unsupported pattern {pattern}")),
        }
    }
    if let Some(obj) = v.as_object() {
        object(root, s, obj, path, errors);
    }
    if let Some(arr) = v.as_array() {
        let len = arr.len() as u64;
        if s.get("minItems")
            .and_then(Value::as_u64)
            .is_some_and(|m| len < m)
        {
            errors.push(format!("{path}: fewer than minItems"));
        }
        if s.get("maxItems")
            .and_then(Value::as_u64)
            .is_some_and(|m| len > m)
        {
            errors.push(format!("{path}: more than maxItems"));
        }
    }
    if let (Some(items), Some(arr)) = (s.get("items"), v.as_array()) {
        for (i, item) in arr.iter().enumerate() {
            check(root, items, item, &format!("{path}[{i}]"), errors);
        }
    }
}

fn object(
    root: &Value,
    s: &Map<String, Value>,
    obj: &Map<String, Value>,
    path: &str,
    errors: &mut Vec<String>,
) {
    let props = s.get("properties").and_then(Value::as_object);
    if let Some(required) = s.get("required").and_then(Value::as_array) {
        for key in required.iter().filter_map(Value::as_str) {
            if !obj.contains_key(key) {
                errors.push(format!("{path}: missing {key}"));
            }
        }
    }
    for (key, value) in obj {
        match props.and_then(|p| p.get(key)) {
            Some(sub) => check(root, sub, value, &format!("{path}.{key}"), errors),
            None => {
                if s.get("additionalProperties") == Some(&Value::Bool(false)) {
                    errors.push(format!("{path}: unexpected key {key}"));
                }
            }
        }
    }
}

/// Checks that a schema is closed: every object schema with `properties` has `additionalProperties: false` and
/// requires every property but the root's `optional` ones.
pub fn closed(schema: &Value, optional: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    walk(schema, "$", optional, &mut out);
    out
}

fn walk(s: &Value, path: &str, optional: &[&str], out: &mut Vec<String>) {
    match s {
        Value::Object(o) => {
            if let Some(props) = o.get("properties").and_then(Value::as_object) {
                if o.get("additionalProperties") != Some(&Value::Bool(false)) {
                    out.push(format!("{path}: additionalProperties is not false"));
                }
                let required: Vec<&str> = o
                    .get("required")
                    .and_then(Value::as_array)
                    .map(|r| r.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                for key in props.keys() {
                    let excused = path == "$" && optional.contains(&key.as_str());
                    if !required.contains(&key.as_str()) && !excused {
                        out.push(format!("{path}: {key} is not required"));
                    }
                }
            }
            for (k, v) in o {
                walk(v, &format!("{path}.{k}"), optional, out);
            }
        }
        Value::Array(a) => {
            for (i, v) in a.iter().enumerate() {
                walk(v, &format!("{path}[{i}]"), optional, out);
            }
        }
        _ => {}
    }
}
