//! A paper-cutout puppet rigged with 2D forward kinematics: inline SVG limbs
//! hang on a `Skeleton`, while procedural gaits and `Keyframes<Pose>` blends
//! make Pip walk past parallax hills, wave from a theatre stage backed by a
//! photographic `Image`, and jump for joy, with built-in `Iris` and `Push`
//! scene transitions joining the three shots.

mod art;

use crate::{DemoMetadata, DemoParameters};
use motion::prelude::*;
use motion::svg::SvgAsset;
use std::f64::consts::{PI, TAU};
use std::sync::Arc;

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "puppet-show",
    "Pip Takes a Bow",
    "Character",
    &[
        "svg-artwork",
        "skeleton-rig",
        "forward-kinematics",
        "iris-transition",
        "image-backdrop",
        "pose-keyframes",
        "spring",
        "push-transition",
    ],
);

const BONES: usize = 12;
const HIP: usize = 0;
const TORSO: usize = 1;
const HEAD: usize = 2;
const HAT: usize = 3;
const ARM_BACK: usize = 4;
const HAND_BACK: usize = 5;
const ARM_FRONT: usize = 6;
const HAND_FRONT: usize = 7;
const THIGH_BACK: usize = 8;
const SHIN_BACK: usize = 9;
const THIGH_FRONT: usize = 10;
const SHIN_FRONT: usize = 11;

/// Distance from the hip joint down to the shoe soles in rest pose.
const LEG: f64 = 75.0;
const STEP_RATE: f64 = 0.9;
const WALK_SPEED: f64 = 120.0;
const JUMP_PERIOD: f64 = 1.25;

fn color(hex: u32) -> Color {
    Color::from_hex(hex)
}

fn svg(markup: String) -> motion::Result<Arc<SvgAsset>> {
    Ok(Arc::new(SvgAsset::from_bytes(markup.as_bytes())?))
}

fn place(asset: &Arc<SvgAsset>, x: f64, y: f64) -> Node {
    Svg::from_asset(asset.clone()).translate(x, y)
}

/// One rigid cutout: the bone it rides, its artwork, and the joint pivot inside it.
struct Part {
    bone: usize,
    asset: Arc<SvgAsset>,
    pivot: (f64, f64),
    pinned: bool,
}

struct Puppet {
    rig: Skeleton<BONES>,
    /// Parts in painter order; back limbs first, front arm last.
    parts: Vec<Part>,
}

impl Puppet {
    fn new() -> motion::Result<Self> {
        let rig = Skeleton::new([
            Bone::new(None, (0.0, 0.0)),
            Bone::new(Some(HIP), (0.0, 0.0)),
            Bone::new(Some(TORSO), (0.0, -80.0)),
            Bone::new(Some(HEAD), (2.0, -50.0)),
            Bone::new(Some(TORSO), (-3.0, -70.0)),
            Bone::new(Some(ARM_BACK), (0.0, 32.0)),
            Bone::new(Some(TORSO), (3.0, -70.0)),
            Bone::new(Some(ARM_FRONT), (0.0, 32.0)),
            Bone::new(Some(HIP), (-5.0, -4.0)),
            Bone::new(Some(THIGH_BACK), (0.0, 36.0)),
            Bone::new(Some(HIP), (5.0, -4.0)),
            Bone::new(Some(THIGH_FRONT), (0.0, 36.0)),
        ])?;
        let shade_tunic = "#b0442d";
        let shade_skin = "#d9a982";
        let shade_trousers = "#263d5e";
        let part = |bone, markup, pivot, pinned| -> motion::Result<Part> {
            Ok(Part {
                bone,
                asset: svg(markup)?,
                pivot,
                pinned,
            })
        };
        let parts = vec![
            part(ARM_BACK, art::upper_arm(shade_tunic), (10.0, 8.0), true)?,
            part(HAND_BACK, art::forearm(shade_skin), (9.0, 7.0), true)?,
            part(THIGH_BACK, art::thigh(shade_trousers), (11.0, 7.0), false)?,
            part(
                SHIN_BACK,
                art::shin(shade_trousers, "#33201a"),
                (11.0, 7.0),
                true,
            )?,
            part(THIGH_FRONT, art::thigh(art::TROUSERS), (11.0, 7.0), false)?,
            part(
                SHIN_FRONT,
                art::shin(art::TROUSERS, art::SHOE),
                (11.0, 7.0),
                true,
            )?,
            part(TORSO, art::torso(), (32.0, 90.0), false)?,
            part(HEAD, art::head(), (35.0, 68.0), false)?,
            part(HAT, art::hat(), (24.0, 40.0), false)?,
            part(ARM_FRONT, art::upper_arm(art::TUNIC), (10.0, 8.0), true)?,
            part(HAND_FRONT, art::forearm(art::SKIN), (9.0, 7.0), true)?,
        ];
        Ok(Self { rig, parts })
    }

