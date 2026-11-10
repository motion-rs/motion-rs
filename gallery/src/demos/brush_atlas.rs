//! Brush preset comparisons using shared geometry and unmodified preset materials.

use super::atlas::{self, Page};
use crate::{DemoMetadata, DemoParameters};
use motion::prelude::*;
use motion::vector::ink::BrushPreset;

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "brush-atlas",
    "Brush Atlas",
    "Brushes",
    &[
        "brush-presets",
        "pressure-nibs",
        "ink-material",
        "blend-modes",
        "bloom",
        "clip-reveal",
        "retained-strokes",
    ],
);

const INK: u32 = 0x252c29;

const PRESETS: [BrushPreset; 10] = [
    BrushPreset::Pen,
    BrushPreset::Marker,
    BrushPreset::Brush,
    BrushPreset::Chalk,
    BrushPreset::Charcoal,
    BrushPreset::Pencil,
    BrushPreset::Watercolor,
    BrushPreset::Calligraphy,
    BrushPreset::Highlighter,
    BrushPreset::Neon,
];

/// Shared nominal nib width in design units; presets retain their own nib and pressure settings.
const NIB_WIDTH: f64 = 18.0;

/// Per-stroke distance-evaluation allowance for the full-length, five-bristle comparison.
const MATERIAL_WORK_LIMIT: u64 = 64_000_000;

pub(super) const PAGES: &[Page] = &[
    Page {
        title: "Pen",
        lines: [
            "A clean pressure stroke",
            "keeps a round nib and",
            "an opaque, smooth edge.",
        ],
    },
    Page {
        title: "Marker",
        lines: [
            "A broad marker follows",
            "the shared path and uses",
            "Multiply compositing.",
        ],
    },
    Page {
        title: "Brush",
        lines: [
            "Five seeded bristles and",
            "pressure form one continuous",
            "stroke with natural variation.",
        ],
    },
    Page {
        title: "Chalk",
        lines: [
            "Dry coverage and grain",
            "break up the same path",
            "without replacing its geometry.",
        ],
    },
    Page {
        title: "Charcoal",
        lines: [
            "A rough, soft-edged mark",
            "retains the preset's material",
            "and pressure settings.",
        ],
    },
    Page {
        title: "Pencil",
        lines: [
            "A fine textured drawing",
            "uses the same nominal nib",
            "and the preset's own response.",
        ],
    },
    Page {
        title: "Watercolor",
        lines: [
            "A translucent pigment wash",
            "uses its material at 0.6",
            "opacity with Multiply.",
        ],
    },
    Page {
        title: "Calligraphy",
        lines: [
            "A shaped nib makes width",
            "depend on path direction",
            "and the preset pressure.",
        ],
    },
    Page {
        title: "Highlighter",
        lines: [
            "A yellow transparent mark",
            "uses 0.45 opacity and",
            "Multiply over the paper.",
        ],
    },
    Page {
        title: "Neon",
        lines: [
            "A luminous turquoise mark",
            "receives local Bloom",
            "against a dark field.",
        ],
    },
];

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    let path = comparison_path();
    Ok(atlas::timeline(
        parameters,
        METADATA,
        PAGES,
        move |page, progress, context| {
            let reveal = Easing::CubicInOut.apply((progress / 0.7).clamp(0.0, 1.0));
            Ok(swatch(context, &path, page, reveal))
        },
    ))
}

