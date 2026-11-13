//! Bounded load-time expansion of named document values, styles and components.

use motion_core::ValidationError;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::io::{self, Write};

/// Most reusable definitions one document may declare.
pub(crate) const MAX_DEFINITIONS: usize = 256;

/// Largest serialized definition, in bytes before expansion.
const MAX_DEFINITION_BYTES: usize = 64 * 1024;

/// Most nested reusable references along one expansion path.
const MAX_REFERENCE_DEPTH: usize = 32;

/// Most JSON containers along one expanded value path.
const MAX_VALUE_DEPTH: usize = 128;

pub(crate) fn resolve(document: &mut Value, budget: usize) -> Result<(), ValidationError> {
    let Some(root) = document.as_object_mut() else {
        return Ok(());
    };
    let definitions = match root.remove("defs") {
        None => Map::new(),
        Some(Value::Object(definitions)) => definitions,
        Some(_) => return Err(invalid("defs", "Definitions must be a JSON object")),
    };
    if definitions.len() > MAX_DEFINITIONS {
        return Err(limit("defs", "Documents declare at most 256 definitions"));
    }
    let mut sizes = BTreeMap::new();
    for (name, value) in &definitions {
        let size = serialized_size(value, MAX_DEFINITION_BYTES)
            .ok_or_else(|| limit(format!("defs.{name}"), "A definition is limited to 64 KiB"))?;
        sizes.insert(name.clone(), size);
    }
    let size = serialized_size(root, budget).ok_or_else(|| {
        limit(
            "document",
            "Definition expansion exceeds the document size limit",
        )
    })?;
    let mut expansion = Expansion {
        definitions: &definitions,
        sizes: &sizes,
        remaining: budget - size,
        references: Vec::new(),
    };
    for (key, value) in root.iter_mut() {
        if key != "params" {
            expansion.expand(value, key, 0)?;
        }
    }
    Ok(())
}

struct Expansion<'a> {
    definitions: &'a Map<String, Value>,
    sizes: &'a BTreeMap<String, usize>,
    remaining: usize,
    references: Vec<String>,
}

