# 07 Particles

## What This Chapter Covers

Create seekable particle content and distinguish trajectories, simulation, appearance and instance rendering. This chapter requires `particles`; explicit GPU rendering also requires `gpu`.

## Minimal Example

```rust,no_run
use motion::prelude::*;
use motion::particles::{Ballistic, ParticleStyle, ParticleTrajectory};

fn main() -> motion::Result {
    let firework = ParticleTrajectory::new(
        Ballistic::burst(Point::new(240.0, 80.0), 200)
            .gravity(Vector2::new(0.0, 180.0))
            .drag(0.8),
    )
    .style(ParticleStyle::GlowDots { radius: 2.0 });
    let timeline = Timeline::new(Size::new(480.0, 270.0), 30)
        .track(|track| track.push(firework.with_duration(3.seconds())));
    render_still(&timeline, 1.seconds(), &RenderOptions::default())?
        .save_png("tutorial-particles.png")
}
```

Start with a few particles to understand their lifetimes, then increase density. Export uses the same timeline without a hand-written per-frame update loop.

## Four Distinct Layers

| Layer | Object | Question |
| --- | --- | --- |
| State | `Particle` | What are the position, appearance and finite state at a given age? |
| Analytic trajectory | `TrajectoryModel` / `ParticleTrajectory` | Can the complete state be evaluated directly at any time? |
| Fixed-step simulation | `ParticleSimulation` | How do ordered emitters and modifiers evolve state? |
| Presentation | `ParticleStyle` / `InstanceBatch` | How is evaluated state drawn as dots, glows, sprites or segments? |

Trajectories are not arbitrarily stackable forces. `Ballistic` evaluates gravity and drag together; `Converge` targets a point set; `Orbit` uses circular phase; `NoiseDrift` returns stable smooth displacement and velocity.

## Random Access and Simulation

Analytic trajectories evaluate stable IDs, explicit birth times, ages and model parameters. A custom `TrajectoryModel` should follow the same contract, not mutate global state in `state` based on its previous call.

Simulation requires an explicit seed, finite duration, emitters, ordered modifiers and `SimulationLimits`. Its fixed step is `1/240` second. Arbitrary queries use the fixed boundary containing that time, not a floating playback cursor.

Store prepared simulations outside callbacks or cache them with keys covering every configuration value and the duration. Backward and repeated seeks reproduce the same state; memory admission also affects cached snapshot spacing.

## Emitter and Modifier Order

| Category | Examples |
| --- | --- |
| Emitters | Point, Rect, Disk, Path, Surface, Burst |
| Forces/Modifiers | Gravity, Wind, Drag, CurlNoise, Attractor, Seek, Vortex |
| Boundaries | BounceBox |
| Appearance over lifetime | SizeOverLifetime, ColorOverLifetime |

Order is visible behavior. Handling collisions after changing position differs from handling them first. Lifetimes and color stops need explicit valid ranges; they are not automatically normalized or sorted.

## The Instance Rendering Boundary

Trajectories, noise and custom emitters/modifiers evaluate in Rust. The GPU receives evaluated positions, radii, colors, rotations, types and segment endpoints; it does not interpret a separate GPU trajectory program.

`Canvas::instances` records a shared `InstanceBatch` without expanding it into many ordinary paths. The extension-level `InstanceUpload::new` accepts typed Disc, Glow, Sprite and Segment records (source-over; `new_blended` selects another `InstanceBlend`). Upload headers, 48-byte primitives and optional shared sprite texels are checked and budgeted.

CPU-only batches have no GPU upload by default. Explicit GPU requests reject batches without a GPU representation rather than silently switching to CPU.

## Large Batches Still Have Costs

Supported CPU and GPU paths use ordered device tiles to narrow per-pixel candidate sets. They preserve original instance order; unordered atomic appends must not change transparent overlap order.

Small batches may use a full loop to avoid fixed overhead. CPU batches with unreliable bounds can fall back to the original path. GPU storage, indices, pixels and actual tile-instance work can still fail admission.

`MAX_INSTANCE_COUNT` is a protocol ceiling of `1048576`, not a recommended particle count. Simulation and emitters retain separate limits. Plexus uses a stable spatial hash for up to 65,536 states and explicitly rejects excessive pair/work counts.

## Determinism and Performance

Fixed seeds address only random input. Ticks, birth times, modifier order, materials, resource environments and backend replay must also be explicit.

Particle count, tile indices, transparent overlap area and trail length all affect work. Reducing counts or increasing budgets changes the scene or admission conditions; it does not prove 30 fps.

## Common Pitfalls

- Rebuilding a complete simulation in a scene callback repeats preparation costs and can use an incorrect cache identity.
- Replacing model age or local time with global seconds changes lifetimes when a clip moves.
- GPU support does not make every custom batch uploadable; an explicit upload representation is required.
- Limiting instance count does not eliminate pixel and overlap costs. A few huge glows can still be expensive.
- Reordering transparent instances or truncating indices changes the image and is not a lossless optimization.

