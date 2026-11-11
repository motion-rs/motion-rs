# 03 Animation

## What This Chapter Covers

Replace manual per-frame updates with values sampled at explicit times, then combine movement, holds, easing and continuous signals.
Basic animation needs no optional features. Content-level vector `Morph` requires `vector`; formula morphing requires `formula`.

## Minimal Example

```rust,no_run
use motion::prelude::*;

fn main() -> motion::Result {
    let movement = Tween::new(60.0, 420.0, 2.seconds())
        .easing(Easing::QuadraticOut)
        .ping_pong();
    let opacity = Keyframes::new(0.0_f64)
        .to(500.milliseconds(), 1.0, Easing::Linear)
        .hold(3.seconds())
        .to(4.seconds(), 0.0, Easing::Linear);
    let timeline = Timeline::new(Size::new(480.0, 270.0), 30).track(|track| {
        track.scene(4.seconds(), move |context| -> motion::Result<_> {
            Ok(Circle::new(Point::new(context.sample(&movement), 135.0), 24.0)?
                .fill(Color::rgb(20, 150, 125))
                .opacity(context.sample(&opacity)))
        })
    }).build()?;
    render_still(&timeline, 1.seconds().after_origin(), &RenderOptions::default())?
        .save_png("tutorial-animation.png")
}
```

Request times in any order: the animation samples the same time function. `ping_pong()` extends a two-second forward motion into a four-second round trip; the final keyframe is also at four seconds.

## Animation and Interpolate

`Animation<Value>` returns a value for an explicit `Time`. A finite animation's `duration()` is `Some(Duration)`; an infinite signal returns `None`.
`Interpolate` defines how two values interpolate using f64 progress. It does not automatically clamp every progress value to `[0, 1]`; springs and easing may intentionally overshoot.

| Type | Suitable Use |
| --- | --- |
| `Constant` | A time-invariant animation value |
| `Tween` | Interpolation between two endpoints |
| `Keyframes` | Absolute local time points, holds, piecewise easing |
| `Spring` | Elastic transitions with physical parameters and a finite settling threshold |
| `Oscillator` | Continuous sine, triangle, square or sawtooth signals |
| `Noise` | Seeded smooth continuous noise |
| `FollowPath` | Path position, tangent and angle sampled by arc length |

`context.sample(&animation)` uses scene-local time. For another clock, call `animation.sample(explicit_time)` rather than implicitly capturing the previous render's time.

## Easing and Keyframes

`Easing::Linear` gives constant speed; `QuadraticOut` gradually decelerates. Steps, CubicBezier and Custom are also available, but parameters and output must remain finite. Bezier solves for X before reading Y; time is not used directly as the curve parameter.

Keyframe `.to(time, value, easing)` takes an absolute local time, while `.hold(duration)` adds a relative hold. The example reaches 1 at 0.5 seconds, holds until 3.5 seconds and reaches 0 at four seconds.

The destination keyframe holds the segment's easing. Duplicate or decreasing target times are errors, not automatically sorted. Sampling exactly at a keyframe returns its stored value.

Built-in easing pins its endpoints. Custom easing clamps only its input and retains
the callback's endpoint values; interior values may overshoot. Nonfinite output
fails explicitly. Checked input does not sandbox a custom callback or catch its
panic.

## Composing Time

| Method | Meaning and Constraints |
| --- | --- |
| `map` | Map sampled values to another value type |
| `zip` | Sample two animations together |
| `delay` | Add a leading delay; negative delays are invalid |
| `speed` | Change speed with a valid factor |
| `then` | Sequence finite animations |
| `reverse` | Sample a finite animation backward |
| `repeat` | Total play count, not the number of additional repetitions |
| `ping_pong` | Forward then backward, independent of the previous frame |
| `boxed` | Erase the concrete animation type for dynamic composition |

Operations requiring a finite duration cannot apply directly to infinite signals. A zero-duration Tween still has explicit endpoint semantics before and after its boundary; it is not a deleted animation.

Repeat counts total plays; zero holds the initial value. Negative delays, invalid
speed factors and duration overflow are rejected. For dynamic input, use checked
Tween construction/easing and `Easing::try_validate` or `try_apply` rather than
assuming every convenience method returns a Result.

`remap` and `piecewise` support Clamp, Extend and Wrap. Input knots must be finite
and strictly increasing; malformed mappings panic instead of sorting the knots.
`FollowPath` returns position, tangent and angle using the existing arc-length
geometry. Noise shares its deterministic primitive with particle NoiseDrift.

## Springs and Continuous Signals

```rust,no_run
use motion::prelude::*;

fn main() {
    let spring = Spring::new(0.0_f64, 100.0)
        .mass(1.0)
        .stiffness(120.0)
        .damping(18.0)
        .settling_threshold(0.0001);
    let pulse = Oscillator::new(Waveform::Sine, 1.0)
        .amplitude(0.25)
        .offset(0.75);
    let drift = Noise::new(42).frequency(0.5).octaves(3);
    let time = 500.milliseconds();
    let _ = (spring.sample(time), pulse.sample(time), drift.sample(time));
}
```

Spring mass, stiffness and damping are positive. A conservative settling envelope determines the finite duration. `settling_threshold` is a relative displacement-envelope threshold, not a pixel error. `initial_velocity` is measured in endpoint distances per second, not pixels per second.

The conservative duration is rounded to ticks; it is not inferred from a previous
frame's displacement.

Use an explicit infinite signal for undamped perpetual oscillation rather than asking a finite Spring to have no end time.

## Where Morph Belongs

Geometric `MorphPath` and content-level `Morph` are different abstractions. The former interpolates paths; the latter handles vector content. Formula `FormulaMorph` also accounts for symbol selection and structural correspondence.

First decide which objects correspond, how fill rules are preserved and how bounds are measured. Two path inputs do not necessarily have compatible vertex order, holes or topology.

Learn time composition here. [06 Content](06-content.md) places content-level Morph in context; [08 Typst and Formulas](08-formula-typst.md) covers formula morphing.

## Common Pitfalls

- Accumulating coordinates in callbacks makes backward seeks, dropped frames and concurrent rendering change the result.
- Treating `.to` time as a segment duration creates duplicate or decreasing keyframes.
- Clamping all interpolation removes intentional overshoot.
- Convenience constructors do not all return `Result`. Some invalid parameters panic; use checked interfaces for dynamic input.
- Infinite animations do not automatically define an output end time. Timelines still need explicit scene or film durations.

## Next Steps

Next chapter: [04 Scenes and Timeline](04-scenes-and-timeline.md).
Contracts: [animation guide](../design/core.md#stateless-animation), [Spring implementation](../../crates/motion-core/src/animation/spring.rs).
Film example: [Room to Move](../../gallery/src/demos/type_in_motion.rs), whose cover graphemes settle on springs.