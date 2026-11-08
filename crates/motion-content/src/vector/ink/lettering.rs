//! Original geometric fourteen-segment single-line glyphs; no bundled font assets.

use super::BrushStroke;
use motion_core::{Color, Path2D, Point, ValidationError};

/// Bounded single-line Latin lettering built from original segment geometry.
#[derive(Clone, Copy, Debug)]
pub struct StrokeFont {
    height: f64,
}

impl StrokeFont {
    /// Creates uppercase/digit lettering with positive local height, at most 512 units.
    pub fn new(height: f64) -> Result<Self, ValidationError> {
        if !height.is_finite() || !(f64::MIN_POSITIVE..=512.0).contains(&height) {
            return Err(invalid("height", "Require positive finite height <=512"));
        }
        Ok(Self { height })
    }

    /// Emits one extended stroke per glyph; spaces/newlines advance without drawing.
    /// Lowercase is mapped to uppercase; unsupported characters fail instead of substituting.
    pub fn strokes(
        &self,
        text: &str,
        width: f64,
        color: Color,
    ) -> Result<Vec<BrushStroke>, ValidationError> {
        if text.chars().count() > 256
            || !width.is_finite()
            || !(f64::MIN_POSITIVE..=512.0).contains(&width)
        {
            return Err(invalid(
                "text",
                "Require <=256 characters and positive finite width <=512",
            ));
        }
        let segments = [
            [[0.0, 0.0], [1.0, 0.0]],
            [[1.0, 0.0], [1.0, 0.5]],
            [[1.0, 0.5], [1.0, 1.0]],
            [[0.0, 1.0], [1.0, 1.0]],
            [[0.0, 0.5], [0.0, 1.0]],
            [[0.0, 0.0], [0.0, 0.5]],
            [[0.0, 0.5], [0.5, 0.5]],
            [[0.5, 0.5], [1.0, 0.5]],
            [[0.0, 0.0], [0.5, 0.5]],
            [[1.0, 0.0], [0.5, 0.5]],
            [[0.5, 0.5], [0.0, 1.0]],
            [[0.5, 0.5], [1.0, 1.0]],
            [[0.5, 0.0], [0.5, 0.5]],
            [[0.5, 0.5], [0.5, 1.0]],
        ];
        let mut result = Vec::new();
        let mut column = 0.0;
        let mut row = 0.0;
        for character in text.chars() {
            if character == '\n' {
                column = 0.0;
                row += 1.4;
                continue;
            }
            if character == ' ' {
                column += 1.0;
                continue;
            }
            let glyph = glyph(character.to_ascii_uppercase())
                .ok_or_else(|| invalid("glyph", "Unsupported single-line character"))?;
            let mut path = Path2D::new();
            for index in glyph {
                let [first, second] = segments[*index];
                let point = |value: [f64; 2]| {
                    Point::new(
                        (column + value[0] * 0.65) * self.height,
                        (row + value[1]) * self.height,
                    )
                };
                path.try_move_to(point(first))?;
                path.try_line_to(point(second))?;
            }
            result.push(BrushStroke::new(path, width, color));
            column += 1.0;
        }
        Ok(result)
    }
}

fn invalid(field: &str, message: &str) -> ValidationError {
    ValidationError::invalid("stroke_font", field, message)
}

fn glyph(character: char) -> Option<&'static [usize]> {
    Some(match character {
        'A' => &[0, 1, 2, 4, 5, 6, 7],
        'B' | '8' => &[0, 1, 2, 3, 4, 5, 6, 7],
        'C' => &[0, 3, 4, 5],
        'D' | 'O' | '0' => &[0, 1, 2, 3, 4, 5],
        'E' => &[0, 3, 4, 5, 6, 7],
        'F' => &[0, 4, 5, 6, 7],
        'G' => &[0, 2, 3, 4, 5, 7],
        'H' => &[1, 2, 4, 5, 6, 7],
        'I' => &[0, 3, 12, 13],
        'J' => &[1, 2, 3, 4],
        'K' => &[4, 5, 9, 11],
        'L' => &[3, 4, 5],
        'M' => &[1, 2, 4, 5, 8, 9],
        'N' => &[1, 2, 4, 5, 8, 11],
        'P' => &[0, 1, 4, 5, 6, 7],
        'Q' => &[0, 1, 2, 3, 4, 5, 11],
        'R' => &[0, 1, 4, 5, 6, 7, 11],
        'S' | '5' => &[0, 2, 3, 5, 6, 7],
        'T' => &[0, 12, 13],
        'U' => &[1, 2, 3, 4, 5],
        'V' => &[1, 5, 10, 11],
        'W' => &[1, 2, 4, 5, 10, 11],
        'X' => &[8, 9, 10, 11],
        'Y' => &[8, 9, 13],
        'Z' => &[0, 3, 9, 10],
        '1' => &[1, 2],
        '2' => &[0, 1, 3, 4, 6, 7],
        '3' => &[0, 1, 2, 3, 6, 7],
        '4' => &[1, 2, 5, 6, 7],
        '6' => &[0, 2, 3, 4, 5, 6, 7],
        '7' => &[0, 1, 2],
        '9' => &[0, 1, 2, 3, 5, 6, 7],
        '-' => &[6, 7],
        '+' => &[6, 7, 12, 13],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lettering_is_bounded_deterministic_and_asset_free() -> motion_core::Result {
        let font = StrokeFont::new(20.0)?;
        let text = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+-";
        let first = font.strokes(text, 1.0, Color::WHITE)?;
        let second = font.strokes(text, 1.0, Color::WHITE)?;
        assert_eq!(first.len(), 38);
        let nodes = first
            .into_iter()
            .map(motion_core::Node::from)
            .collect::<Vec<_>>();
        let again = second
            .into_iter()
            .map(motion_core::Node::from)
            .collect::<Vec<_>>();
        let compile = |nodes| {
            motion_core::Node::group(nodes).compile(
                motion_core::Size::new(800.0, 40.0),
                motion_core::PixelSize::new(800, 40),
                motion_core::ValidationLimits::default(),
            )
        };
        assert_eq!(compile(nodes)?, compile(again)?);
        assert!(font.strokes("\u{4e2d}", 1.0, Color::WHITE).is_err());
        assert!(font.strokes(&"A".repeat(257), 1.0, Color::WHITE).is_err());
        assert!(StrokeFont::new(f64::NAN).is_err());
        Ok(())
    }
}