fn swatch(context: &SceneContext<'_>, path: &Path2D, index: usize, reveal: f64) -> Node {
    let preset = PRESETS[index];
    let dark = matches!(preset, BrushPreset::Neon);
    let paper = if dark { 0x171c1b } else { 0xffffff };
    let rule = if dark { 0x3e4b46 } else { 0xdce1dc };
    let mut nodes = vec![
        atlas::VIEWPORT.fill(Color::from_hex(paper)).into(),
        Rect::new(28.0, 152.0, 484.0, 153.0)
            .fill(Color::from_hex(rule))
            .into(),
    ];
    for column in 0..5 {
        let left = 40.0 + column as f64 * 108.0;
        nodes.push(
            Rect::new(left, 86.0, left + 1.0, 216.0)
                .fill(Color::from_hex(rule))
                .into(),
        );
    }
    if reveal > 0.0 {
        let stroke = context.memo(("brush-preset", index), || {
            preset_node(preset, path.clone(), NIB_WIDTH, preset_color(preset))
        });
        nodes.push(
            Node::group([stroke
                .pivot(Pivot::Point(Point::ORIGIN))
                .scale(0.86)
                .translate(35.0, 98.0)])
            .clip_with(
                Rect::new(20.0, 42.0, 20.0 + 472.0 * reveal, 264.0),
                ClipMode::Geometric,
            ),
        );
    }
    nodes.push(atlas::text(
        "18-unit nib / shared path",
        13.0,
        if dark { 0xb9cec5 } else { 0x424e47 },
        28.0,
        264.0,
    ));
    Node::group(nodes)
}

