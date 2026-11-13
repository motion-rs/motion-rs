//! Document parameters: overrides, load-time calculations, substitution and reference schemas.

use motion_core::ValidationError;
use serde_json::{Map, Value};

/// Key that marks a parameter reference object, `{"$param": "name"}`.
pub(crate) const PARAM_KEY: &str = "$param";

/// Key that marks a finite load-time numeric calculation.
const CALC_KEY: &str = "$calc";

/// Largest accepted serialized size of one parameter value.
const MAX_PARAMETER_BYTES: usize = 64 * 1024;

/// Most parameters one document may declare.
const MAX_PARAMETERS: usize = 256;

/// Deepest accepted chain of calculated parameter dependencies.
const MAX_CALC_DEPTH: usize = 32;

/// Deepest accepted JSON value traversal during calculation resolution.
const MAX_VALUE_DEPTH: usize = 128;

/// Largest integer magnitude represented exactly by the expression engine's f64.
const MAX_EXACT_INTEGER: f64 = 9_007_199_254_740_992.0;

/// Merges overrides, resolves load-time calculations and substitutes references.
///
/// The root `params` object declares the defaults. Overrides replace whole
/// values by key and unknown keys are rejected. Each `{"$param": "name"}` object
/// anywhere outside `params` is replaced by that parameter's value, so a
/// parameter can supply numbers, colors, strings, points or whole subtrees.
/// Calculations resolve numeric dependencies after overrides, without time or
/// element bindings. Effective values are stored in `params` for expressions.
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
            if serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
                > MAX_PARAMETER_BYTES
            {
                return Err(limit(
                    format!("params.{name}"),
                    "A parameter value is limited to 64 KiB",
                ));
            }
            parameters.insert(name.clone(), value.clone());
        }
    }
    let mut calculations = Calculations {
        parameters: &parameters,
        resolved: Map::new(),
        active: Vec::new(),
        remaining: budget,
    };
    for name in parameters.keys() {
        calculations.parameter(name)?;
    }
    let mut remaining = calculations.remaining;
    let parameters = calculations.resolved;
    for (key, value) in root.iter_mut() {
        substitute(value, &parameters, key, &mut remaining)?;
    }
    if !parameters.is_empty() {
        root.insert("params".into(), Value::Object(parameters));
    }
    Ok(())
}

/// Rewrites a JSON schema to accept calculations and parameter/definition references.
///
/// The document schema describes resolved documents; this relaxes it for editors
/// that validate unresolved files. Reference objects are structural: resolved
/// values are still checked by compilation.
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
        definitions.insert("CalcRef".into(), serde_json::json!({
            "description": "A finite numeric expression evaluated once on load from document parameters, without time or element bindings.",
            "type": "object",
            "properties": {CALC_KEY: {"type": "string", "maxLength": 4096}},
            "required": [CALC_KEY],
            "additionalProperties": false
        }));
        definitions.insert("UseRef".into(), serde_json::json!({
            "description": "A reusable definition, expanded on load; sibling fields override defaults. `use` is a reserved key: any object outside root `params` carrying it is a reference.",
            "type": "object",
            "properties": {"use": {"type": "string"}},
            "required": ["use"],
            "additionalProperties": true
        }));
        definitions.insert(
            "ValueRef".into(),
            serde_json::json!({
                "anyOf": [
                    {"$ref": "#/$defs/ParamRef"},
                    {"$ref": "#/$defs/UseRef"},
                    {"$ref": "#/$defs/CalcRef"}
                ]
            }),
        );
    }
    if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
        properties.insert(
            "defs".into(),
            serde_json::json!({
                "description": "Named reusable values, styles and components, expanded on load.",
                "type": "object",
                "maxProperties": crate::definitions::MAX_DEFINITIONS,
                "additionalProperties": true
            }),
        );
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
    *schema = serde_json::json!({"anyOf": [
        inner, {"$ref": "#/$defs/ValueRef"}
    ]});
}

