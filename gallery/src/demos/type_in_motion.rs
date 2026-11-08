//! Editorial magazine study: a serif cover with spring-settled letters, a typeset
//! two-column spread with a drop cap and pull quote, and a full-bleed closing page,
//! joined by directional page wipes and finished with an ink-sweep mask.

use super::label;
use crate::{DemoMetadata, DemoParameters};
use motion::prelude::*;
use std::f64::consts::{FRAC_PI_2, PI};

const PAPER: u32 = 0xf1ece1;

const INK: u32 = 0x16130f;

const VERMILION: u32 = 0xdc3c2a;

const OCHRE: u32 = 0xe3a72f;

const FOREST: u32 = 0x1f4a3d;

const BLUSH: u32 = 0xe9b9a6;

const DISPLAY: &str = "Didot";

const BODY: &str = "Georgia";

const SANS: &str = "Helvetica Neue";

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "type-in-motion",
    "Room to Move",
    "Editorial",
    &[
        "rich-text",
        "grapheme-animation",
        "spring",
        "paragraph-layout",
        "masks",
        "transitions",
    ],
);

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    let duration = parameters.seconds.seconds();
    let shot = Time::from_ticks(duration.ticks() * 2 / 5);
    let turn = Time::from_ticks(duration.ticks() / 10);
    Ok(Timeline::new(Size::new(960.0, 540.0), 30)
        .track(|track| {
            track
                .scene(shot, cover)
                .transition(
                    Wipe {
                        angle: 0.0,
                        feather: 36.0,
                    },
                    turn,
                )
                .scene(shot, spread)
                .transition(
                    Wipe {
                        angle: FRAC_PI_2,
                        feather: 0.0,
                    },
                    turn,
                )
                .scene(shot, closing)
        })
        .duration(duration))
}

fn color(hex: u32) -> Color {
    Color::from_hex(hex)
}

/// A hairline rule that draws itself from `left` to `right`.
fn rule(left: f64, right: f64, y: f64, thickness: f64, fill: Color, amount: f64) -> Node {
    Rect::new(left, y, left + (right - left) * amount, y + thickness)
        .fill(fill)
        .into()
}

fn small_caps(value: &str, size: f64, fill: Color) -> Text {
    label(value, SANS, size, size * 0.28, FontWeight::BOLD, fill)
}

/// Letters rise out of a clipped line with a spring, one grapheme after another.
fn rising(text: Text, spring: &std::sync::Arc<Spring<f64>>, delay: Duration, lift: f64) -> Text {
    let spring = spring.clone();
    text.split(TextSplit::Characters)
        .animate_parts(65.milliseconds(), move |part| {
            let local = part.time() - delay;
            let amount = spring.sample(local);
            PartStyle::new()
                .offset(0.0, (1.0 - amount) * lift)
                .rotate((1.0 - amount) * -0.1)
                .opacity((local.as_seconds() * 6.0).clamp(0.0, 1.0))
        })
}

fn cover_art(context: &SceneContext<'_>, settle: f64) -> Node {
    let drift = math::sin(context.progress() * PI * 2.0);
    let arch = {
        let mut path = Path2D::new();
        path.move_to(Point::new(0.0, 250.0))
            .line_to(Point::new(0.0, 90.0))
            .quadratic_to(Point::new(0.0, 0.0), Point::new(90.0, 0.0))
            .quadratic_to(Point::new(180.0, 0.0), Point::new(180.0, 90.0))
            .line_to(Point::new(180.0, 250.0))
            .close();
        path
    };
    Node::group((
        Rect::new(0.0, 0.0, 250.0, 330.0).fill(color(FOREST)),
        arch.fill(color(OCHRE))
            .translate(34.0, 54.0 + (1.0 - settle) * 40.0),
        Circle::new(Point::new(96.0, 232.0), 78.0)
            .fill(color(VERMILION))
            .translate(drift * 8.0, 0.0),
        Circle::new(Point::new(96.0, 232.0), 78.0)
            .stroke(1.2, color(PAPER))
            .translate(drift * 8.0 + 14.0, -10.0),
        Rect::new(0.0, 296.0, 250.0, 330.0).fill(color(INK)),
    ))
    .clip_with(Rect::new(0.0, 0.0, 250.0, 330.0), ClipMode::Geometric)
    .translate(660.0, 70.0 + (1.0 - settle) * 24.0)
    .opacity(settle.clamp(0.0, 1.0))
}