    /// Solves the pose and pins every cutout to its joint, with brass split pins.
    fn draw(&self, pose: &Pose<BONES>) -> motion::Result<Node> {
        let solved = self.rig.solve(pose)?;
        let mut nodes = Vec::with_capacity(self.parts.len() * 2);
        for part in &self.parts {
            let joint = solved[part.bone];
            nodes.push(joint.apply(
                Svg::from_asset(part.asset.clone()).translate(-part.pivot.0, -part.pivot.1),
            ));
            if part.pinned {
                let (x, y) = joint.position();
                nodes.push(
                    Circle::new(Point::new(x, y), 3.4)
                        .fill(color(0xc9a24a))
                        .into(),
                );
                nodes.push(
                    Circle::new(Point::new(x - 0.8, y - 0.8), 1.3)
                        .fill(color(0xf3dc96))
                        .into(),
                );
            }
        }
        Ok(Node::group(nodes).layered_shadow(
            color(0x3b2414).with_alpha(0.55),
            7.0,
            Vector2::new(5.0, 6.0),
        ))
    }
}

/// Builds a pose from sparse `(bone, rotation)` channels plus a hip offset.
fn pose(hip: (f64, f64), rotations: &[(usize, f64)]) -> Pose<BONES> {
    let mut pose = Pose::default();
    pose.bones[HIP].translation = hip;
    for &(bone, rotation) in rotations {
        pose.bones[bone].rotation = rotation;
    }
    pose
}

fn rest() -> Pose<BONES> {
    pose(
        (0.0, 0.0),
        &[
            (ARM_BACK, 0.1),
            (HAND_BACK, -0.15),
            (ARM_FRONT, -0.08),
            (HAND_FRONT, -0.2),
            (THIGH_BACK, 0.05),
            (THIGH_FRONT, -0.05),
        ],
    )
}

/// Procedural gait. Hanging limbs swing forward with negative (anticlockwise)
/// rotation; knees fold backward only while their leg is in the swing phase.
fn walk(phase: f64) -> Pose<BONES> {
    let (swing, velocity) = (math::sin(phase), math::cos(phase));
    pose(
        (0.0, -3.0 * (1.0 + math::cos(2.0 * phase))),
        &[
            (TORSO, 0.06),
            (HEAD, -0.04 + 0.03 * math::sin(2.0 * phase)),
            (HAT, 0.05 * math::sin(2.0 * phase + 0.6)),
            (ARM_BACK, -0.42 * swing),
            (HAND_BACK, -0.3 - 0.25 * swing.max(0.0)),
            (ARM_FRONT, 0.42 * swing),
            (HAND_FRONT, -0.3 - 0.25 * (-swing).max(0.0)),
            (THIGH_BACK, 0.45 * swing),
            (SHIN_BACK, 0.08 + 0.75 * (-velocity).max(0.0)),
            (THIGH_FRONT, -0.45 * swing),
            (SHIN_FRONT, 0.08 + 0.75 * velocity.max(0.0)),
        ],
    )
}

