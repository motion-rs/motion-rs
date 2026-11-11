//! Compilation of expressions and `@id.property` bindings between elements.

use super::Compiler;
use super::expr::{ExprAnimation, parse};
use super::value::Prop;
use crate::element::Element;
use crate::value::Animated;
use motion_core::{Animation, Duration};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

/// Deepest chain of bindings that may refer to one another.
const MAX_BINDING_DEPTH: usize = 32;

/// Most distinct bindings one scene may resolve.
const MAX_BINDINGS: usize = 1024;

/// A numeric signal shared between the binding cache and expression terms.
type Signal = Arc<dyn Animation<f64>>;

impl<'a> Compiler<'a> {
    /// Records the element identifiers of one scene before its properties compile.
    pub(crate) fn index_ids(&mut self, element: &'a Element, path: &str) {
        if let Some(id) = &element.id {
            let valid = id
                .chars()
                .next()
                .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
                && id
                    .chars()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == '_')
                && id.len() <= 128;
            if !valid {
                self.error(
                    path,
                    "id",
                    "Ids are 1..=128 ASCII letters, digits or `_`, not starting with a digit",
                );
            } else if self.ids.insert(id.clone(), element).is_some() {
                self.error(
                    path,
                    "id",
                    format!("Duplicate element id `{id}` in this scene"),
                );
            }
        }
        if let crate::content::Content::Group(children) = &element.content {
            for (index, child) in children.iter().enumerate() {
                self.index_ids(child, &format!("{path}.children[{index}]"));
            }
        }
        if let Some(mask) = &element.mask {
            self.index_ids(&mask.element, &format!("{path}.mask.element"));
        }
    }

    /// Compiles one numeric expression into a sampled animation.
    pub(crate) fn expression(&mut self, source: &str, path: &str) -> Option<Signal> {
        let parsed = match parse(source, self.params) {
            Ok(parsed) => parsed,
            Err(message) => {
                self.error(path, "expr", message);
                return None;
            }
        };
        let mut references = Vec::with_capacity(parsed.references.len());
        for reference in &parsed.references {
            references.push(self.reference(reference, path));
        }
        let references = references.into_iter().collect::<Option<Vec<_>>>()?;
        Some(Arc::new(ExprAnimation::new(parsed.expr, references)))
    }

    /// Resolves `id.path.to.property` to the other element's own numeric property.
    fn reference(&mut self, reference: &str, path: &str) -> Option<Signal> {
        if let Some(signal) = self.bindings.get(reference) {
            return Some(signal.clone());
        }
        if self.resolving.iter().any(|active| active == reference) {
            let chain = self.resolving.join(" -> ");
            self.error(
                path,
                "expr",
                format!("Binding cycle: {chain} -> {reference}"),
            );
            return None;
        }
        if self.resolving.len() >= MAX_BINDING_DEPTH || self.bindings.len() >= MAX_BINDINGS {
            self.limit(
                path,
                "expr",
                "Bindings chain too deeply or are too numerous",
            );
            return None;
        }
        self.resolving.push(reference.to_owned());
        let signal = self.resolve_reference(reference, path);
        self.resolving.pop();
        let signal = signal?;
        self.bindings.insert(reference.to_owned(), signal.clone());
        Some(signal)
    }

    fn resolve_reference(&mut self, reference: &str, path: &str) -> Option<Signal> {
        let segments: Vec<&str> = reference.split('.').collect();
        let (id, property) = (segments[0], &segments[1..]);
        let Some(element) = self.ids.get(id).copied() else {
            let mut message = format!("No element with id `{id}` in this scene");
            if let Some(hint) =
                crate::diagnostics::did_you_mean(id, self.ids.keys().map(String::as_str))
            {
                message.push_str(". ");
                message.push_str(&hint);
            }
            self.error(path, "expr", message);
            return None;
        };
        if property.is_empty() || property.iter().any(|segment| segment.is_empty()) {
            self.error(
                path,
                "expr",
                format!("`@{reference}` must name a property, as `@{id}.property`"),
            );
            return None;
        }
        if !self.documents.contains_key(id) {
            let value = serde_json::to_value(element).ok()?;
            self.documents.insert(id.to_owned(), value);
        }
        let document = &self.documents[id];
        let here = format!("{path} (@{reference})");
        if let Some(value) = descend(document, property)
            && let Ok(animated) = Animated::<f64>::deserialize(value.clone())
        {
            let property = self.animated(&animated, &here)?;
            return Some(scalar(property));
        }
        let (last, parent) = property.split_last()?;
        let component = match *last {
            "x" | "left" | "0" => Some(0),
            "y" | "top" | "1" => Some(1),
            "right" | "2" => Some(2),
            "bottom" | "3" => Some(3),
            _ => None,
        };
        if let (Some(component), Some(value)) = (component, descend(document, parent)) {
            if let Ok(animated) = Animated::<[f64; 2]>::deserialize(value.clone())
                && component < 2
            {
                let property = self.animated(&animated, &here)?;
                return Some(project(property, component));
            }
            if let Ok(animated) = Animated::<[f64; 4]>::deserialize(value.clone()) {
                let property = self.animated(&animated, &here)?;
                return Some(project(property, component));
            }
        }
        self.error(
            path,
            "expr",
            format!("`@{reference}` does not name a numeric property of `{id}`"),
        );
        None
    }
}

/// Follows object keys and array indices through a serialized element.
fn descend<'v>(mut value: &'v Value, segments: &[&str]) -> Option<&'v Value> {
    for segment in segments {
        value = match value {
            Value::Object(object) => object.get(*segment)?,
            Value::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(value)
}

pub(super) fn scalar(property: Prop<f64>) -> Signal {
    match property {
        Prop::Fixed(value) => Arc::new(move |_: Duration| value),
        Prop::Varying(animation) => animation,
    }
}

fn project<const COUNT: usize>(property: Prop<[f64; COUNT]>, component: usize) -> Signal {
    match property {
        Prop::Fixed(value) => {
            let value = value[component];
            Arc::new(move |_: Duration| value)
        }
        Prop::Varying(animation) => {
            Arc::new(move |time: Duration| animation.sample(time)[component])
        }
    }
}
