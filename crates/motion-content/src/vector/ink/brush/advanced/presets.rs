//! Composable brush presets that configure ordinary brush stroke builders.

use super::super::{BrushStroke, Ink};
use super::NibMode;
use motion_core::{Color, NodeModifiers, Path2D};

/// Composable brush geometry/material presets; all settings remain overridable.
#[derive(Clone, Copy, Debug)]
pub enum BrushPreset {
    /// Fine rounded ink pen.
    Pen,
    /// Flat self-darkening marker.
    Marker,
    /// Tangent-following bristle brush.
    Brush,
    /// Rough granular chalk.
    Chalk,
    /// Rough paper-responsive charcoal.
    Charcoal,
    /// Fine granular pencil.
    Pencil,
    /// Feathered edge-pooling watercolor.
    Watercolor,
    /// Fixed wide nib and tapered pressure.
    Calligraphy,
    /// Translucent marker; `node` applies Multiply compositing.
    Highlighter,
    /// Glow pen; `node` adds the existing linear-light Bloom filter.
    Neon,
}

impl BrushPreset {
    /// Returns a normal BrushStroke builder with deterministic preset parameters.
    pub fn stroke(self, path: Path2D, width: f64, color: Color) -> BrushStroke {
        let stroke = BrushStroke::new(path, width, color);
        match self {
            Self::Pen => stroke.ink(Ink::new(0).edge_roughness(0.1)),
            Self::Marker | Self::Highlighter => stroke
                .nib(0.3, 0.4)
                .nib_mode(NibMode::Fixed(0.4))
                .build_up(0.3),
            Self::Brush => stroke
                .nib(0.5, 0.0)
                .nib_mode(NibMode::FollowPath(0.0))
                .pressure(0.2, 1.0, 0.1)
                .bristles(5, width, 0.2, 0)
                .sample_limit(64)
                .speed_response(0.001)
                .ink(Ink::new(0).dryness(0.25).fibers(0.8).bleed(1.0)),
            Self::Chalk | Self::Charcoal => stroke.ink(
                Ink::new(0)
                    .granulation(0.8)
                    .edge_roughness(1.0)
                    .paper(1, 0.7, 1.0),
            ),
            Self::Pencil => stroke.ink(Ink::new(0).granulation(0.4).paper(1, 0.5, 0.5)),
            Self::Watercolor => stroke
                .ink(
                    Ink::new(0)
                        .bleed(4.0)
                        .edge_pooling(0.7, 2.0)
                        .granulation(0.2),
                )
                .edge_darkening(0.3),
            Self::Calligraphy => stroke
                .nib(0.2, 0.7)
                .nib_mode(NibMode::Fixed(0.7))
                .pressure(0.3, 1.2, 0.1)
                .ink(Ink::new(0).dryness(0.2)),
            Self::Neon => stroke,
        }
    }

    /// Composes preset-specific opacity/blending/lighting after constructing its stroke.
    pub fn node(self, path: Path2D, width: f64, color: Color) -> motion_core::Node {
        let node = motion_core::Node::from(self.stroke(path, width, color));
        match self {
            Self::Highlighter => node
                .opacity(0.45)
                .blend_mode(motion_core::BlendMode::Multiply),
            Self::Marker => node.blend_mode(motion_core::BlendMode::Multiply),
            Self::Watercolor => node
                .opacity(0.6)
                .blend_mode(motion_core::BlendMode::Multiply),
            Self::Neon => node.filter(motion_core::Filter::Bloom {
                threshold: 0.1,
                knee: 0.1,
                radius: 3.0,
                levels: 3,
                falloff: 0.5,
                intensity: 1.0,
                tint: Color::WHITE,
            }),
            _ => node,
        }
    }
}
