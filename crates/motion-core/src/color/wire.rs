//! Serde and JSON Schema support for [`Color`]: one CSS-style string.

use crate::Color;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

impl Serialize for Color {
    /// Writes `#rrggbb[aa]` when the color is exactly representable in 8 bits and
    /// `rgb(r g b / a)` with fractional channels otherwise.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let [red, green, blue, alpha] = self.to_rgba8();
        if Color::rgba(red, green, blue, alpha) == *self {
            if alpha == u8::MAX {
                serializer.collect_str(&format_args!("#{red:02x}{green:02x}{blue:02x}"))
            } else {
                serializer.collect_str(&format_args!("#{red:02x}{green:02x}{blue:02x}{alpha:02x}"))
            }
        } else {
            let [red, green, blue, alpha] = self.to_rgba_float();
            serializer.collect_str(&format_args!(
                "rgb({} {} {} / {alpha})",
                red * 255.0,
                green * 255.0,
                blue * 255.0
            ))
        }
    }
}

impl<'de> Deserialize<'de> for Color {
    /// Accepts the syntax of [`Color::parse`].
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = std::borrow::Cow::<str>::deserialize(deserializer)?;
        Color::parse(&text).map_err(D::Error::custom)
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for Color {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Color".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "Named, #hex, rgb(), hsl() or oklch() color syntax.",
            "examples": ["#14a082", "rgb(20 160 130 / 50%)", "white"]
        })
    }
}
