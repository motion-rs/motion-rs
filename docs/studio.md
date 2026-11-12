# Motion Studio

Motion Studio is an optional egui workbench for compiled Rust scenes. It requires a
native desktop even when scene rendering selects CPU. Pass one StudioScene to
`motion::studio::run(scene)` or select one Project timeline. There is no scene list
or in-window scene switching; editing Rust source still requires rebuilding. A
declarative document is the exception: see [hot reload](#hot-reload).

```sh
cargo run --release --features studio --example studio
cargo run --release -p motion-gallery --features studio,gpu --bin motion-gallery -- studio --demo type-in-motion
```

## Preview and Playback

The default theme is neutral dark gray with a teal accent, with a light option; the
sun/moon icon in the header switches it. Colors, spacing, corner radii and type sizes
come from one theme module, so custom-painted regions follow the active theme. The
header also holds the preview scale, backend and mute controls (mute is a speaker
icon). The native window title uses the scene name. The sidebar icon at the right
edge of the header toggles the Inspector; narrow windows hide it by default. There is no left sidebar or preview
refresh button. A fixed bottom status bar displays the selected backend, displayed
frame and busy indicator without moving during playback. Long backend status remains
available in its hover tooltip.

Preview uses a bounded coordinator and reusable frame-renderer slots. Seeking pauses
playback and discards obsolete replies. Scale, quality, backend and opt-in bounds/pixel
inspection form the render identity. Normal Studio frames are transparent: scene
background, dark/white and checkerboard choices are composed by the Viewer without
rebuilding the renderer. Asset failures keep placeholders and diagnostics.

Playback and timeline dragging use Draft. Dragging renders at 25%; release first
restores the selected scale, then an idle 150 ms interval requests Final. Draft uses
0.35 px flattening, one shutter sample including nested subtrees, half-density ink
material scaled to the device, and a single-box blur approximation. Final retains
the original 1/256 px tolerance, shutter samples and three-box filter kernels.
Studio exports explicitly select Final regardless of the preview quality.

View Fit/50/100/200% zoom is independent of 25/50/100% render scale. Middle-drag
or Space-drag pans (the cursor becomes a hand); pinch or Ctrl-scroll zooms around
the pointer, and trackpad scrolling pans once zoomed. The viewer toolbar uses icon
toggles for safe areas, grid, node bounds and the pixel inspector. A corner chip
shows the output size and current zoom. The checkerboard is a single repeating
texture rather than per-cell rectangles. Background choices, safe-area overlays and
a grid affect presentation only. Pixels enables explicit memory output/readback;
the inspector reports sRGB8 and decoded SDR linear values, not the unclipped HDR
working buffer. Capture one reference for two-frame A/B split comparison. Once the
reference is captured, Pixels can be disabled to resume native GPU preview; retaining
the reference alone does not require current-frame CPU pixels or readback.

Bounds uses a separately planned read-only snapshot of the compiler's actual
layout and z order. Picking is approximate visual-bounds picking, not exact raster
coverage. The Node tab shows occurrence paths/names, transforms, filters and local
clip/mask metadata. Inspection errors enter diagnostics without changing exports.
The Diagnostics tab retains at most 128 entries and seeks to the selected frame;
matching occurrence paths are highlighted when the inspection reply arrives.

The resizable timeline draws Timeline tracks, clip activation ranges and incoming
transition overlaps. The ruler adapts its
tick spacing and shows frames or minutes:seconds, and the transport shows a
`MM:SS.mmm` timecode. The playhead is a red line with a flag; hovering the ruler shows
the time, frame and a thumbnail. In/Out controls share the export and loop interval,
with Out exclusive; drag the In/Out handles on the ruler or use Mark In/Out, and
frames outside the range are dimmed. L toggles the same loop state as the transport
button without seeking or changing the playback clock. Space plays/pauses,
Left/Right steps one frame (ten with Shift), and Home/End seeks first/last.
Keyboard-input focus suppresses transport shortcuts; Space-drag pans the Viewer.
J/K/L shuttle controls and I/O/F shortcuts are not implemented. Add/remove cue markers at the
playhead; these are editor annotations, not an engine scheduling API. A background
AudioAnalysis worker builds a 1024-point RMS energy waveform, capped at 300 seconds.
Analysis failures are diagnostics, not visual-frame failures. The cache strip shows
actually retained frames, merged into contiguous runs. Hover requests separate CPU/Draft/25% thumbnails. An
independent LRU holds at most 128 images and 16 MiB, each at most 160x90 pixels;
thumbnail replies never replace the main preview or pin GPU frame textures.
Seek/setting changes cancel pending thumbnail requests and reject old-generation
completions. Resource-error placeholders are not retained as healthy thumbnails.

The preview uses the GPU when the window's device is available and the CPU otherwise;
the backend selector lists only CPU and GPU. GPU admission failures, including
`max_bytes`, and device loss are typed errors and never switch the preview to the CPU.
The backend selector controls the preview workers independently of export settings;
changing it rebuilds their renderers and invalidates previews from the previous selection.
Frame-cache capacity is separate from per-frame GPU storage allowance.

Playback is real-time only: Play waits for its first frame (at most two seconds),
whose neighbors render in parallel with it, and then starts the clock and sound without
preparing a complete film or waiting for any later frame. If rendering falls behind, the
next requests advance with the clock without waiting for the previous frame. Speculative
targets account for the latency observed for the playback settings within the current
playback (cold first frames and Final idle renders do not count); late obsolete frames
are discarded, and the current frame is not rendered when a render takes longer than a
frame period, because it could only arrive after the clock has passed it.
Exports still render every requested frame independently of preview playback.

Playback uses up to six slots (available CPUs minus one, further limited by 2 GiB
divided by the largest per-frame allowance). Its speculative window holds up to one
predicted frame more than there are slots (at least four) when the cache permits; a
multi-slot pool reserves one slot for seek. Paused rendering and low-priority neighbor prefetch run one job at a time.
Seek/setting changes invalidate obsolete generations; already executing callbacks
finish before their slot stops, but cannot publish obsolete results. Only requested
main frames are published, and sound remains the playback clock. No complete-film
preparation or alternative playback mode is introduced.

By default every preview session renders on its own GPU device and the UI uploads the
read-back frame as a premultiplied ColorImage. The UI's wgpu device holds its fence lock
while the UI thread waits for the next drawable, so rendering on it stalls every
submission from the preview workers: Signal Garden at 960x540 took about 135 ms per
frame on one slot and gained nothing from more slots, against about 50 ms on a private
device. Set `MOTION_STUDIO_SHARED_GPU=1` to render on the UI's device instead and hand
frames over as textures without a readback; the ring below applies only to that mode.
The ring's textures are Rgba8Unorm because the renderer writes encoded sRGB bytes into
them; egui samples a native texture as linear and encodes it for display, so they are
registered through an Rgba8UnormSrgb view of the same bytes. Without that view the shared
mode showed every color re-encoded, visibly lighter than the CPU and private-device
previews, which agree to within one level per channel.

Native output and worker-side premultiplied CPU uploads share a texture ring per active
size/device. The ring holds three slots when idle and grows with the playback window
(slots plus predicted frames plus the displayed one, at most twelve and within the frame
cache budget), so prefetched frames are not evicted before the clock reaches them. UI
registrations are reused by slot ID. A texture is
reused only after all frame leases release it; evictable native cache entries do not
permanently pin the ring. Dispatch reserves capacity before handing a frame to a
worker, so concurrent jobs cannot overbook the ring. When all slots remain leased
or reserved, dispatch pauses instead of falling back to full-frame CPU readback and
upload. Reservations release after submission or failure; size/device changes
retire slots safely. Without scene GPU support the UI uploads a premultiplied
ColorImage; pixel inspection retains its
explicit straight-RGBA/readback path. Driver/egui references and retired leases are
not a claim that process GPU memory is limited to exactly the ring size.

The preview LRU defaults to 256 MiB of logical retained payload, without a fixed
frame-count ceiling. `MOTION_STUDIO_CACHE_MIB=0..1024` overrides it; zero disables
retention and prefetch. CPU pixels are shared between the cache and replies rather
than copied on cache hits. Accounting includes CPU/GPU pixel payload and entry
metadata, but excludes allocator/driver overhead, renderer resources and externally
held frames: it is not a process RSS or total GPU-memory limit. At 50% scale, an
eight-second 960x540/30fps film needs about 119 MiB for RGBA pixels alone.

Workers use the same RenderOptions/RenderSession contracts as headless output;
FrameRenderer supplies a composition's indexed clock. Request diagnostics remain
session-owned; author computations and bounded coverage/layout/material partitions
are shared across preview slots and setting changes. Each registered scene has an
isolated author partition, and panic/device loss invalidates its author/frame cache.
This recovery applies to a failure published for the requested main frame; obsolete
or speculative failures are discarded rather than promoted to main diagnostics.
Explicit pixel inspection needs memory output; normal native GPU previews keep
their pixels on the device.

Internally, PlaybackState, InspectorState, ExportState and DiagnosticsState group
their owning controls and observations. PreviewState remains independent: an old
frame can be displayed while its replacement is pending. The checked StudioClock
supplies indexed timing. Grouping preserves preview-worker, texture-lease, audio
and export destruction order; it does not merge concurrent request states.

The Stats tab shows actual worker latency, recently presented/target fps (32-frame
window, reset when starting playback), cache hits, skipped playback frames,
logical work/peak bytes and opt-in evaluation/compile/coverage/memory-output timing,
worker upload enqueue time and native preparation/submission wall time.
CPU memory output is fused, not separately timed composition/conversion. Native
GPU submissions keep their own observations and worker submit-to-completion latency;
these are CPU wall-clock observations, not GPU timestamp queries, RSS or acceptance
benchmarks. Headless render sessions leave stage timing disabled by default.

Native preview output reads the final RGBA32F working texture directly, without
a full linear-float output buffer or resolve pass. Crop origins, deferred Plus
alpha, finite-value errors and RGBA/BGRA/half-float target contracts are retained.
The original conservative logical byte/work allowance is unchanged. Completion
and publication policy are also unchanged; this is not extra frame pipelining.

For a headless measurement of the same submit/finish API on the Gallery film:

```sh
MOTION_BENCH_TEXTURES=1 MOTION_BENCH_BACKENDS=gpu \
MOTION_BENCH_CASES=type-in-motion MOTION_BENCH_QUALITY=draft \
MOTION_BENCH_SEQUENCES=1 MOTION_BENCH_SEQUENCE_FRAMES=240 \
MOTION_BENCH_VIDEO=0 MOTION_BENCH_FONT=tests/fixtures/fonts/NotoSans-Regular.ttf \
cargo bench --locked --offline -p motion-bench --bench films --features gpu
```

Texture mode requires GPU-only sequences without video; quality is `draft` by
default or explicit `final`. It records complete submit/finish latency, preparation
plus submission, and completion wait for each sequential frame. Three raw RGBA
references are read back outside timing. The sequence has 240 samples, independent
of the setup measurement's `MOTION_BENCH_SAMPLES`. Checksums encode frame order,
not every rendered pixel. These are CPU wall times without a window, cache ring,
playback scheduler, display latency or dropped-frame observations; they must not
be reported as Studio presentation FPS or a complete visual acceptance test.

Transport preserves rational frame rates. When sound is active, audible samples
drive the visual clock. Audio preparation
holds the current image; underruns/device/timestamp faults pause instead of silently
switching to wall time. Mute affects preview only. Empty mixes open no sound device.

Timeline audio and configured local files are AudioClip occurrences in one mix.
The audio worker prepares at most 8192 output samples per window through MixedAudio;
source offsets and clip-local fades continue across windows and export crops.
Files use the same deferred decoder as headless export; decoding is not performed in the device
callback. Current device output requires 48 kHz stereo f32. In/Out looping
restarts preparation and is not guaranteed gapless.

## Export and Session

The Inspector tabs (Export, Node, Diagnostics, Stats) are four equal-width icon
buttons with tooltips. Stats groups values into Playback, Preview worker, Cache and
Frame cost sections. The right Inspector queues H.264, H.265, VP9 with alpha, ProRes 4444, PNG sequences,
GIF, and individual PNG stills. Presets include 1080p H.264, transparent ProRes and
640x360 GIF. GIF/PNG sequences are explicitly silent; GIF uses the timeline's rate
up to 100 fps, otherwise 30 fps. At most 16 pending tasks run serially, each with a
snapshot of size/range/backend/shutter/codec/audio settings. Queued tasks can be
removed; the current task can be cancelled. Up to 64 completion records retain
typed-error messages and an Open folder action. Size, range,
backend and shutter settings also apply when streamed file audio is configured.
Selected ranges preserve clip-local audio fades. PNG ignores audio.

The Export form keeps only transient values (size, range, backend, shutter, codec choice); one conversion turns it into the encode `ExportOptions` that `export` consumes. A `StudioScene` holds a single `Timeline` that preview, audio playback and export share, with audio files resolved by the mixer.

Exports run separately, retain typed errors and observe cooperative cancellation.
Closing discards queued work, cancels the active job and waits for cleanup. Existing output
is replaced only after successful finalization with explicit overwrite permission.

Session persistence at `.motion/studio.json` stores bounded scene/frame/preview/range/cue
settings without replaying or exporting automatically. Audio configuration can be
saved/loaded separately; referenced source media is not embedded.

This redesign excludes parameter editing, shuttle shortcuts/command palette, docked
panels, audio scrubbing and the debug-build warning. Session persistence is not a
docking implementation.

## Hot Reload

A scene registered with `StudioScene::reloadable(watch, load)` is rebuilt while the
workbench runs. `Project` registers it for declarative documents: `studio film.json`, or a
timeline added with `Project::document`, watches the document and every file it referred
to when Studio started. The watched files' modification times are polled four times a
second; once a change has been quiet for 200 ms, `load` runs on a worker thread.

A successful reload replaces the scene and keeps the playhead, cues, preview settings and
export settings, clamped to the new length. Cached frames are dropped and playback is
paused. A failure keeps the previous scene, shows the error in the header and adds it to
the diagnostics list, so a file saved halfway through an edit never blanks the preview.
Exports already queued or running keep the scene they started with. Resources that appear
only after launch are not watched, and the per-scene GPU context requested by
`MOTION_STUDIO_SHARED_GPU` applies to the launch scene only.

See [Gallery](../gallery/README.md), [rendering](design/rendering.md) and
[reliability](design/reliability.md). Worker tests do not certify physical UI interaction,
real-device audio, resizing across displays or cross-platform driver recovery.