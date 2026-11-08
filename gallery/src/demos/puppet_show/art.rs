//! Inline SVG paper cutouts: puppet limbs, scrolling hill strips and stage sets.
//!
//! Every limb is authored in its own local frame and paired with the pixel
//! pivot that sits on its skeleton joint, so the rig only places and rotates.

use motion::math;
use std::f64::consts::TAU;
use std::fmt::Write;

pub(super) const TUNIC: &str = "#d9573b";
pub(super) const TROUSERS: &str = "#33507a";
pub(super) const SKIN: &str = "#f2c6a0";
pub(super) const SHOE: &str = "#4a2a1c";

/// Wraps SVG body markup in a document of the given size.
fn document(width: f64, height: f64, body: &str) -> String {
    format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{width}' height='{height}' \
         viewBox='0 0 {width} {height}'>{body}</svg>"
    )
}

/// Torso tunic; pivot (32, 90) is the hip joint.
pub(super) fn torso() -> String {
    document(
        64.0,
        96.0,
        &format!(
            "<path d='M15 10 Q32 0 49 10 L58 84 Q32 96 6 84 Z' fill='{TUNIC}' stroke='#9c3524' stroke-width='1.6'/>\
             <path d='M10 70 Q32 80 54 70 L56 78 Q32 88 8 78 Z' fill='#f3d9a4'/>\
             <path d='M21 9 Q32 21 43 9' fill='none' stroke='#f6e6c4' stroke-width='3' stroke-linecap='round'/>\
             <circle cx='32' cy='36' r='3.4' fill='#f6e6c4'/>\
             <circle cx='32' cy='52' r='3.4' fill='#f6e6c4'/>"
        ),
    )
}

/// Profile head facing right; pivot (35, 68) is the neck joint.
pub(super) fn head() -> String {
    document(
        72.0,
        72.0,
        &format!(
            "<rect x='29' y='52' width='12' height='20' rx='4' fill='#d9a982'/>\
             <circle cx='35' cy='36' r='26' fill='{SKIN}' stroke='#c99572' stroke-width='1.4'/>\
             <path d='M10 34 Q8 10 32 9 Q50 8 56 20 Q44 16 36 22 Q24 18 20 36 Z' fill='#6b3b22'/>\
             <circle cx='24' cy='40' r='5.5' fill='#e8b48f'/>\
             <circle cx='61' cy='38' r='5' fill='#eab18c'/>\
             <ellipse cx='48' cy='33' rx='3.2' ry='4.4' fill='#2b1a12'/>\
             <circle cx='49' cy='31.5' r='1.1' fill='#fff8ec'/>\
             <circle cx='47' cy='46' r='5.5' fill='#ec8f7a' opacity='0.55'/>\
             <path d='M45 52 Q51 56 57 50' fill='none' stroke='#7a3324' stroke-width='2.2' stroke-linecap='round'/>"
        ),
    )
}

/// Pompom hat; pivot (24, 40) is the middle of the brim.
pub(super) fn hat() -> String {
    document(
        48.0,
        46.0,
        "<path d='M7 38 Q10 14 26 6 Q40 14 42 38 Z' fill='#e9b237' stroke='#b7801c' stroke-width='1.4'/>\
         <path d='M14 22 Q26 16 38 22' fill='none' stroke='#d9573b' stroke-width='3'/>\
         <rect x='3' y='35' width='42' height='9' rx='4.5' fill='#3c7d6e'/>\
         <circle cx='26' cy='7' r='6.5' fill='#f6e6c4' stroke='#d8c39a' stroke-width='1.2'/>",
    )
}

/// Sleeve; pivot (10, 8) is the shoulder.
pub(super) fn upper_arm(fill: &str) -> String {
    document(
        20.0,
        46.0,
        &format!("<rect x='2' y='0' width='16' height='44' rx='8' fill='{fill}'/>"),
    )
}

/// Forearm and mitten hand; pivot (9, 7) is the elbow.
pub(super) fn forearm(fill: &str) -> String {
    document(
        18.0,
        48.0,
        &format!(
            "<rect x='3' y='0' width='12' height='36' rx='6' fill='{fill}'/>\
             <circle cx='9' cy='38' r='8.5' fill='{fill}'/>"
        ),
    )
}

/// Trouser thigh; pivot (11, 7) is the hip.
pub(super) fn thigh(fill: &str) -> String {
    document(
        22.0,
        48.0,
        &format!("<rect x='2' y='0' width='18' height='46' rx='9' fill='{fill}'/>"),
    )
}

/// Shin and shoe pointing right; pivot (11, 7) is the knee. Sole at y = 50.
pub(super) fn shin(fill: &str, shoe: &str) -> String {
    document(
        42.0,
        52.0,
        &format!(
            "<rect x='3' y='0' width='16' height='40' rx='7' fill='{fill}'/>\
             <path d='M1 38 H24 Q38 38 39 48 V50 H1 Z' fill='{shoe}'/>"
        ),
    )
}

/// One 960-wide periodic hill strip; integer wave counts keep its edges seamless.
pub(super) fn hills(
    fill: &str,
    base: f64,
    amplitude: f64,
    waves: (f64, f64),
    phase: f64,
) -> String {
    let mut path = String::from("M0 540");
    for step in 0..=96 {
        let x = f64::from(step) * 10.0;
        let fraction = x / 960.0;
        let y = base
            - amplitude
                * (0.65 * math::sin(fraction * TAU * waves.0 + phase)
                    + 0.35 * math::sin(fraction * TAU * waves.1 + phase * 2.3));
        let _ = write!(path, " L{x:.1} {y:.1}");
    }
    path.push_str(" L960 540 Z");
    document(960.0, 540.0, &format!("<path d='{path}' fill='{fill}'/>"))
}