fn wave_intro() -> Keyframes<Pose<BONES>> {
    let raised = pose(
        (0.0, 0.0),
        &[
            (TORSO, -0.04),
            (HEAD, 0.12),
            (ARM_BACK, 0.25),
            (HAND_BACK, -0.5),
            (ARM_FRONT, -2.25),
            (HAND_FRONT, -0.75),
            (THIGH_BACK, 0.08),
            (THIGH_FRONT, -0.08),
        ],
    );
    Keyframes::new(rest()).hold(0.15.seconds()).to(
        0.6.seconds(),
        raised,
        Easing::BackOut { overshoot: 1.6 },
    )
}

/// One jump cycle: crouch, spring up, tuck at the apex, land and recover.
fn jump_cycle() -> Keyframes<Pose<BONES>> {
    let crouch = pose(
        (0.0, 12.0),
        &[
            (TORSO, 0.24),
            (HEAD, -0.15),
            (ARM_BACK, 0.75),
            (HAND_BACK, -0.25),
            (ARM_FRONT, 0.7),
            (HAND_FRONT, -0.25),
            (THIGH_BACK, -0.7),
            (SHIN_BACK, 1.15),
            (THIGH_FRONT, -0.8),
            (SHIN_FRONT, 1.25),
        ],
    );
    let stretch = pose(
        (0.0, -6.0),
        &[
            (TORSO, -0.06),
            (HEAD, -0.12),
            (ARM_BACK, -2.85),
            (HAND_BACK, -0.1),
            (ARM_FRONT, -3.0),
            (HAND_FRONT, -0.1),
            (THIGH_BACK, 0.12),
            (SHIN_BACK, 0.2),
            (THIGH_FRONT, 0.05),
            (SHIN_FRONT, 0.15),
        ],
    );
    let tuck = pose(
        (0.0, 0.0),
        &[
            (TORSO, 0.1),
            (HEAD, 0.08),
            (ARM_BACK, -2.7),
            (HAND_BACK, -0.3),
            (ARM_FRONT, -1.9),
            (HAND_FRONT, -0.5),
            (THIGH_BACK, -1.0),
            (SHIN_BACK, 1.5),
            (THIGH_FRONT, -1.2),
            (SHIN_FRONT, 1.6),
        ],
    );
    let mut land = crouch.clone();
    land.bones[ARM_BACK].rotation = -1.1;
    land.bones[ARM_FRONT].rotation = -1.3;
    let period = |fraction: f64| (JUMP_PERIOD * fraction).seconds();
    Keyframes::new(rest())
        .to(period(0.22), crouch, Easing::QuadraticInOut)
        .to(period(0.32), stretch.clone(), Easing::CubicOut)
        .to(period(0.55), tuck, Easing::SineInOut)
        .to(period(0.74), stretch, Easing::SineInOut)
        .to(period(0.84), land, Easing::QuadraticOut)
        .to(period(1.0), rest(), Easing::BackOut { overshoot: 1.4 })
}

/// Places a skeleton-space puppet with its soles on `ground`.
fn stand(puppet: Node, x: f64, ground: f64, scale: f64) -> Node {
    puppet
        .pivot(Pivot::Point(Point::ORIGIN))
        .scale(scale)
        .translate(x, ground - LEG * scale)
}

/// Two copies of a 960-wide strip, scrolled left by `offset` and wrapped.
fn tiled(asset: &Arc<SvgAsset>, offset: f64) -> Node {
    let offset = offset.rem_euclid(960.0);
    Node::group([
        place(asset, -offset, 0.0),
        place(asset, 960.0 - offset, 0.0),
    ])
}

fn caption(text: &str, x: f64, y: f64, ink: u32) -> Node {
    Text::new(text).size(19.0).fill(color(ink)).translate(x, y)
}

fn paper_shadow(node: Node, radius: f64) -> Node {
    node.layered_shadow(
        color(0x5b3a1e).with_alpha(0.5),
        radius,
        Vector2::new(0.0, radius * 0.6),
    )
}

struct Stroll {
    puppet: Arc<Puppet>,
    sun: Arc<SvgAsset>,
    cloud: Arc<SvgAsset>,
    far: Arc<SvgAsset>,
    middle: Arc<SvgAsset>,
    near: Arc<SvgAsset>,
    meadow: Arc<SvgAsset>,
}

