# Documentation

## Learn and Use

- [Tutorial](tutorial/README.md): the authoring learning path, from the first still
  to animation, content, audio, export and extensions.
- [Motion Studio](studio.md): desktop workbench operation, preview and playback.
- [Gallery](../gallery/README.md): finished short films.
- [Command line](cli.md): the `list`, `still`, `export`, `studio` and `schema` commands of a `Project` binary.
- [Rustdoc source](../crates/motion/src/lib.rs): exact public signatures and examples.

## Understand the Engine

[Design contracts](design/README.md) explain principles, architecture, core/content
ownership, rendering, export, audio, resources, GPU execution, particles, reliability and
declarative documents. They are references
for integration and contribution, not prerequisites for the tutorial.

## Maintain and Measure

- [Development checks](development.md): feature matrices, tests, fixtures and CI.
- [Source style](style.md): file layout, field order, comments and lint conventions.
- [Development plan](plan.md): project priorities, remaining work and the performance plan.
- [Benchmarks](benchmarks.md): workloads, procedures and current measured results.
- [Measurement manifest](performance-runs.json): recorded runs and provenance.
- [Quality report](quality-report.md): image-quality measurements, with provenance.

## Conventions

Documentation is written in English. Terms are defined in the
[design glossary](design/README.md#glossary). Every Rust block in the tutorial, the command-line
page and the rendering and resources design pages is compiled as a Rustdoc test (see
[development checks](development.md)).
