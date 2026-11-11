use super::*;
use motion_core::Rect;
use motion_render::RenderPrepare;

#[test]
fn typst_math_frame_keeps_original_spans_without_source_rewriting() -> motion_core::Result<()> {
    let compiler = crate::TypstCompiler::new();
    for math in ["x^2 + frac(1,y)", "sum_(i=1)^n i", "sqrt(x^2+1)"] {
        let input = Input::new(math, 24.0)?;
        let compiled = compiler.layout(input.source.clone());
        let document = compiled
            .output
            .map_err(|diagnostics| failure(format!("{diagnostics:?}")))?;

        fn visit(frame: &typst::layout::Frame, input: &Input, spans: &mut Vec<Range<usize>>) {
            for (_, item) in frame.items() {
                match item {
                    typst::layout::FrameItem::Group(group) => visit(&group.frame, input, spans),
                    typst::layout::FrameItem::Text(text) => {
                        for glyph in &text.glyphs {
                            if let Some(range) = input.range(glyph.span.0) {
                                spans.push(range);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut spans = Vec::new();
        visit(&document.pages()[0].frame, &input, &mut spans);
        assert!(!spans.is_empty());
        assert!(
            spans
                .iter()
                .all(|range| range.start < range.end && range.end <= math.len())
        );
        assert!(document.pages()[0].frame.size().x.to_pt() > 0.0);
    }
    Ok(())
}

#[test]
fn typst_formulas_keep_curves_rules_original_ranges_and_real_pixels() -> motion_core::Result<()> {
    let compiler = FormulaCompiler::new();
    for math in [
        "x^2 + frac(1,y)",
        "sum_(i=1)^n i",
        "sqrt(x^2+1)",
        "integral_0^1 x dif x",
    ] {
        let formula = compiler.compile(math, &FormulaOptions::default())?;
        assert!(formula.symbols().iter().any(|symbol| !symbol.structural));
        assert!(
            formula
                .symbols()
                .iter()
                .filter_map(|symbol| symbol.source.as_ref())
                .all(|range| range.end <= math.len()
                    && math.is_char_boundary(range.start)
                    && math.is_char_boundary(range.end))
        );
        assert!(formula.symbols().windows(2).all(|pair| {
            pair[0]
                .source
                .as_ref()
                .map(|range| range.start)
                .unwrap_or(usize::MAX)
                <= pair[1]
                    .source
                    .as_ref()
                    .map(|range| range.start)
                    .unwrap_or(usize::MAX)
        }));
        let size = formula.tree.view_box();
        let output =
            motion_core::PixelSize::new(size.width().ceil() as u32, size.height().ceil() as u32);
        let frame = formula
            .node()
            .compile(
                Size::new(size.width(), size.height()),
                output,
                motion_core::ValidationLimits::default(),
            )?
            .prepare(
                motion_core::ValidationLimits::default(),
                motion_core::RasterLimits::default(),
            )?
            .render_linear(motion_core::RasterLimits::default())?;
        assert!(frame.linear_pixels().any(|pixel| pixel[3] > 0.5), "{math}");
        if math.contains("frac") {
            assert!(formula.symbols().iter().any(|symbol| symbol.structural));
        }
    }
    let text = "#text[ABC]";
    let formula = compiler.compile(text, &FormulaOptions::default())?;
    let offset = text.find('B').unwrap();
    let selected = formula.select_source(offset..offset + 1)?;
    assert_eq!(selected.symbols().len(), 1);
    assert_eq!(selected.symbols()[0].character.as_ref(), "B");
    Ok(())
}

#[test]
fn formula_cache_versions_concurrency_and_source_diagnostics_are_isolated()
-> motion_core::Result<()> {
    let compiler = FormulaCompiler::new();
    let options = FormulaOptions::default();
    let mut results = Vec::new();
    std::thread::scope(|scope| {
        let handles = (0..4)
            .map(|_| scope.spawn(|| compiler.compile("frac(x, y)", &options).unwrap()))
            .collect::<Vec<_>>();
        for handle in handles {
            results.push(handle.join().unwrap());
        }
    });
    assert!(
        results
            .iter()
            .all(|result| Arc::ptr_eq(&results[0], result))
    );
    let revised = compiler.compile(
        "frac(x, y)",
        &FormulaOptions {
            revision: 1,
            ..options.clone()
        },
    )?;
    assert!(!Arc::ptr_eq(&results[0], &revised));
    assert_ne!(results[0].version(), revised.version());
    let other = FormulaCompiler::new().compile("frac(x, y)", &options)?;
    assert_ne!(other.version(), results[0].version());
    let error = compiler
        .compile("unknown_function(x)", &options)
        .unwrap_err();
    assert_eq!(error.kind(), motion_core::ErrorKind::Formula);
    let diagnostics = error.downcast_ref::<FormulaCompileError>().unwrap();
    assert!(diagnostics.diagnostics().iter().any(|diagnostic| {
        diagnostic
            .source
            .as_ref()
            .is_some_and(|range| range.end <= 19)
    }));
    assert_eq!(
        error.to_string(),
        compiler
            .compile("unknown_function(x)", &options)
            .unwrap_err()
            .to_string()
    );
    assert!(
        compiler
            .compile(
                "x",
                &FormulaOptions {
                    size: f64::NAN,
                    ..options
                }
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn fractions_scripts_roots_keep_measured_geometry_and_bounded_input() -> motion_core::Result<()> {
    let compiler = FormulaCompiler::new();
    let options = FormulaOptions {
        size: 32.0,
        ..FormulaOptions::default()
    };
    let ink = |formula: Arc<FormulaAsset>| -> motion_core::Result<Rect> {
        let tree = formula.vector_tree();
        let mut bounds = Rect::default();
        for leaf in tree.paint_leaves() {
            if let motion_content::VectorItem::Path(path) = leaf.item {
                bounds = bounds.union(motion_core::display_list::mapped_bounds(
                    path.path.bounding_box(),
                    leaf.transform,
                    "formula.ink",
                )?);
            }
        }
        Ok(bounds)
    };
    let base = ink(compiler.compile("x", &options)?)?;
    let fraction = ink(compiler.compile("frac(1, 2)", &options)?)?;
    let scripts = ink(compiler.compile("sqrt(x^2_i)", &options)?)?;
    assert!(
        fraction.height() > base.height(),
        "fraction={fraction:?}, base={base:?}"
    );
    assert!(
        scripts.width() > base.width(),
        "scripts={scripts:?}, base={base:?}"
    );
    assert!(
        scripts.height() > base.height(),
        "scripts={scripts:?}, base={base:?}"
    );
    let deep = format!("{}x{}", "sqrt(".repeat(2048), ")".repeat(2048));
    assert!(deep.len() > 8192);
    assert!(compiler.compile(deep, &options).is_err());
    Ok(())
}

#[test]
fn compiler_cache_eviction_and_restricted_world_preserve_owned_results() -> motion_core::Result<()>
{
    let compiler = FormulaCompiler::new();
    let pinned = compiler.compile("x", &FormulaOptions::default())?;
    for revision in 1..36 {
        compiler.compile(
            "x",
            &FormulaOptions {
                revision,
                ..Default::default()
            },
        )?;
    }
    let stats = compiler.cache_stats()?;
    assert!(
        stats.entries <= 32 && stats.retained_bytes > 0 && stats.retained_bytes <= 64 * 1024 * 1024
    );
    assert_eq!(pinned.source(), "x");
    assert!(pinned.select_symbol("x").is_ok());
    let error = compiler
        .compile("#read(\"forbidden.txt\")", &FormulaOptions::default())
        .unwrap_err();
    assert_eq!(error.kind(), motion_core::ErrorKind::Formula);
    assert!(error.to_string().contains("access denied"), "{error}");
    let unicode = compiler.compile("α + x", &FormulaOptions::default())?;
    assert!(unicode.select_source(1..2).is_err());
    assert!(unicode.select_symbol("α").is_ok());
    Ok(())
}

#[test]
#[ignore = "requires explicit MOTION_TEST_FONT; no font discovery"]
fn explicit_formula_fonts_are_frozen_and_environment_versioned() -> motion_core::Result<()> {
    let path = std::env::var_os("MOTION_TEST_FONT")
        .ok_or_else(|| std::io::Error::other("Set MOTION_TEST_FONT"))?;
    let mut fonts = motion_content::FontLibrary::new();
    fonts.load_file(path)?;
    let compiler = FormulaCompiler::with_fonts(&fonts)?;
    let formula = compiler.compile(
        "#text(font: \"Arial\")[ABC] + x",
        &FormulaOptions::default(),
    )?;
    assert!(
        formula
            .symbols()
            .iter()
            .any(|symbol| symbol.family.as_ref().contains("Arial"))
    );
    assert_ne!(formula.version().1, FormulaCompiler::new().environment());
    Ok(())
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires real native GPU; uses bundled math fonts, no window or encoder"]
fn formula_writing_selection_and_morph_match_cpu_gpu() -> motion_core::Result<()> {
    use motion_core::NodeModifiers;
    let compiler = FormulaCompiler::new();
    let first = compiler.compile(
        "x + frac(1, y) + sqrt(x)",
        &FormulaOptions {
            size: 16.0,
            ..Default::default()
        },
    )?;
    let second = compiler.compile(
        "y + x + x",
        &FormulaOptions {
            size: 16.0,
            fill: Color::rgb(40, 190, 230),
            revision: 1,
            ..Default::default()
        },
    )?;
    let morph = first.morph_to(&second)?;
    let selected = first.select_symbol("x")?;
    let limits = motion_core::RasterLimits::default();
    let mut gpu = motion_render::gpu::Gpu::new()?;
    let mut cases = 0;
    let mut maximum = 0u8;
    for progress in [0.0, 0.2, 0.5, 0.85, 1.0, 0.5] {
        for node in [
            first.node(),
            first.write(progress)?,
            selected.write(progress)?,
            selected.apply(|node| node.opacity(0.5)),
            morph.sample(progress)?,
        ] {
            let node = node.translate(2.125, 1.25).opacity(0.8).clip_with(
                Rect::new(0.0, 0.0, 192.0, 72.0),
                motion_core::ClipMode::Geometric,
            );
            let frame = node
                .compile(
                    Size::new(192.0, 72.0),
                    motion_core::PixelSize::new(192, 72),
                    motion_core::ValidationLimits::default(),
                )?
                .prepare(motion_core::ValidationLimits::default(), limits)?;
            let srgb = motion_render::PixelFormat::SrgbRgba8 {
                premultiplied: false,
            };
            let render = |gpu: &mut motion_render::gpu::Gpu| gpu.render_linear(&frame, limits);
            let actual = render(&mut gpu)?;
            assert_eq!(actual, render(&mut gpu)?);
            let expected = frame.render_buffer(srgb, limits)?;
            let actual = actual.to_format(srgb, limits)?;
            for (expected, actual) in expected.bytes().iter().zip(actual.bytes()) {
                maximum = maximum.max(expected.abs_diff(*actual));
            }
            assert!(maximum <= 1, "case={cases},max_byte={maximum}");
            cases += 1;
        }
    }
    assert_eq!(cases, 30);
    println!("Typst formulas cases={cases} max_byte={maximum} GPU replay=exact");
    Ok(())
}