impl Stroll {
    fn new(puppet: Arc<Puppet>) -> motion::Result<Self> {
        Ok(Self {
            puppet,
            sun: svg(art::sun("#f08a4b", "#f6b26b"))?,
            cloud: svg(art::cloud())?,
            far: svg(art::hills("#ecc196", 300.0, 34.0, (2.0, 5.0), 0.4))?,
            middle: svg(art::hills("#d99a6c", 360.0, 30.0, (3.0, 7.0), 1.9))?,
            near: svg(art::hills("#a8b46f", 412.0, 22.0, (4.0, 9.0), 3.1))?,
            meadow: svg(art::meadow())?,
        })
    }

    /// The camera tracks Pip, so the world scrolls at stride speed while
    /// deeper paper layers lag behind for parallax.
    fn frame(&self, context: &SceneContext<'_>) -> motion::Result<Node> {
        let time = context.seconds();
        let travel = time * WALK_SPEED;
        let phase = time * TAU * STEP_RATE;
        let entrance = (1.0 - (time / 1.2).min(1.0)).powi(3);
        let puppet = self.puppet.draw(&walk(phase))?;
        let clouds = Node::group(
            (0..3)
                .map(|index| {
                    let base = f64::from(index) * 340.0 + 60.0;
                    let x = (base - travel * 0.08).rem_euclid(1140.0) - 170.0;
                    place(&self.cloud, x, 50.0 + f64::from(index % 2) * 46.0)
                })
                .collect::<Vec<_>>(),
        );
        Ok(Node::group([
            context.bounds().fill(color(0xf7e3bd)).into(),
            place(&self.sun, 640.0, 40.0)
                .pivot(Pivot::Point(Point::new(90.0, 90.0)))
                .rotate(time * 0.12),
            paper_shadow(clouds, 4.0),
            paper_shadow(tiled(&self.far, travel * 0.15), 5.0),
            paper_shadow(tiled(&self.middle, travel * 0.35), 6.0),
            paper_shadow(tiled(&self.near, travel * 0.65), 6.0),
            paper_shadow(tiled(&self.meadow, travel), 4.0),
            stand(puppet, 450.0 - 380.0 * entrance, 452.0, 1.0),
            caption("Pip Takes a Bow", 36.0, 30.0, 0x5a2e1c),
            caption("I. A little stroll", 36.0, 56.0, 0x9c5a3a).opacity(0.8),
        ]))
    }
}

struct Stage {
    puppet: Arc<Puppet>,
    curtain: Arc<SvgAsset>,
    stage: Arc<SvgAsset>,
    proscenium: Arc<SvgAsset>,
    intro: Keyframes<Pose<BONES>>,
    curtains: Spring<f64>,
}