impl Expansion<'_> {
    fn expand(
        &mut self,
        value: &mut Value,
        path: &str,
        depth: usize,
    ) -> Result<(), ValidationError> {
        if depth > MAX_VALUE_DEPTH {
            return Err(limit(path, "Expanded JSON nesting exceeds 128 levels"));
        }
        match value {
            Value::Object(object) if object.contains_key("use") => {
                let name = object["use"]
                    .as_str()
                    .ok_or_else(|| invalid(path, "`use` requires a definition name string"))?
                    .to_owned();
                let definition = self.definitions.get(&name).ok_or_else(|| {
                    let mut message = format!("Unknown definition: {name}");
                    if let Some(hint) = crate::diagnostics::did_you_mean(
                        &name,
                        self.definitions.keys().map(String::as_str),
                    ) {
                        message.push_str(". ");
                        message.push_str(&hint);
                    }
                    invalid(path, message)
                })?;
                if self.references.contains(&name) {
                    return Err(invalid(
                        path,
                        format!("Cyclic definition reference: {name}"),
                    ));
                }
                if self.references.len() >= MAX_REFERENCE_DEPTH {
                    return Err(limit(path, "Definition references exceed 32 levels"));
                }
                if object.len() > 1 && !definition.is_object() {
                    return Err(invalid(
                        path,
                        "Only object definitions accept field overrides",
                    ));
                }
                self.remaining =
                    self.remaining
                        .checked_sub(self.sizes[&name])
                        .ok_or_else(|| {
                            limit(path, "Definition expansion exceeds the document size limit")
                        })?;
                let mut replacement = definition.clone();
                object.remove("use");
                if let Some(defaults) = replacement.as_object_mut() {
                    defaults.append(object);
                }
                *value = replacement;
                self.references.push(name);
                let result = self.expand(value, path, depth);
                self.references.pop();
                result
            }
            Value::Object(object) => {
                for (key, child) in object {
                    self.expand(child, &format!("{path}.{key}"), depth + 1)?;
                }
                Ok(())
            }
            Value::Array(items) => {
                for (index, child) in items.iter_mut().enumerate() {
                    self.expand(child, &format!("{path}[{index}]"), depth + 1)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

struct SizeCounter {
    limit: usize,
    bytes: usize,
}

impl Write for SizeCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .filter(|total| *total <= self.limit)
            .ok_or_else(|| io::Error::other("Serialized value exceeds its byte limit"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn serialized_size(value: &impl Serialize, limit: usize) -> Option<usize> {
    let mut counter = SizeCounter { limit, bytes: 0 };
    serde_json::to_writer(&mut counter, value).ok()?;
    Some(counter.bytes)
}

fn invalid(path: impl Into<String>, message: impl Into<String>) -> ValidationError {
    ValidationError::invalid(path, "defs", message)
}

fn limit(path: impl Into<String>, message: impl Into<String>) -> ValidationError {
    ValidationError::limit(path, "defs", message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn definitions_expand_values_styles_and_components() {
        let mut document = json!({
            "defs": {
                "radius": 4,
                "style": {"fill": "red", "opacity": 0.5},
                "dot": {"use": "style", "circle": {"radius": {"use": "radius"}}}
            },
            "content": {"children": [
                {"use": "dot", "fill": "blue", "id": "first"},
                {"use": "dot", "id": "second"}
            ]}
        });
        resolve(&mut document, 4096).unwrap();
        assert_eq!(
            document,
            json!({"content": {"children": [
                {"fill": "blue", "opacity": 0.5, "circle": {"radius": 4}, "id": "first"},
                {"fill": "red", "opacity": 0.5, "circle": {"radius": 4}, "id": "second"}
            ]}})
        );
    }

    #[test]
    fn definitions_replace_whole_fields_and_arrays() {
        let mut document = json!({
            "defs": {"shape": {"rect": {"bounds": [0, 0, 4, 4]}}, "points": [1, 2]},
            "content": {"use": "shape", "rect": {"bounds": [0, 0, 8, 8]}},
            "position": {"use": "points"}
        });
        resolve(&mut document, 4096).unwrap();
        assert_eq!(document["content"]["rect"], json!({"bounds": [0, 0, 8, 8]}));
        assert_eq!(document["position"], json!([1, 2]));
    }

    #[test]
    fn definitions_reject_cycles_and_unknown_names_at_referring_paths() {
        for (definitions, name, message) in [
            (json!({"dot": {"use": "dot"}}), "dot", "Cyclic"),
            (
                json!({"first": {"use": "second"}, "second": {"use": "first"}}),
                "first",
                "Cyclic",
            ),
            (json!({"dot": {}}), "dots", "Did you mean"),
        ] {
            let mut document = json!({"defs": definitions, "content": {"use": name}});
            let error = resolve(&mut document, 4096).unwrap_err().to_string();
            assert!(error.contains("content"), "{error}");
            assert!(error.contains(message), "{error}");
        }
    }

    #[test]
    fn definitions_admit_bytes_before_cloning() {
        let mut document = json!({
            "defs": {"pair": [{"use": "leaf"}, {"use": "leaf"}], "leaf": "x".repeat(64)},
            "content": {"use": "pair"}
        });
        let error = resolve(&mut document, 150).unwrap_err().to_string();
        assert!(error.contains("content[1]"), "{error}");
        assert!(error.contains("size limit"), "{error}");
    }

    #[test]
    fn definitions_reject_malformed_references() {
        for reference in [json!({"use": 1}), json!({"use": "scalar", "fill": "red"})] {
            let mut document = json!({"defs": {"scalar": 1}, "content": reference});
            assert!(resolve(&mut document, 4096).is_err());
        }
        let mut document = json!({"defs": [], "content": {}});
        assert!(resolve(&mut document, 4096).is_err());
    }

    #[test]
    fn definitions_enforce_count_size_and_reference_depth() {
        let mut definitions = Map::new();
        for index in 0..=MAX_DEFINITIONS {
            definitions.insert(index.to_string(), json!(0));
        }
        let mut document = json!({"defs": definitions});
        assert!(resolve(&mut document, crate::MAX_DOCUMENT_BYTES).is_err());
        let mut document = json!({"defs": {"large": "x".repeat(MAX_DEFINITION_BYTES)}});
        assert!(resolve(&mut document, crate::MAX_DOCUMENT_BYTES).is_err());

        let mut definitions = Map::new();
        for index in 0..MAX_REFERENCE_DEPTH {
            definitions.insert(index.to_string(), json!({"use": (index + 1).to_string()}));
        }
        definitions.insert(MAX_REFERENCE_DEPTH.to_string(), json!(1));
        let mut document = json!({"defs": definitions, "value": {"use": "1"}});
        let mut accepted = document.clone();
        resolve(&mut accepted, 4096).unwrap();
        assert_eq!(accepted["value"], 1);
        document["value"]["use"] = json!("0");
        let error = resolve(&mut document, 4096).unwrap_err().to_string();
        assert!(error.contains("32 levels"), "{error}");
    }

    #[test]
    fn definitions_keep_total_expansion_and_nesting_bounded() {
        let mut definitions = Map::new();
        definitions.insert("0".into(), json!("x"));
        for index in 1..=16 {
            let reference = json!({"use": (index - 1).to_string()});
            definitions.insert(index.to_string(), json!([reference, reference]));
        }
        let mut document = json!({"defs": definitions, "value": {"use": "16"}});
        let error = resolve(&mut document, 4096).unwrap_err().to_string();
        assert!(error.contains("size limit"), "{error}");

        let mut nested = json!(0);
        for _ in 0..=MAX_VALUE_DEPTH {
            nested = json!([nested]);
        }
        let mut document = json!({"value": nested});
        let error = resolve(&mut document, 4096).unwrap_err().to_string();
        assert!(error.contains("128 levels"), "{error}");
    }

    #[test]
    fn definitions_load_with_parameters_and_round_trip_resolved_values() {
        let text = r#"{
            "version": 1, "size": [16, 16], "fps": 10,
            "params": {"tint": "red"},
            "defs": {
                "style": {"fill": {"$param": "tint"}},
                "tile": {"use": "style", "rect": {"bounds": [0, 0, 16, 16]}}
            },
            "tracks": [{"items": [{"scene": {"duration": 1, "content": {"use": "tile"}}}]}]
        }"#;
        let document =
            crate::Document::from_json_with(text, Some(&json!({"tint": "blue"}))).unwrap();
        let resolved = document.to_json().unwrap();
        assert!(!resolved.contains("\"defs\""));
        assert!(!resolved.contains("\"use\""));
        assert_eq!(crate::Document::from_json(&resolved).unwrap(), document);
        let compiled = document.compile(&Default::default()).unwrap();
        let frame = motion_render::render_still(
            compiled.timeline(),
            motion_core::Time::ZERO,
            &motion_render::RenderOptions::default(),
        )
        .unwrap();
        assert!(
            frame
                .bytes()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| *pixel == [0, 0, 255, 255])
        );
    }

    #[test]
    fn definitions_locate_expanded_type_errors_at_the_use_site() {
        let text = r#"{
            "version": 1, "size": [16, 16], "fps": 10,
            "defs": {"bad": {"circle": {"center": [0, 0], "radius": "wrong"}}},
            "tracks": [{"items": [{"scene": {"duration": 1, "content": {"use": "bad"}}}]}]
        }"#;
        let error = crate::Document::from_json(text).unwrap_err();
        let diagnostic = error.downcast_ref::<ValidationError>().unwrap();
        assert!(
            diagnostic
                .path
                .starts_with("tracks[0].items[0].scene.content.circle.radius"),
            "{diagnostic:?}"
        );
        assert!(diagnostic.message.contains("line 4"), "{diagnostic:?}");
    }
}