fn cover(context: &SceneContext<'_>) -> Node {
    let progress = context.progress();
    let spring = context.cache("masthead-spring", || {
        Spring::new(0.0, 1.0).stiffness(170.0).damping(22.0)
    });
    let settle = spring.sample(context.seconds().seconds() - 250.milliseconds());
    let masthead = rising(
        label("ROOM", DISPLAY, 176.0, -3.0, FontWeight::BOLD, color(INK)),
        &spring,
        0.milliseconds(),
        230.0,
    );
    let reveal = math::smoothstep(0.1, 0.5, progress);
    Node::group((
        context.bounds().fill(color(PAPER)),
        cover_art(context, settle),
        Node::group([masthead.translate(34.0, 40.0)])
            .clip_with(Rect::new(20.0, 20.0, 640.0, 262.0), ClipMode::Geometric),
        rule(40.0, 620.0, 276.0, 3.0, color(INK), reveal),
        small_caps("THE ISSUE ON MOVEMENT", 12.0, color(INK))
            .translate(40.0, 294.0)
            .opacity(reveal),
        small_caps("NO. 07  /  AUTUMN", 12.0, color(VERMILION))
            .translate(400.0, 294.0)
            .opacity(reveal),
        label(
            "How cities, bodies and ideas",
            DISPLAY,
            34.0,
            0.0,
            FontWeight::NORMAL,
            color(INK),
        )
        .translate(40.0, 352.0)
        .opacity(math::smoothstep(0.2, 0.6, progress)),
        label(
            "find the space to change.",
            DISPLAY,
            34.0,
            0.0,
            FontWeight::NORMAL,
            color(VERMILION),
        )
        .translate(40.0, 394.0)
        .opacity(math::smoothstep(0.3, 0.7, progress)),
        rule(40.0, 620.0, 486.0, 1.0, color(INK).with_alpha(0.4), reveal),
        small_caps(
            "ESSAY  42     PHOTOGRAPHY  58     INTERVIEW  76",
            10.0,
            color(INK),
        )
        .translate(40.0, 498.0)
        .opacity(reveal),
    ))
}

const COLUMNS: [&str; 2] = [
    "A room is never only walls. It is the distance between the door and the window, the pause before a sentence, the width of a step. Designers have long known that emptiness is a material.",
    "It tilts a poster toward the eye, gives a headline its breath, lets a crowd become a procession. We spent a season watching what people do with an extra metre of room.",
];

