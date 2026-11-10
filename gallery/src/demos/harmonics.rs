//! Combines native Typst formula writing, semantic formula morphing and an
//! animated function graph into a mathematical film about doubling frequency.

use crate::{DemoMetadata, DemoParameters};
use motion::eager::FormulaCompiler;
use motion::prelude::*;
use std::f64::consts::TAU;

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "harmonics",
    "Harmonics",
    "Mathematics",
    &[
        "typst-formulas",
        "formula-writing",
        "semantic-morph",
        "function-graphs",
    ],
);

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    let compiler = FormulaCompiler::new();
    let options = FormulaOptions {
        size: 48.0,
        fill: Color::from_hex(0x182628),
        ..FormulaOptions::default()
    };
    let first = compiler.compile("f(t) = sin(2 pi t)", &options)?;
    let second = compiler.compile("f(t) = sin(4 pi t)", &options)?;
    let morph = first.morph_to(&second)?;
    Timeline::new(Size::new(960.0, 540.0), 30)
        .track(|track| {
            track.scene(
                parameters.seconds.seconds(),
                move |context: &SceneContext<'_>| -> motion::Result<_> {
                    let progress = context.progress();
                    let change = ((progress - 0.4) / 0.25).clamp(0.0, 1.0);
                    let change = change * change * (3.0 - 2.0 * change);
                    let equation = if progress < 0.35 {
                        first.write((progress / 0.3).clamp(0.0, 1.0))?
                    } else {
                        morph.sample(change)?
                    };
                    let frequency = 1.0 + change;
                    let phase = context.seconds() * 0.8;
                    let mut path = Path2D::new();
                    for sample in 0..=160 {
                        let fraction = sample as f64 / 160.0;
                        let point = Point::new(
                            80.0 + fraction * 800.0,
                            350.0 - math::sin(fraction * TAU * frequency - phase) * 70.0,
                        );
                        if sample == 0 {
                            path.try_move_to(point)?;
                        } else {
                            path.try_line_to(point)?;
                        }
                    }
                    let cursor = (progress * 0.7 + 0.1).rem_euclid(1.0);
                    let point = Point::new(
                        80.0 + cursor * 800.0,
                        350.0 - math::sin(cursor * TAU * frequency - phase) * 70.0,
                    );
                    Ok((
                        context.bounds().fill(Color::from_hex(0xf1f6f5)),
                        Text::new("HARMONICS")
                            .size(23.0)
                            .fill(Color::from_hex(0x182628))
                            .translate(40.0, 34.0),
                        equation.pivot(Pivot::Center).at(Point::new(480.0, 173.0)),
                        Rect::new(80.0, 350.0, 880.0, 351.0).fill(Color::from_hex(0xc0cfcb)),
                        path.stroke(3.0, Color::from_hex(0x188b75)),
                        Circle::new(point, 7.0)?.fill(Color::from_hex(0xeb7257)),
                        Text::new("One rhythm becomes another.")
                            .size(17.0)
                            .fill(Color::from_hex(0x182628))
                            .translate(40.0, 478.0),
                    ))
                },
            )
        })
        .build()
}
