//! Document parameters: override merging, `$param` substitution and the relaxed schema.

use motion_core::ValidationError;
use serde_json::{Map, Value};

/// Key that marks a parameter reference object, `{"$param": "name"}`.
pub(crate) const PARAM_KEY: &str = "$param";

/// Largest accepted serialized size of one parameter value.
const MAX_PARAMETER_BYTES: usize = 64 * 1024;

/// Most parameters one document may declare.
const MAX_PARAMETERS: usize = 256;

/// Merges overrides into the declared defaults and substitutes every reference.
///
/// The root `params` object declares the defaults. Overrides replace whole
/// values by key and unknown keys are rejected. Each `{"$param": "name"}` object
/// anywhere outside `params` is replaced by that parameter's value, so a
/// parameter can supply numbers, colors, strings, points or whole subtrees.
/// The merged values are written back to `params`, where expressions read them.
pub(crate) fn resolve(
    document: &mut Value,
    overrides: Option<&Value>,
    budget: usize,
) -> Result<(), ValidationError> {
    let Some(root) = document.as_object_mut() else {
        return Ok(());
    };
    let mut parameters = match root.remove("params") {
        None => Map::new(),
        Some(Value::Object(parameters)) => parameters,
        Some(_) => {
            return Err(invalid("params", "Parameters must be a JSON object"));
        }
    };
    if parameters.len() > MAX_PARAMETERS {
        return Err(limit("params", "Documents declare at most 256 parameters"));
    }
    for (name, value) in &parameters {
        if serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len()) > MAX_PARAMETER_BYTES {
            return Err(limit(
                format!("params.{name}"),
                "A parameter value is limited to 64 KiB",
            ));
        }
    }
    if let Some(overrides) = overrides {
        let overrides = overrides
            .as_object()
            .ok_or_else(|| invalid("params", "Parameter overrides must be a JSON object"))?;
        for (name, value) in overrides {
            if !parameters.contains_key(name) {
                return Err(invalid(
                    format!("params.{name}"),
                    unknown_parameter(name, &parameters),
                ));
            }
            parameters.insert(name.clone(), value.clone());
        }
    }
    let mut remaining = budget;
    for (key, value) in root.iter_mut() {
        substitute(value, &parameters, key, &mut remaining)?;
    }
    if !parameters.is_empty() {
        root.insert("params".into(), Value::Object(parameters));
    }
    Ok(())
}

/// Rewrites a JSON schema so every value position also accepts a `$param` reference.
///
/// The document schema describes resolved documents; this relaxes it for editors
/// that validate unresolved files. Parameter objects are structural: the resolved
/// value is still checked by compilation.
pub(crate) fn relax_schema(schema: &mut Value) {
    let reference = serde_json::json!({
        "description": "A parameter reference, replaced by the named parameter value on load.",
        "type": "object",
        "properties": {PARAM_KEY: {"type": "string"}},
        "required": [PARAM_KEY],
        "additionalProperties": false
    });
    relax(schema);
    if let Some(definitions) = schema.get_mut("$defs").and_then(Value::as_object_mut) {
        definitions.insert("ParamRef".into(), reference);
    }
}

fn relax(schema: &mut Value) {
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    for key in ["properties", "$defs"] {
        if let Some(Value::Object(children)) = object.get_mut(key) {
            for child in children.values_mut() {
                relax(child);
                wrap(child);
            }
        }
    }
    for key in ["items", "additionalProperties"] {
        if let Some(child) = object.get_mut(key)
            && child.is_object()
        {
            relax(child);
            wrap(child);
        }
    }
    for key in ["anyOf", "oneOf", "allOf", "prefixItems"] {
        if let Some(Value::Array(children)) = object.get_mut(key) {
            for child in children {
                relax(child);
            }
        }
    }
}

fn wrap(schema: &mut Value) {
    let is_reference = schema.get("$ref").and_then(Value::as_str) == Some("#/$defs/ParamRef");
    if is_reference || schema.is_boolean() {
        return;
    }
    let inner = std::mem::take(schema);
    *schema = serde_json::json!({"anyOf": [inner, {"$ref": "#/$defs/ParamRef"}]});
}

fn substitute(
    value: &mut Value,
    parameters: &Map<String, Value>,
    path: &str,
    remaining: &mut usize,
) -> Result<(), ValidationError> {
    match value {
        Value::Object(object) => {
            if let Some(name) = object.get(PARAM_KEY).filter(|_| object.len() == 1) {
                let name = name
                    .as_str()
                    .ok_or_else(|| invalid(path, "`$param` requires a parameter name string"))?;
                let replacement = parameters
                    .get(name)
                    .ok_or_else(|| invalid(path, unknown_parameter(name, parameters)))?;
                let size = serde_json::to_vec(replacement).map_or(usize::MAX, |bytes| bytes.len());
                *remaining = remaining.checked_sub(size).ok_or_else(|| {
                    limit(
                        path,
                        "Parameter substitution exceeds the document size limit",
                    )
                })?;
                *value = replacement.clone();
                return Ok(());
            }
            for (key, child) in object.iter_mut() {
                substitute(child, parameters, &format!("{path}.{key}"), remaining)?;
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter_mut().enumerate() {
                substitute(child, parameters, &format!("{path}[{index}]"), remaining)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn unknown_parameter(name: &str, parameters: &Map<String, Value>) -> String {
    let mut message = format!("Unknown parameter: {name}");
    if let Some(hint) =
        crate::diagnostics::did_you_mean(name, parameters.keys().map(String::as_str))
    {
        message.push_str(". ");
        message.push_str(&hint);
    }
    message
}

fn invalid(path: impl Into<String>, message: impl Into<String>) -> ValidationError {
    ValidationError::invalid(path, "params", message)
}

fn limit(path: impl Into<String>, message: impl Into<String>) -> ValidationError {
    ValidationError::limit(path, "params", message)
}