## Extended Styles

Old Dots/GlowDots/Sprites/Trails literals retain their original twelve-word uploads. Optional builders select sixteen-word records without changing Particle:

```rust
use motion::prelude::*;
use motion::particles::{ParticleCurve, ParticleBlend, ParticleShape, ParticleStyle, TrailOptions};
let trails = ParticleStyle::Trails { length: 120.milliseconds(), width: 2.0 }
    .trails(TrailOptions { segments: 16, taper: Some(ParticleCurve::linear(0.0, 1.0)),
        fade: Some(ParticleCurve::linear(0.0, 1.0)), smooth: true, ribbon: true })
    .blend(ParticleBlend::Plus);
let confetti = ParticleStyle::Shape { radius: 2.0, shape: ParticleShape::Quad { aspect: 1.5 } }.flip();
```

SourceOver, Plus, Screen and Multiply are batch policies. Taper/fade follow oldest-to-newest trail progress; samples are shared across particles. Smoothing subdivides at most four times per interval. Polygon (3..=32 sides), Ring, Capsule, Soft and Quad require no texture. stretch_by_speed(scale, maximum) explicitly bounds velocity elongation.

SpriteSheet supports lifetime, fixed-fps, seeded random and custom[0] selection. SpriteAtlas packs 1..=64 equal-sized sources under a byte budget; bilinear sampling clamps inside the selected cell. SpriteAtlas::new_padded accepts mixed sizes by centring each image on a transparent, uniform cell; `.random(seed)` gives each particle a stable texture and `.style(size, align)` returns a ready Sprites style carrying the sheet (multi-texture sprites without extra draw calls). ProceduralSprite generates SoftCircle, Smoke, Star, Ring and FireCore with an explicit seed.

Bokeh uses custom[1] depth, stable far-to-near ordering, optional parallax and bounded CoC. At focus its radius is unchanged; RGB receives highlight/CoC-squared compensation while alpha retains coverage. This is a 2.5D aperture approximation, not a depth buffer. IntensityOverLifetime multiplies HDR RGB without multiplying alpha or accumulating per-step intensity. ParticleCurve/ColorCurve have explicit times and easing; invalid overshoot is an error. Temperature maps 1000..=40000 K into linear RGB; ColorField samples position or speed.

## Emission and Contact

```rust
use motion::prelude::*;
use motion::particles::{BurstEmitter, CollisionResponse, Inherit, ParticleEventKind,
    ParticleSimulation, SegmentCollider, SubEmitter};
use std::sync::Arc;
let simulation = ParticleSimulation::new(7).duration(1.seconds())
    .emitter(BurstEmitter::new(Time::ZERO, 4, Point::new(20.0, 5.0)))
    .modifier(SegmentCollider { start: Point::new(0.0, 40.0), end: Point::new(80.0, 40.0),
        response: CollisionResponse { restitution: 0.5, friction: 0.2, kill_on_hit: true } })
    .sub_emitter(SubEmitter { on: ParticleEventKind::Collision,
        emitter: Arc::new(BurstEmitter::new(Time::ZERO, 2, Point::ORIGIN).lifetime(100.milliseconds())),
        inherit: Inherit::Position, probability: 1.0 });
assert_eq!(simulation.sample(100.milliseconds())?, simulation.sample(100.milliseconds())?);
# Ok::<(), motion::Error>(())
```

CircleCollider, SegmentCollider and PolygonCollider use swept contacts. SignedDistanceGrid prepares an explicitly resolved grid with immutable exact-boundary guards for thin geometry; cached uses the existing SceneCache. SdfCollider requires speed*step strictly smaller than the smallest cell. Resolution, preparation work and bytes have explicit limits, with no downsampling fallback. Physical contact radius is particle.size, independent of presentation radius.

Events are sorted by ID, then Birth/Collision/AgeReached/Death. Sub-emitter callbacks receive local step zero/time zero; inheritance translates local emission and optionally adds velocity or multiplies premultiplied color. Depth, total births and per-boundary events have independent limits. accepts_event can filter stages; recursive rules must terminate or fail a budget. `events` is empty unless the simulation uses the replay engine, which prewarm, `loop_period`, time remapping, reverse queries, more than one worker, group modifiers, sub-emitters, non-default birth/event/sub-depth limits or a modifier that resets color or charges more than one work unit selects.

SurfaceEmitter.inherit_color(true) preserves exact geometric points and area density. VectorSurface.sample_colored_points retains ordered solid/gradient fills; alpha is not density. Pattern fills explicitly reject color inheritance. ImageEmitter prepares an alpha/luminance CDF and inherits premultiplied pixel color; zero-weight images fail. SurfaceEmitter.from_text captures existing shaped glyph outlines (text feature); bitmap-only text is not a filled outline surface. Every emitter also accepts `.paint(Paint, bounds)` (solid or gradient) to colour births by the paint at the emission point without altering positions or random draws. PathEmitter.along_path samples animated normalized arc length and tangent launch velocity.