fn spread(context: &SceneContext<'_>) -> Node {
    let progress = context.progress();
    let page = math::smoothstep(0.0, 0.25, progress);
    let quote = math::smoothstep(0.22, 0.6, progress);
    let column = |value: &'static str| {
        Text::new(value)
            .family(BODY)
            .size(12.0)
            .line_height(1.55)
            .max_width(150.0)
            .alignment(TextAlignment::Justified)
            .fill(color(INK))
    };
    let drop_cap = label("T", DISPLAY, 104.0, 0.0, FontWeight::BOLD, color(VERMILION));
    let headline = label("Make", DISPLAY, 92.0, -2.0, FontWeight::BOLD, color(INK))
        .split(TextSplit::Characters)
        .animate_parts(55.milliseconds(), |part| {
            let local = part.time().as_seconds() * 3.5;
            PartStyle::new()
                .offset(0.0, (1.0 - math::smoothstep(0.0, 1.0, local)) * 40.0)
                .opacity(local.clamp(0.0, 1.0))
        });
    let headline_b = label(
        "space.",
        DISPLAY,
        92.0,
        -2.0,
        FontWeight::BOLD,
        color(VERMILION),
    )
    .split(TextSplit::Characters)
    .animate_parts(55.milliseconds(), |part| {
        let local = part.time().as_seconds() * 3.5 - 0.5;
        PartStyle::new()
            .offset(0.0, (1.0 - math::smoothstep(0.0, 1.0, local)) * 40.0)
            .opacity(local.clamp(0.0, 1.0))
    });
    let left = Node::group((
        context.bounds().fill(color(PAPER)),
        // Gutter shadow, as if the page were bound.
        Rect::new(470.0, 0.0, 490.0, 540.0).fill(
            LinearGradient::new(Point::new(470.0, 0.0), Point::new(490.0, 0.0))
                .stop(0.0, color(INK).with_alpha(0.0))
                .stop(0.5, color(INK).with_alpha(0.12))
                .stop(1.0, color(INK).with_alpha(0.0))
                .units(GradientUnits::UserSpace),
        ),
        small_caps("ESSAY", 11.0, color(VERMILION))
            .translate(44.0, 38.0)
            .opacity(page),
        rule(44.0, 436.0, 60.0, 1.0, color(INK), page),
        headline.translate(40.0, 70.0),
        headline_b.translate(40.0, 160.0),
        drop_cap.translate(44.0, 296.0).opacity(page),
        column(COLUMNS[0]).translate(124.0, 300.0).opacity(page),
        column(COLUMNS[1]).translate(286.0, 300.0).opacity(page),
        small_caps("42", 11.0, color(INK)).translate(44.0, 500.0),
    ));
    // Right page: plate and pull quote.
    let right = Node::group((
        Rect::new(520.0, 38.0, 916.0, 270.0).fill(color(BLUSH)),
        Circle::new(
            Point::new(718.0, 154.0),
            74.0 + 10.0 * math::sin(progress * PI * 2.0),
        )
        .fill(color(VERMILION)),
        Rect::new(520.0, 200.0, 916.0, 270.0).fill(color(FOREST)),
        Circle::new(Point::new(838.0, 120.0), 22.0).fill(color(OCHRE)),
        label(
            "Emptiness is a\nmaterial.",
            DISPLAY,
            40.0,
            -0.5,
            FontWeight::NORMAL,
            color(INK),
        )
        .translate(520.0, 304.0 + (1.0 - quote) * 18.0)
        .opacity(quote),
        rule(520.0, 640.0, 410.0, 3.0, color(VERMILION), quote),
        small_caps("MARA LINDQVIST, ARCHITECT", 10.0, color(INK))
            .translate(520.0, 424.0)
            .opacity(quote),
        small_caps("43", 11.0, color(INK)).translate(896.0, 500.0),
    ));
    Node::group([left, right])
}

fn closing(context: &SceneContext<'_>) -> Node {
    let progress = context.progress();
    let sweep = -240.0 + math::smoothstep(0.05, 0.8, progress) * 1320.0;
    let title = |fill: Color| {
        Node::group((
            label("Room", DISPLAY, 200.0, -4.0, FontWeight::BOLD, fill).translate(52.0, 40.0),
            label("to move.", DISPLAY, 200.0, -4.0, FontWeight::BOLD, fill).translate(52.0, 216.0),
        ))
    };
    let sheet = Node::from(Rect::new(0.0, 0.0, 960.0, 540.0).fill(Color::WHITE));
    Node::group((
        context.bounds().fill(color(VERMILION)),
        Circle::new(Point::new(790.0 - progress * 70.0, 270.0), 190.0)
            .stroke(1.5, color(PAPER).with_alpha(0.5)),
        title(color(PAPER)),
        // A band of ink sweeps through the title and inverts it.
        title(color(INK)).mask(
            sheet.clip_with(
                Rect::new(sweep, 0.0, sweep + 190.0, 540.0),
                ClipMode::Geometric,
            ),
            MaskMode::Alpha,
        ),
        small_caps("SUBSCRIBE  /  ROOMMAGAZINE.EXAMPLE", 12.0, color(PAPER))
            .translate(52.0, 482.0)
            .opacity(math::smoothstep(0.4, 0.7, progress)),
        small_caps("NO. 07", 12.0, color(PAPER))
            .translate(824.0, 482.0)
            .opacity(math::smoothstep(0.4, 0.7, progress)),
    ))
}