fn substitute(
    value: &mut Value,
    parameters: &Map<String, Value>,
    path: &str,
    remaining: &mut usize,
) -> Result<(), ValidationError> {
    match value {
        Value::Object(object) => {
            if object.contains_key(CALC_KEY) {
                let source = calculation_source(object, path)?;
                charge(remaining, source.len(), path)?;
                *value = calculation_value(
                    crate::compile::calculate(source, &mut |name| {
                        let value = parameters
                            .get(name)
                            .ok_or_else(|| unknown_parameter(name, parameters))?;
                        numeric(value, name)
                    })
                    .map_err(|message| invalid(path, message))?,
                );
                return Ok(());
            }
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

struct Calculations<'a> {
    parameters: &'a Map<String, Value>,
    resolved: Map<String, Value>,
    active: Vec<String>,
    remaining: usize,
}

impl Calculations<'_> {
    fn parameter(&mut self, name: &str) -> Result<Value, ValidationError> {
        if let Some(value) = self.resolved.get(name) {
            return Ok(value.clone());
        }
        let path = format!("params.{name}");
        if self.active.iter().any(|active| active == name) {
            return Err(invalid(
                &path,
                format!("Calculated parameter dependency cycle at `{name}`"),
            ));
        }
        if self.active.len() >= MAX_CALC_DEPTH {
            return Err(limit(
                &path,
                "Calculated parameter dependencies exceed 32 levels",
            ));
        }
        let source = self
            .parameters
            .get(name)
            .ok_or_else(|| invalid(&path, unknown_parameter(name, self.parameters)))?;
        let mut value = source.clone();
        self.active.push(name.to_owned());
        self.value(&mut value, &path, 0)?;
        self.active.pop();
        self.resolved.insert(name.to_owned(), value.clone());
        Ok(value)
    }

    fn value(
        &mut self,
        value: &mut Value,
        path: &str,
        depth: usize,
    ) -> Result<(), ValidationError> {
        if depth > MAX_VALUE_DEPTH {
            return Err(limit(path, "Calculated values exceed 128 nesting levels"));
        }
        match value {
            Value::Object(object) if object.contains_key(CALC_KEY) => {
                let source = calculation_source(object, path)?;
                charge(&mut self.remaining, source.len(), path)?;
                let mut dependency_error = None;
                let result = crate::compile::calculate(source, &mut |name| {
                    let raw = self
                        .parameters
                        .get(name)
                        .ok_or_else(|| unknown_parameter(name, self.parameters))?;
                    if !raw.is_number() && !raw.is_boolean() && raw.get(CALC_KEY).is_none() {
                        return Err(format!("Parameter `{name}` is not a number"));
                    }
                    let value = self.parameter(name).map_err(|error| {
                        let message = error.to_string();
                        dependency_error = Some(error);
                        message
                    })?;
                    numeric(&value, name)
                })
                .map_err(|message| dependency_error.unwrap_or_else(|| invalid(path, message)))?;
                *value = calculation_value(result);
            }
            Value::Object(object) => {
                for (key, child) in object {
                    self.value(child, &format!("{path}.{key}"), depth + 1)?;
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter_mut().enumerate() {
                    self.value(child, &format!("{path}[{index}]"), depth + 1)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn calculation_source<'a>(
    object: &'a Map<String, Value>,
    path: &str,
) -> Result<&'a str, ValidationError> {
    if object.len() != 1 {
        return Err(invalid(path, "`$calc` cannot have sibling fields"));
    }
    object
        .get(CALC_KEY)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(path, "`$calc` requires an expression string"))
}

fn numeric(value: &Value, name: &str) -> Result<f64, String> {
    match value {
        Value::Bool(flag) => Ok(f64::from(u8::from(*flag))),
        Value::Number(number) => number
            .as_f64()
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("Parameter `{name}` is not finite")),
        _ => Err(format!("Parameter `{name}` is not a number")),
    }
}

fn calculation_value(value: f64) -> Value {
    if value.fract() == 0.0
        && value.abs() <= MAX_EXACT_INTEGER
        && !(value == 0.0 && value.is_sign_negative())
    {
        Value::from(value as i64)
    } else {
        Value::from(value)
    }
}

fn charge(remaining: &mut usize, bytes: usize, path: &str) -> Result<(), ValidationError> {
    *remaining = remaining.checked_sub(bytes).ok_or_else(|| {
        limit(
            path,
            "Parameter calculations exceed the document size limit",
        )
    })?;
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

#[cfg(test)]
mod tests {
    use super::resolve;
    use serde_json::json;

    #[test]
    fn substituted_parameters_are_charged_once_per_use() {
        let value = "x".repeat(1000);
        let size = serde_json::to_vec(&value).unwrap().len();
        let make = || json!({"params": {"label": value}, "content": {"$param": "label"}});
        let mut document = make();
        resolve(&mut document, None, size).unwrap();
        assert_eq!(document["content"], json!(value));
        assert!(resolve(&mut make(), None, size - 1).is_err());
        let mut twice = json!({"params": {"label": value},
            "content": [{"$param": "label"}, {"$param": "label"}]});
        assert!(resolve(&mut twice, None, size * 2 - 1).is_err());
    }
}