FlowField reads a seeded curl or image RG vectors. RadialForce supports radius/falloff. Boids, Spring (open chain or closed loop) and ParticleCollisions are opt-in population operators before independent modifiers, with stable traversal and logical work bounds.

## Shared Curl And Evolving Smoke

`CurlNoise { frequency, strength, seed, phase_speed }` uses the analytic curl
`[d potential/dy, -d potential/dx]` of a shared two-dimensional Perlin potential.
`FlowField::curl(seed, frequency, strength)` selects the same field. Both have an
editable `phase_speed`; time advects the potential, with no mutable random cursor. Frequency must be positive
and finite, and sampled coordinates must fit the signed-32-bit lattice.

```rust
use motion::prelude::*;
use motion::particles::{BurstEmitter, CurlNoise, FlowField, ParticleSimulation, ProceduralSprite};
use motion::noise::Fractal;
use motion::render::RasterLimits;
let simulation = ParticleSimulation::new(7).duration(200.milliseconds())
    .emitter(BurstEmitter::new(Time::ZERO, 16, Point::new(4.0, 4.0)).speed(2.0))
    .modifier(CurlNoise { frequency: 0.13, strength: 2.0, seed: 42, phase_speed: 1.0 })
    .modifier(FlowField::curl(19, 0.17, 0.5));
assert_eq!(simulation.sample(150.milliseconds())?, simulation.sample(150.milliseconds())?);
let material = ProceduralSprite::Smoke {
    scale: 3.0, evolution: 0.5, fractal: Fractal::default(),
};
let smoke = material.generate_at(32, 42, 250.milliseconds(), RasterLimits::sandbox())?;
# let _ = smoke;
# Ok::<(), motion::Error>(())
```

Each curl modifier charges 96 work units per update under `SimulationLimits`;
insufficient work allowance fails rather than dropping samples. Worker count and
seek order do not change the resulting state.

`ProceduralSprite::Smoke` samples spatial fBm over `perlin3([x, y, time * evolution], seed)`;
spatial octaves do not multiply the time coordinate. `scale` is 0.25..=64,
`Fractal` has at most eight octaves, and `generate_at(size, seed, time, limits)`
admits output pixels, floating temporary/mip storage and octave work before
allocation. Size is 2..=512. Each call returns an immutable image: use it as a
`ParticleStyle::Sprites` material or precompute an explicit `SpriteAtlas` for
flipbook playback. Changing simulation time alone does not regenerate a sprite.

The Smoke and Embers presets use the same curl and smoke material. Particles and sprite generation still execute in Rust; the GPU renders the
generated images and evaluated instance batches, not a GPU particle simulation.

## Time and Workers

prewarm prepares a lead-in before visible zero; authored bursts at zero occur during it. loop_period uses half-period-offset phases with cosine crossfades; both populations share max_particles and query work. Fixed-step states stay quantized, so seamless means vanishing wrap contribution, not continuous integration.

time_remap(scale, offset, allow_reverse) maps queries before replay. Negative scale requires explicit opt-in; reverse queries revive historical particles. Outside the visible half-open clip is empty. workers(1..=64) parallelizes independent updates with ordered events (populations under 128 stay serial; results are byte-identical for every worker count). Neighbour searches (Plexus, Boids, contacts) share the bounded spatial hash `NeighborIndex`, which chunks large populations across threads and concatenates in index order. Colliders treat `Particle::size` as the collision radius (see `collision.rs`). Emitters and modifiers draw from counter-derived Random::for_particle streams, independent of traversal and worker count. Floating simulation stays on CPU. Default max_particles remains 16,384 for compatibility; explicit limits can raise it to 65,536.

## Presets

```rust
use motion::prelude::*;
use motion::particles::presets::{Preset, PresetOptions};
let mut options = PresetOptions::new(7, 2.seconds(), Rect::new(0.0, 0.0, 320.0, 180.0));
options.density = 0.5;
let sparks = Preset::Sparks.simulation(options)?;
let customized = sparks.modifier(motion::particles::Drag::new(0.3));
# Ok::<(), motion::Error>(())
```

Preset::ALL includes Sparks, Embers, Dust, Motes, Snow, Rain, Smoke, Fire, Confetti, Fireworks, Fireflies, LogoDissolve and LogoAssemble. Every recipe returns a lazy ParticleSimulation builder; logo recipes require options.logo. Studio's studio-gated particle_presets registers the same recipes for preview/export. The gallery `particle_comparisons` generator writes every Bloom off/on x tone-map None/AgX combination without benchmarking:

```sh
cargo run -p motion-gallery --bin particle_comparisons -- /tmp/motion-particle-comparisons
```

## Next Chapter

Next chapter: [08 Typst and Formulas](08-formula-typst.md). Particle-only films can continue directly to [09 Rendering](09-rendering.md).
See the [particle contracts](../design/particles.md) and [trajectory implementation](../../crates/motion-content/src/particles/trajectory.rs).
Film: [Signal Garden](../../gallery/src/demos/signal_garden.rs).
