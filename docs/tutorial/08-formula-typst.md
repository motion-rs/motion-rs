# 08 Typst and Formulas

## What This Chapter Covers

Turn mathematical typesetting into shared vector scene content, then select symbols, animate writing and morph formulas.
Single-page typesetting requires `typst`. Formulas, selection and formula Morph require `formula`, which includes the former.

## Minimal Example

Enable `formula`. The compiler uses an explicit in-memory environment and bundled fonts; no system Typst command is needed.

```rust,no_run
use motion::eager::FormulaCompiler;
use motion::prelude::*;

fn main() -> motion::Result {
    let compiler = FormulaCompiler::new();
    let formula = compiler.compile(
        "E = m c^2",
        &FormulaOptions {
            size: 40.0,
            fill: Color::rgb(20, 140, 120),
            ..FormulaOptions::default()
        },
    )?;
    let selected = formula
        .select_symbol("c")?
        .apply(|node| node.translate(0.0, -8.0));
    let timeline = Timeline::new(Size::new(480.0, 180.0), fps!(30)).track(|track| {
        track.scene(1.seconds(), move |_| selected.clone().translate(40.0, 55.0))
    }).build()?;
    render_still(&timeline, Time::ZERO, &RenderOptions::default())?
        .save_png("tutorial-formula.png")
}
```

Compilation and selection can fail before scene construction. Subsequent frames reuse the prepared result.

## Choosing an Interface

| API | Purpose and Timing |
| --- | --- |
| `TypstCompiler::compile` | Compile trusted single-page markup into a `TypstDocument` |
| `TypstDocument::node` | Add an outlined SVG page to the shared vector pipeline |
| `FormulaCompiler::compile` | Prepare a math `FormulaAsset` and metadata in advance |
| `Formula::typst` | Retain deferred math construction in a scene |
| `select_symbol` / `select_subexpression` | Select symbols or subexpressions from their actual sources |
| `write` | Reveal a vector formula by progress |
| `morph_to` / `FormulaMorph::sample` | Establish correspondence, then sample the morph |

Math input uses Typst math syntax, not implicit LaTeX translation. Do not pass external LaTeX package strings directly to the compiler and expect compatibility.

## Deferred Authoring

This also requires `formula` and suits scenes that do not need metadata in advance:

```rust,no_run
use motion::prelude::*;

fn main() -> motion::Result {
    let equation = Formula::typst("f(t) = sin(2 pi t)")
        .size(36.0)
        .fill(Color::rgb(20, 140, 120));
    let timeline = Timeline::new(Size::new(480.0, 180.0), fps!(30)).track(|track| {
        track.scene(2.seconds(), move |context| {
            equation.clone().write(context.progress()).translate(32.0, 60.0)
        })
    }).build()?;
    render_still(&timeline, 1.seconds().after_origin(), &RenderOptions::default())?
        .save_png("tutorial-formula-writing.png")
}
```

`Formula` retains source and parameters; failures return diagnostics at scene compilation. `.write` changes progress without creating a compiler per frame. Changes to fonts, source, size or fill still need their true cache identity.

## Selection and Writing

Selection depends on original Typst sources and rendered occurrences, not source guesses from final glyph positions. Repeated symbols can have multiple matches. Range selection uses half-open symbol ranges, not arbitrary string indices.

Writing reveals vectors rather than truncating strings per frame. Truncating markup often breaks syntax, and re-typesetting changes layout. A complete formula with explicit progress better suits fixed-layout animation.

## Formula Morphing

Compile both endpoints, call `morph_to` outside the scene, then `sample` by progress. Harmonics follows this workflow:

```rust,no_run
use motion::eager::FormulaCompiler;
use motion::prelude::*;

fn main() -> motion::Result {
    let compiler = FormulaCompiler::new();
    let options = FormulaOptions::default();
    let first = compiler.compile("f(t) = sin(2 pi t)", &options)?;
    let second = compiler.compile("f(t) = sin(4 pi t)", &options)?;
    let morph = first.morph_to(&second)?;
    let _midpoint = morph.sample(0.5)?;
    Ok(())
}
```

This is content and symbol correspondence, not a crossfade between bitmaps. Inspect selection and structural matches. For unsuitable direct matches, explicitly choose fades or reorganize the endpoint expressions.

## Trust Model

The Typst compiler provides explicit input and in-memory fonts, without arbitrary external files, packages/imports or current-date access. Bounded single-page documents, sources and outputs do not imply hard limits on compilation time, temporary memory or internal memoization.

Compile trusted input only. Servers accepting untrusted markup need process isolation, resource limits, timeouts and their own review boundary. Library input-length checks are not a sandbox.

Supply frozen FontLibrary bytes when extending project fonts. Later registrations should not silently rewrite existing compilers or document snapshots.

## Common Pitfalls

- Enabling `typst` does not provide formula APIs. Math selection and Morph require `formula`.
- Prepare or reuse deferred objects instead of creating TypstCompiler or recompiling unchanged source every frame.
- Byte indices, source ranges and rendered-symbol ranges are different.
- Input-length limits do not sandbox compilation CPU or memory.
- Formulas are not just images; their vectors participate in ordinary layout, transforms, clipping and composition.

## Next Steps

Next chapter: [09 Rendering](09-rendering.md).
Complete program: [Typst formula example](../../examples/typst_formula.rs).
Film: [Harmonics](../../gallery/src/demos/harmonics.rs). Contracts: [compiler implementation](../../crates/motion-formula/src/compiler.rs), [Typst document implementation](../../crates/motion-formula/src/typst_asset.rs).