impl Stage {
    fn frame(&self, context: &SceneContext<'_>) -> motion::Result<Node> {
        let time = context.seconds();
        let wave = ((time - 0.45) / 0.3).clamp(0.0, 1.0);
        let mut pose = self.intro.sample(context.time());
        pose.bones[HAND_FRONT].rotation += wave * 0.55 * math::sin(time * TAU * 1.7);
        pose.bones[ARM_FRONT].rotation += wave * 0.12 * math::sin(time * TAU * 1.7 + 0.8);
        pose.bones[HIP].translation.1 += -2.5 * wave * (1.0 + math::cos(time * TAU * 1.7 * 2.0));
        pose.bones[HEAD].rotation += wave * 0.05 * math::sin(time * TAU * 0.85);
        let open = self.curtains.sample(context.time()).max(0.0);
        let curtain = |x: f64, mirror: bool| {
            let node = place(&self.curtain, 0.0, 0.0);
            let node = if mirror {
                node.pivot(Pivot::Point(Point::new(160.0, 0.0)))
                    .scale_xy(-1.0, 1.0)
            } else {
                node
            };
            paper_shadow(node.translate(x, 60.0), 8.0)
        };
        let lights = Node::group(
            (0..7)
                .map(|index| {
                    let x = 180.0 + f64::from(index) * 100.0;
                    let flicker = 0.85 + 0.15 * math::sin(time * 5.0 + f64::from(index) * 1.7);
                    Node::group([
                        Node::from(
                            Circle::new(Point::new(x, 516.0), 16.0)
                                .fill(color(0xffd27a).with_alpha(0.28 * flicker)),
                        ),
                        Node::from(Circle::new(Point::new(x, 516.0), 6.0).fill(color(0xfff1c2))),
                    ])
                })
                .collect::<Vec<_>>(),
        );
        Ok(Node::group([
            context.bounds().fill(color(0x5a1f2b)).into(),
            Image::open("landscape.jpg")
                .size(580.0, 340.0)
                .fit(Fit::Cover)
                .translate(190.0, 92.0),
            Rect::new(190.0, 92.0, 770.0, 432.0)
                .fill(color(0xf2a65a).with_alpha(0.22))
                .into(),
            Rect::new(190.0, 92.0, 770.0, 432.0)
                .fill(color(0x3b1a10).with_alpha(0.12))
                .into(),
            place(&self.stage, 0.0, 0.0),
            stand(self.puppet.draw(&pose)?, 480.0, 456.0, 1.2),
            curtain(178.0 - 225.0 * open, false),
            curtain(462.0 + 225.0 * open, true),
            paper_shadow(place(&self.proscenium, 0.0, 0.0), 6.0),
            lights,
            caption("II. Hello, everyone!", 36.0, 30.0, 0xf3d9a4).opacity(0.85),
        ]))
    }
}

struct Finale {
    puppet: Arc<Puppet>,
    sun: Arc<SvgAsset>,
    far: Arc<SvgAsset>,
    near: Arc<SvgAsset>,
    meadow: Arc<SvgAsset>,
    banner: Arc<SvgAsset>,
    cycle: Keyframes<Pose<BONES>>,
    hat: Spring<f64>,
    seed: u64,
}

/// Deterministic SplitMix64-derived value in [0, 1).
fn random(seed: u64, index: u64, channel: u64) -> f64 {
    let mut value = seed
        ^ index.wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ channel.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^= value >> 31;
    (value >> 11) as f64 / (1u64 << 53) as f64
}

