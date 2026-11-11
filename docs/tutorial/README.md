# motion-rs Tutorial

This tutorial follows the authoring workflow rather than crate boundaries. Start with a still image, then learn animation, scene structure, content, audio and export before exploring rendering and extensions.

The examples use the current workspace API. The Cargo package is `motion-rs`; the Rust library is `motion`.
All Rust examples use `rust,no_run`. Enable the features listed in each chapter and supply any required external files. Reading or checking the documentation does not automatically generate output.

## Reading Paths

- First-time users: read 00, then work through 01–05. Chapter 02 covers value types, 03 time-varying values, 04 scene structure and 05 low-level drawing interfaces.
- Creating films: after the main path, choose 06–08 for your content, then read 10–12. GPU internals are not a prerequisite.
- Integrating into an application: focus on 09, 11 and 12 for output, device and task lifetimes.
- Tuning and contributing: read 13 and 14 alongside the existing design documents. Internal implementations are not necessarily stable authoring APIs.

## Contents

| Chapter | Topics |
| --- | --- |
| [00 Overview and Pipeline](00-overview.md) | Design principles, features, output boundaries |
| [01 Quick Start](01-quick-start.md) | Installation, first frame, video, CLI, Gallery, Studio |
| [02 Foundations](02-foundations.md) | Time, geometry, color, transforms, paths, randomness |
| [03 Animation](03-animation.md) | Tween, Keyframes, easing, Spring, animation primitives, Morph |
| [04 Scenes and Timeline](04-scenes-and-timeline.md) | scene, Node, layout, Track, transitions, cameras, Project |
| [05 Drawing](05-drawing.md) | Canvas, Paint, Stroke, Clip, Filter, validation, budgets |
| [06 Content and Resources](06-content.md) | Images, SVG, text, Ink, Diagram, Captions, resource stores |
| [07 Particles](07-particles.md) | Trajectories, simulation, styles, instances, determinism |
| [08 Typst and Formulas](08-formula-typst.md) | Compilation, selection, writing, morphing, trust model |
| [09 Rendering](09-rendering.md) | CPU/GPU, prepared frames, sessions, caches, memory and texture output |
| [10 Audio](10-audio.md) | Timeline occurrences, decoding, mixing, resampling, analysis |
| [11 Export](11-export.md) | Encoders, formats, transactions, concurrency, progress, cancellation |
| [12 Studio](12-studio.md) | Workbench, preview, real-time playback, audio, export queue |
| [13 Performance and Reliability](13-performance-and-reliability.md) | Measurement, budgets, errors, determinism, testing |
| [14 Architecture and Extensions](14-architecture-and-extending.md) | Ownership, dependency direction, public extension points, contribution boundaries |
| [15 Cinematic Output](15-cinematic.md) | Display transforms, Main10, LUT/CDL, lens optics, DOF, shutter and film looks |
| [16 Declarative Documents](16-declarative.md) | JSON films, animated properties, schema, Project and CLI loading, diagnostics |
| [Appendix: Gallery Guide](appendix-gallery.md) | Film structure and related chapters |

## Example Conventions

1. Each chapter includes its purpose, features, a minimal program, API tables, pitfalls and further reading.
2. CPU is used unless stated otherwise. Enabling `gpu` adds support; it does not select the backend.
3. File paths are relative to the program's working directory. Configure resource roots and font environments explicitly.
4. By default, examples do not overwrite existing work, automatically launch windows or run performance measurements.
5. Short authoring examples use `motion::prelude::*`. Use `motion::extend` for low-level protocols, `motion::render` for explicit rendering interfaces, `motion::encode` for encoders and progress sinks, and `motion::eager` for precompiled content.

The [design contracts](../design/README.md) describe architecture, rendering and
ownership; [development checks](../development.md) describe validation procedures.
These references are not prerequisites for the tutorial.