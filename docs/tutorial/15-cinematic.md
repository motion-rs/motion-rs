# Cinematic Output

All cinematic processing is opt-in. `RenderOptions::default()` preserves the
original output path. Floating RGBA16F exports remain scene-linear; display
transforms affect SDR RGBA8 and YUV only. Transparency is unchanged.

```rust,no_run
use motion::prelude::*;
let display = DisplayTransform {
    tone_map: ToneMap::AgX,
    reference_white: Some(1.0),
    grain: Some(FilmGrain {
        seed: 42, amount: 0.012, size: 1.5,
        luma_response: 1.0, chroma: 0.1,
    }),
    ..DisplayTransform::default()
};
let options = RenderOptions::default().display(display);
```

The order is exposure, CDL, look LUT, tone curve, saturation, display LUT,
output transfer, grain, Bayer dither, quantization. Reference-white normalization
uses `curve(rgb*k)/curve(k)`. Canonical Khronos PBR Neutral includes a dark offset:
it is not exactly identity below 0.76 and does not preserve SDR white at 1.0.
ACES uses Hill's RRT/ODT fit. AgX is linearized after its gamma-2.2 polynomial,
before the output OETF. Hable supports an explicit normalization white.

## Spatial Effects

`Filter::Bloom` uses soft extraction, normalized octave weights and bounded
Gaussian support, computed on a 2x downsampling pyramid. `Filter::halation` and `Filter::anamorphic` are transparent
combinations of existing primitives. `FilterOperation::Transform` inverse maps
pixel centers with transparent borders. `BlurXY` has separate horizontal and
vertical sigmas; local sigmas use the parent's maximum singular scale, and its
axes are device-aligned even when the parent rotates or shears.

`Filter::Optics` supports radial lens distortion/chromatic dispersion, physical
cos-fourth vignette, circular/polygonal bokeh with 8 to 64 radius-scaled taps, bounded radial blur,
Sobel/Lambert/Blinn-Phong relighting, mirrored lens-flare ghosts with annular
weighting and `Optics::Anamorphic` horizontal streaks. Bokeh is an aperture approximation, not a depth-buffer renderer.
Lens/radial/affine/flare resampling requests full-frame CPU source support;
blur and relighting retain bounded band halos. Source light outside the recorded
canvas cannot contribute to a frame. These operations are charged to raster
work/storage allowances and may be rejected before rendering.

`Optics::Anamorphic { threshold, length, stretch, tint, intensity }` extracts
highlights above `threshold`, blurs them with a horizontal sigma `length` and
vertical sigma `length / stretch` (`stretch` in 1..=64, device-axis aligned), tints
the streak and adds it back with Plus. It shares the Glow-style CPU and GPU
pipeline, grows its output bounds by `ceil(3 sigma)` per axis and is charged to the
raster work and storage allowances like the underlying blur passes.
`Filter::anamorphic` remains the fixed-threshold primitive composition.

`DepthOfField` captures explicit `DepthPlane`s in far-to-near order. Distances
and focal length share scene units. The thin-lens circle of confusion is exactly
zero at focus and capped by `max_radius`; optional pinhole perspective scales
both geometry and camera translation. Each out-of-focus plane is wrapped in a
`Optics::Bokeh` of that radius, so depth of field is rendered by the ordinary CPU/GPU
optics path; plane coordinates are camera-centred. `NodeModifiers::layered_shadow`
composes three alpha silhouettes behind one source. `Elevation::shadow(height)`
with `NodeModifiers::elevated` builds a contact, key and ambient shadow whose
sigmas and offsets grow with height and whose opacity fades; `direction` and
`color` are editable, and zero height adds nothing. `gate_weave` takes an explicit seed
and frame number, with no implicit clock or random state.

## Grading And LUTs

`Cdl::temperature_tint` composes simple channel gains, not a spectral white
balance. `ColorWheels` has smooth shadow/midtone/highlight offsets around 0.18;
`bake(size, domain_max, cdl)` returns an original immutable LUT usable in both
`DisplayTransform::look_lut` and `Filter::Lut`.