/// Grass band with periodic paper flowers and tufts.
pub(super) fn meadow() -> String {
    let mut body = String::from(
        "<rect x='0' y='448' width='960' height='92' fill='#7f9a52'/>\
         <rect x='0' y='448' width='960' height='6' fill='#94ad62'/>",
    );
    for index in 0..12 {
        let x = 40.0 + f64::from(index) * 80.0;
        let y = 476.0 + f64::from(index % 3) * 18.0;
        let petal = ["#f6e6c4", "#f2a65a", "#e98b9a"][index as usize % 3];
        let _ = write!(
            body,
            "<path d='M{x} {y} q-4 -10 -9 -14 M{x} {y} q3 -12 8 -15' stroke='#5d7a3a' \
             stroke-width='3' fill='none' stroke-linecap='round'/>\
             <circle cx='{cx}' cy='{cy}' r='5' fill='{petal}'/>\
             <circle cx='{cx}' cy='{cy}' r='2' fill='#c9782c'/>",
            cx = x + 26.0,
            cy = y - 4.0,
        );
    }
    document(960.0, 540.0, &body)
}

/// Fluffy cloud, 150 x 70.
pub(super) fn cloud() -> String {
    document(
        150.0,
        70.0,
        "<path d='M14 62 Q0 62 4 48 Q8 34 26 38 Q30 14 56 18 Q72 0 94 14 Q118 8 122 32 \
         Q146 32 146 50 Q146 62 130 62 Z' fill='#fff6e6'/>",
    )
}

/// Sun with sixteen paper rays; centre (90, 90).
pub(super) fn sun(fill: &str, ray: &str) -> String {
    let mut body = String::new();
    for index in 0..16 {
        let angle = f64::from(index) * TAU / 16.0;
        let (sin, cos) = (math::sin(angle), math::cos(angle));
        let (side_sin, side_cos) = (math::sin(angle + 0.12), math::cos(angle + 0.12));
        let (other_sin, other_cos) = (math::sin(angle - 0.12), math::cos(angle - 0.12));
        let _ = write!(
            body,
            "<path d='M{:.1} {:.1} L{:.1} {:.1} L{:.1} {:.1} Z' fill='{ray}'/>",
            90.0 + cos * 88.0,
            90.0 + sin * 88.0,
            90.0 + side_cos * 52.0,
            90.0 + side_sin * 52.0,
            90.0 + other_cos * 52.0,
            90.0 + other_sin * 52.0,
        );
    }
    let _ = write!(body, "<circle cx='90' cy='90' r='54' fill='{fill}'/>");
    document(180.0, 180.0, &body)
}

/// One theatre curtain panel with folds and a gold tassel edge, 320 x 400.
pub(super) fn curtain() -> String {
    let mut body = String::from(
        "<path d='M0 0 H320 V384 Q300 400 280 384 Q260 400 240 384 Q220 400 200 384 \
         Q180 400 160 384 Q140 400 120 384 Q100 400 80 384 Q60 400 40 384 Q20 400 0 384 Z' \
         fill='#a8323a'/>",
    );
    for fold in 0..8 {
        let x = 20.0 + f64::from(fold) * 40.0;
        let _ = write!(
            body,
            "<path d='M{x} 0 Q{} 200 {x} 386' stroke='#7f2129' stroke-width='9' fill='none' opacity='0.6'/>",
            x + 8.0
        );
    }
    body.push_str("<rect x='0' y='0' width='320' height='8' fill='#e3b04b'/>");
    document(320.0, 400.0, &body)
}

/// Theatre wall with the stage opening cut out, and perspective floorboards.
pub(super) fn stage() -> String {
    let mut body = String::from(
        "<path d='M0 0 H960 V540 H0 Z M190 92 V430 H770 V92 Z' fill='#5a1f2b' fill-rule='evenodd'/>\
         <rect x='120' y='430' width='720' height='110' fill='#8a5a36'/>\
         <rect x='120' y='430' width='720' height='12' fill='#a8744a'/>",
    );
    for board in 1..8 {
        let x = 120.0 + f64::from(board) * 90.0;
        let _ = write!(
            body,
            "<path d='M{x} 442 L{:.1} 540' stroke='#6e4428' stroke-width='2'/>",
            480.0 + (x - 480.0) * 1.35
        );
    }
    document(960.0, 540.0, &body)
}

/// Gilded proscenium pillars and the scalloped valance, drawn over the curtains.
pub(super) fn proscenium() -> String {
    let mut valance = String::from("M178 60 H782 V96");
    for scallop in (0..12).rev() {
        let x = 178.0 + f64::from(scallop) * 50.33;
        let _ = write!(valance, " Q{:.1} 128 {x:.1} 96", x + 25.0);
    }
    valance.push_str(" Z");
    document(
        960.0,
        540.0,
        &format!(
            "<path d='M166 60 H794 V442 H782 V72 H178 V442 H166 Z' fill='#e3b04b'/>\
             <path d='{valance}' fill='#a8323a'/>\
             <rect x='178' y='60' width='604' height='10' fill='#e3b04b'/>"
        ),
    )
}

/// Swallow-tail ribbon banner, 420 x 90.
pub(super) fn banner() -> String {
    document(
        420.0,
        90.0,
        "<path d='M0 30 H70 V78 H0 L22 54 Z' fill='#9c3524'/>\
         <path d='M420 30 H350 V78 H420 L398 54 Z' fill='#9c3524'/>\
         <path d='M44 12 Q210 -4 376 12 V66 Q210 50 44 66 Z' fill='#d9573b'/>\
         <path d='M54 20 Q210 6 366 20' stroke='#f3d9a4' stroke-width='2' fill='none' stroke-dasharray='6 6'/>",
    )
}