fn preset_node(preset: BrushPreset, path: Path2D, width: f64, color: Color) -> Node {
    let node = Node::from(
        preset
            .stroke(path, width, color)
            .material_work_limit(MATERIAL_WORK_LIMIT),
    );
    match preset {
        BrushPreset::Marker => node.blend_mode(BlendMode::Multiply),
        BrushPreset::Watercolor => node.opacity(0.6).blend_mode(BlendMode::Multiply),
        BrushPreset::Highlighter => node.opacity(0.45).blend_mode(BlendMode::Multiply),
        BrushPreset::Neon => node.filter(Filter::Bloom {
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

fn comparison_path() -> Path2D {
    let mut path = Path2D::new();
    path.move_to(Point::new(12.0, 65.0))
        .cubic_to(
            Point::new(110.0, -30.0),
            Point::new(230.0, 10.0),
            Point::new(180.0, 73.0),
        )
        .cubic_to(
            Point::new(125.0, 133.0),
            Point::new(85.0, 33.0),
            Point::new(210.0, 41.0),
        )
        .cubic_to(
            Point::new(300.0, 41.0),
            Point::new(300.0, 108.0),
            Point::new(360.0, 68.0),
        )
        .cubic_to(
            Point::new(405.0, 26.0),
            Point::new(450.0, 27.0),
            Point::new(500.0, 66.0),
        );
    path
}

fn preset_color(preset: BrushPreset) -> Color {
    Color::from_hex(match preset {
        BrushPreset::Highlighter => 0xe9c52a,
        BrushPreset::Neon => 0x52ead2,
        _ => INK,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const PAGE_COUNT: usize = PAGES.len();

    fn preset_name(preset: BrushPreset) -> &'static str {
        match preset {
            BrushPreset::Pen => "Pen",
            BrushPreset::Marker => "Marker",
            BrushPreset::Brush => "Brush",
            BrushPreset::Chalk => "Chalk",
            BrushPreset::Charcoal => "Charcoal",
            BrushPreset::Pencil => "Pencil",
            BrushPreset::Watercolor => "Watercolor",
            BrushPreset::Calligraphy => "Calligraphy",
            BrushPreset::Highlighter => "Highlighter",
            BrushPreset::Neon => "Neon",
        }
    }

    #[test]
    fn presets_cover_all_variants() {
        assert_eq!(
            PRESETS.map(preset_name),
            [
                "Pen",
                "Marker",
                "Brush",
                "Chalk",
                "Charcoal",
                "Pencil",
                "Watercolor",
                "Calligraphy",
                "Highlighter",
                "Neon",
            ]
        );
        assert_eq!(
            PRESETS
                .iter()
                .map(std::mem::discriminant)
                .collect::<HashSet<_>>()
                .len(),
            10
        );
        assert_eq!(PAGE_COUNT, 10);
        assert_eq!(PAGES.len(), PRESETS.len());
        for (page, preset) in PAGES.iter().zip(PRESETS) {
            assert_eq!(page.title, preset_name(preset));
        }
        for page in 0..PAGE_COUNT {
            let (actual, local) =
                atlas::page_at((page as f64 + 0.5) / PAGE_COUNT as f64, PAGE_COUNT);
            assert_eq!(actual, page);
            assert!((local - 0.5).abs() <= 1e-12);
        }
        assert_eq!(atlas::page_at(1.0, PAGE_COUNT), (PAGE_COUNT - 1, 1.0));
    }

    #[test]
    fn timeline_retains_shared_size_frame_rate_and_duration() -> motion::Result {
        for seconds in [1, 8, 20, 60] {
            let timeline = timeline(DemoParameters {
                seconds,
                ..DemoParameters::default()
            })?;
            assert_eq!(timeline.size(), Size::new(960.0, 540.0));
            assert_eq!(timeline.total_duration(), seconds.seconds());
            assert_eq!(timeline.frame_count(), u64::from(seconds) * 30);
        }
        Ok(())
    }

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT; CPU stills with the demo's explicit material budget"]
    fn every_preset_animates_and_replays_exactly() -> motion::Result {
        let font = std::env::var_os("MOTION_TEST_FONT")
            .ok_or_else(|| std::io::Error::other("Set MOTION_TEST_FONT"))?;
        let options = crate::render_options(Some(std::path::Path::new(&font)))?
            .backend(BackendPreference::Cpu)
            .size(PixelSize::new(288, 162));
        let timeline = timeline(DemoParameters {
            seconds: 20,
            ..DemoParameters::default()
        })?;
        for page in 0..PAGE_COUNT {
            let moment = |local: f64| ((page as f64 + local) * 20.0 / PAGE_COUNT as f64).seconds();
            let early = render_still(&timeline, moment(0.15), &options)?;
            let later = render_still(&timeline, moment(0.8), &options)?;
            let replay = render_still(&timeline, moment(0.15), &options)?;
            assert_eq!(early.bytes(), replay.bytes(), "page {page} replay");
            let changed = (46..136)
                .flat_map(|vertical| {
                    (117..269).map(move |horizontal| (vertical * 288 + horizontal) * 4)
                })
                .filter(|offset| {
                    early.bytes()[*offset..*offset + 4] != later.bytes()[*offset..*offset + 4]
                })
                .count();
            assert!(changed > 8, "preset {} did not animate", page);
        }
        Ok(())
    }

    #[test]
    fn configured_budget_preserves_builtin_preset_compositing() -> motion::Result {
        let mut path = Path2D::new();
        path.move_to(Point::new(24.0, 48.0)).cubic_to(
            Point::new(42.0, 14.0),
            Point::new(70.0, 82.0),
            Point::new(102.0, 44.0),
        );
        let options = RenderOptions::default()
            .backend(BackendPreference::Cpu)
            .size(PixelSize::new(128, 96));
        for preset in PRESETS {
            let background = if matches!(preset, BrushPreset::Neon) {
                0x171c1b
            } else {
                atlas::PAPER
            };
            let render = |node: Node| {
                let timeline = Timeline::new(Size::new(128.0, 96.0), 30)
                    .background(Color::from_hex(background))
                    .track(|track| track.scene(1.seconds(), move |_| node.clone()));
                render_still(&timeline, 0.5.seconds(), &options)
            };
            let expected = render(preset.node(path.clone(), NIB_WIDTH, preset_color(preset)))?;
            let actual = render(preset_node(
                preset,
                path.clone(),
                NIB_WIDTH,
                preset_color(preset),
            ))?;
            assert_eq!(
                actual.bytes(),
                expected.bytes(),
                "{} compositing",
                preset_name(preset)
            );
        }
        Ok(())
    }
}