`motion_core::Lut3d::from_cube` accepts bounded 2..=64 3D cubes, red-fastest
storage and explicit domains, optionally preceded by a 2..=65536 sample 1D
shaper. `Lut1d::from_cube` reads standalone 1D files. Filesystem loading is
explicit through `load_lut` and `load_shaper` in `motion_content` (also exported
by the facade's `vector` feature). Unsupported directives, duplicate headers,
nonfinite values and mismatched sample counts are rejected; files are capped
at 8 MiB.

Each shaper row contains independent R/G/B samples, interpolated linearly.
The order is input encoding, shaper, tetrahedral cube, output decoding. Neither
the shaper nor cube is an implicit output transfer function. Out-of-domain inputs
clamp, but shaper outputs may be negative or HDR. In combined files DOMAIN_MIN/MAX
describe the shaper input; the cube defaults to 0..1. `LUT_1D_INPUT_RANGE` and
`LUT_3D_INPUT_RANGE` provide explicit scalar ranges. A DOMAIN and an INPUT_RANGE
for the same stage are ambiguous and rejected. Programmatic construction can
specify each stage's per-channel domain independently:

```rust
use motion::prelude::*;
let shaper = Lut1d::new(
    [0.0; 3], [16.0; 3],
    vec![[0.0; 3], [0.8; 3], [1.0; 3]],
)?;
let look = ColorWheels::default().bake(3, 1.0, Cdl::default())?
    .with_shaper(shaper);
assert!((look.sample([8.0; 3])[0] - 0.8).abs() < 1e-6);
# Ok::<(), motion::ValidationError>(())
```

CPU/GPU use the same interpolation and Linear/sRGB/Log encoding contract.
GPU LUT data remains in storage buffers; both subtree filters and look/display
LUTs include the shaper. External OCIO/FFmpeg reference acceptance is separate.

Generate an original look without third-party LUT licensing:

```sh
node tools/generate-lut.cjs teal-orange look.cube 33
```

## Procedural Atmosphere

`motion::atmosphere::Atmosphere` (feature `vector`) offers editable `fog(seed)`,
`dust(seed)` and `light_leak(seed)` recipes. They generate immutable linear RGBA16F
image assets through the ordinary image pipeline, without requiring image codecs.
Tint, opacity, HDR intensity, pixel scale, velocity and fractal parameters are
explicit. Fog has a vertical density gradient; dust has deterministic sparse
soft motes; light leaks have a normalized elliptical envelope.

```rust
use motion::prelude::*;
use motion::atmosphere::Atmosphere;
use motion::render::RasterLimits;
let mut fog = Atmosphere::fog(42);
fog.scale = 80.0;
let asset = fog.render(PixelSize::new(64, 48), 1.seconds().after_origin(), RasterLimits::sandbox())?;
let overlay = Node::from(Image::from_asset(std::sync::Arc::new(asset)))
    .blend_mode(BlendMode::Screen);
# let _ = overlay;
# Ok::<(), motion::Error>(())
```

`Atmosphere::overlay(size, time, limits)` returns the same image as a bilinear
node using `Atmosphere::blend_mode()` (Screen for fog, Plus for dust and light
leaks). Use SourceOver for an obscuring fog layer or Screen/Plus for bright atmospheric
overlays. Rendering at a different explicit `Time` advects a continuous 2D field;
it does not integrate particles or solve three-dimensional volume scattering.
Use depth-plane-specific fog recipes for authored 2.5D atmospheric perspective.
`render_region(canvas, origin, region, time, limits)` retains global coordinates
and full-canvas envelopes: base texels match a full render exactly. Independently
generated mipmaps are regional, not a seamless full-canvas mip chain.

`motion::noise` supplies `hash32/hash2/hash3`, `value2/value2_d`,
`perlin2/perlin2_d`, `curl2`, `perlin3`, `simplex2`, `worley2`, `domain_warp`,
`turbulence2`, `ridged2` and amplitude-normalized `fbm`. Spatial coordinates and
output use f64 to preserve existing audio/brush callers. `Fractal` admits 1..=8 octaves,
lacunarity 1..=8 and gain 0..=1. Fractal coordinates must fit the signed-32-bit
lattice at every octave. Atmosphere validates parameters and admits pixels,
temporary float storage, mip storage and octave work before allocating output.
GPU draws the generated image. The renderer now assembles a shared `noise.wgsl`
with matching integer hashes, spatial algorithms and f32 interpolation (N1/N4).
Native readback checks exact integer hashes and floating results within `2e-6`
on the fixture's representable coordinates; large-coordinate f32 precision is
not equivalent to CPU f64. WGSL invalid primitive/fBm samples return zero and
invalid domain-warp recipes return the input point; Rust fractal/warp APIs
instead report `ValidationError`. These guards do not replace host admission.

```rust
use motion::noise::{DomainWarp, Fractal, domain_warp, fbm, simplex2, worley2};
let point = domain_warp([0.37, -1.23], DomainWarp { seed: 42, ..DomainWarp::default() })?;
let cloud = fbm(|point| simplex2(point, 19), point, Fractal::default())?;
let (nearest, second) = worley2(point, 19);
let cell_edge = second - nearest;
assert!(cloud.is_finite() && cell_edge >= 0.0);
# Ok::<(), motion::Error>(())
```

Domain warp returns displaced coordinates and does not automatically alter the
built-in atmosphere recipes or implement physical caustics/volume scattering.
Display `FilmGrain` uses shared `noise::film_grain`, which draws its Gaussian
cells from the same N0 lattice hash as the other noise functions. Its WGSL seed
is the XOR of the low/high u32 seed words. The [particle chapter](07-particles.md)
covers the 2D curl and the evolving smoke sprite (N3). SVG-standard turbulence
and stitchTiles (N2) remain separate, unimplemented work.

## Shutter And Delivery

`ShutterSampling` adds explicit seeded stratified jitter, Box/Triangle/Cosine
exposure and adaptive 8..=64 sample selection. It can capture a selected subtree
with `.capture(blur, context, output_size, build)`, or be installed on
`RenderOptions::shutter_sampling`. Every whole-frame capture, including the
default policy, rejects a nested subtree capture.
Adaptive selection compares endpoint device-space geometry; it cannot detect
periodic motion returning to the same endpoint and falls back to 64 for changing
structure. Such content should use an explicit fixed sample count.

```rust,no_run
use motion::prelude::*;
let look = Look::film(42);
let options = ExportOptions::new("film.mp4")
    .codec(Codec::H265 {
        quality: 18,
        preset: EncoderPreset::Slow,
        bit_depth: BitDepth::Ten,
        params: None,
    })
    .render(|options| look.render_options(options));
```

Build each frame with `look.scene(context, output, build)`: it captures the shutter,
then applies the spatial filters once to the accumulated frame (not once per shutter
sample). `render_options` carries only the display transform. Use a 24fps timeline for film cadence. `Look::clean()` is neutral
display-only, `cinematic()` adds restrained bloom/lens/shutter, and `film(seed)`
adds encoded grain and gate drift. Recipes remain editable pure data.

Main10 is restricted to H.265. The native path writes little-endian 16-bit plane
bytes using FFmpeg byte strides. GPU rendering packs 10-bit Y/U/V (the GPU export pipeline honours
`Codec::H265 { bit_depth, .. }`, and agrees with the CPU export within one code value)
directly from
the final linear texture when width is a multiple of 8 and height is even.
Each sample occupies the low 10 bits of a little-endian u16, not a P010 high-bit
sample. Readback contains 3 bytes per pixel; FFmpeg still receives CPU plane
bytes, so this is not zero-copy GPU encoding. Other valid even widths retain
CPU output conversion after GPU linear readback. Metadata is BT.709 primaries/matrix
and limited range; sRGB transfer is tagged IEC 61966-2-1. `OutputTransfer::Gamma24`
is encoded as a pure 2.4 power for YUV and tagged BT.709, the conventional SDR
delivery tag (not an assertion that BT.709 OETF equals gamma 2.4). Transformed
RGBA8 also follows the selected transfer; the identity path remains sRGB.
`params` (the x265 parameter string) is explicit, size-bounded and exists only on `Codec::H265`.

Run the comparison with
`cargo run -p motion-rs --all-features --example cinematic -- clean` and
`cargo run -p motion-rs --all-features --example cinematic -- film`.
Both create Main10 videos with the
same deterministic moving HDR source. P6 quality measurements are in
[the quality report](../quality-report.md). Performance execution is separate.

Append `--still` to either example command to create a single-frame PNG under
`docs/assets/`, without replacing an existing file. The archived comparison is
[clean](../assets/cinematic-clean.png) and [film](../assets/cinematic-film.png).