impl Finale {
    fn frame(&self, context: &SceneContext<'_>) -> motion::Result<Node> {
        let time = context.seconds();
        let local = time.rem_euclid(JUMP_PERIOD);
        let fraction = local / JUMP_PERIOD;
        let mut pose = self.cycle.sample(local.seconds());
        let air = ((fraction - 0.3) / 0.48).clamp(0.0, 1.0);
        pose.bones[HIP].translation.1 -= 120.0 * 4.0 * air * (1.0 - air);
        // The hat floats up during the ascent and settles on a spring after landing.
        let landing = JUMP_PERIOD * 0.78;
        let hat = if local < landing {
            -16.0 * math::sin(air * PI * 0.5)
        } else {
            -16.0 * (1.0 - self.hat.sample((local - landing).seconds()))
        };
        pose.bones[HAT].translation.1 = hat;
        pose.bones[HAT].rotation = hat * -0.012;
        let height = 4.0 * air * (1.0 - air);
        let shadow = Node::from(
            Circle::new(Point::new(0.0, 0.0), 46.0)
                .fill(color(0x3b2414).with_alpha(0.22 - 0.12 * height)),
        )
        .scale_xy(1.0 - 0.4 * height, 0.16)
        .translate(480.0, 458.0);
        let confetti = Node::group(
            (0..44u64)
                .map(|index| {
                    let speed = 45.0 + 50.0 * random(self.seed, index, 0);
                    let start = 600.0 * random(self.seed, index, 1);
                    let x = 960.0 * random(self.seed, index, 2)
                        + 22.0 * math::sin(time * 1.6 + index as f64);
                    let y = (start + speed * time).rem_euclid(600.0) - 40.0;
                    let spin = time * (1.5 + 2.0 * random(self.seed, index, 3)) + index as f64;
                    let hue =
                        [0xd9573b, 0xe9b237, 0x3c7d6e, 0xe98b9a, 0xf6e6c4][(index % 5) as usize];
                    Node::from(Rect::new(-5.0, -7.0, 5.0, 7.0).fill(color(hue)))
                        .scale_xy(math::cos(spin * 1.7), 1.0)
                        .rotate(spin)
                        .translate(x, y)
                })
                .collect::<Vec<_>>(),
        );
        let pop = 1.0 - math::exp(-(time * 6.0)) * math::cos(time * 14.0);
        let banner = Node::group([
            place(&self.banner, 0.0, 0.0),
            Text::new("Bravo, Pip!")
                .size(30.0)
                .fill(color(0xfff4dc))
                .pivot(Pivot::Center)
                .at(Point::new(210.0, 37.0)),
        ])
        .pivot(Pivot::Point(Point::new(210.0, 45.0)))
        .scale(pop.max(0.0))
        .translate(270.0, 26.0);
        Ok(Node::group([
            context.bounds().fill(color(0xf4c58a)).into(),
            Rect::new(0.0, 0.0, 960.0, 180.0)
                .fill(color(0xef9f6a).with_alpha(0.55))
                .into(),
            place(&self.sun, 390.0, 170.0)
                .pivot(Pivot::Point(Point::new(90.0, 90.0)))
                .rotate(-time * 0.2),
            paper_shadow(place(&self.far, 0.0, 0.0), 5.0),
            paper_shadow(place(&self.near, 0.0, 0.0), 6.0),
            paper_shadow(place(&self.meadow, 0.0, 0.0), 4.0),
            shadow,
            paper_shadow(banner, 5.0),
            stand(self.puppet.draw(&pose)?, 480.0, 458.0, 1.05),
            confetti,
            caption("III. Hooray!", 36.0, 500.0, 0x5a2e1c).opacity(0.8),
        ]))
    }
}

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    let puppet = Arc::new(Puppet::new()?);
    let stroll = Arc::new(Stroll::new(puppet.clone())?);
    let stage = Arc::new(Stage {
        puppet: puppet.clone(),
        curtain: svg(art::curtain())?,
        stage: svg(art::stage())?,
        proscenium: svg(art::proscenium())?,
        intro: wave_intro(),
        curtains: Spring::new(0.0, 1.0).stiffness(40.0).damping(9.0),
    });
    let finale = Arc::new(Finale {
        puppet,
        sun: svg(art::sun("#f26b3a", "#f8a35c"))?,
        far: svg(art::hills("#c0604a", 350.0, 40.0, (2.0, 3.0), 2.2))?,
        near: svg(art::hills("#8a4040", 410.0, 26.0, (3.0, 5.0), 0.7))?,
        meadow: svg(art::meadow())?,
        banner: svg(art::banner())?,
        cycle: jump_cycle(),
        hat: Spring::new(0.0, 1.0).stiffness(260.0).damping(9.0),
        seed: parameters.seed,
    });
    let duration = parameters.seconds.seconds();
    let ticks = duration.ticks();
    let first = Time::from_ticks(ticks * 3 / 8);
    let second = Time::from_ticks(ticks * 7 / 20);
    let turn = Time::from_ticks(ticks / 10);
    let third = duration - first - second + turn + turn;
    Ok(Timeline::new(Size::new(960.0, 540.0), 30)
        .track(|track| {
            track
                .scene(first, move |context: &SceneContext<'_>| {
                    stroll.frame(context)
                })
                .transition(
                    Iris {
                        shape: IrisShape::Circle,
                    },
                    turn,
                )
                .scene(second, move |context: &SceneContext<'_>| {
                    stage.frame(context)
                })
                .transition(
                    Push {
                        direction: Direction::Up,
                    },
                    turn,
                )
                .scene(third, move |context: &SceneContext<'_>| {
                    finale.frame(context)
                })
        })
        .duration(duration))
}
