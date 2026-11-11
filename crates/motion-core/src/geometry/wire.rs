//! Serde and JSON Schema support for [`Point`] and [`Rect`]: fixed-length number arrays.

use super::Point;
use super::bounds::Rect;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

impl Serialize for Point {
    /// Writes `[x, y]`.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        [self.x, self.y].serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Point {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let [x, y] = <[f64; 2]>::deserialize(deserializer)?;
        Ok(Self::new(x, y))
    }
}

impl Serialize for Rect {
    /// Writes the opposite corners as `[left, top, right, bottom]`.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        [self.left, self.top, self.right, self.bottom].serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Rect {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let [left, top, right, bottom] = <[f64; 4]>::deserialize(deserializer)?;
        Ok(Self::new(left, top, right, bottom))
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for Point {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Point".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        <[f64; 2]>::json_schema(generator)
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for Rect {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Rect".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        <[f64; 4]>::json_schema(generator)
    }
}
