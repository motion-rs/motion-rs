use super::*;
use motion_core::Point;
use motion_core::Shape;
use motion_core::TimeLiteral;
use motion_render::RenderPrepare;

#[test]
fn session_layout_cache_reuses_fresh_texts_and_evicts_lru_without_clearing() -> motion_core::Result
{
    use super::*;
    let mut fonts = FontLibrary::new();
    let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/fonts/NotoSans-Regular.ttf");
    fonts.load_file(&font)?;
    let render_cache = motion_core::RenderCache::with_limits(motion_core::CacheLimits {
        entries: 2,
        bytes: 1024 * 1024,
    });
    let cache = render_cache
        .partition(motion_core::CachePartition::Paragraph)
        .clone();
    let assets = motion_core::Assets::new(".")?.with_fonts(&fonts);
    let drawing = DrawContext::default()
        .with_assets(&assets)
        .with_render_cache(render_cache.clone());
    let first = Text::new("first").resolve(&drawing)?;
    let second = Text::new("second").resolve(&drawing)?;
    assert!(Arc::ptr_eq(&first, &Text::new("first").resolve(&drawing)?));
    Text::new("third").resolve(&drawing)?;
    assert!(Arc::ptr_eq(&first, &Text::new("first").resolve(&drawing)?));
    assert!(!Arc::ptr_eq(
        &second,
        &Text::new("second").resolve(&drawing)?
    ));
    assert_eq!(cache.len(), 2);
    assert!(cache.retained_bytes() > 0 && cache.retained_bytes() <= cache.max_bytes());
    assert!(!Arc::ptr_eq(
        &first,
        &Text::new("first").size(33.0).resolve(&drawing)?
    ));
    let original = Text::new("revision").resolve(&drawing)?;
    fonts.load_file(font)?;
    assert!(!Arc::ptr_eq(
        &original,
        &Text::new("revision").resolve(&drawing.with_fonts(&fonts))?
    ));
    Ok(())
}

#[test]
fn session_layout_cache_converges_concurrent_misses_and_clear_releases_charges()
-> motion_core::Result {
    use super::*;
    let mut fonts = FontLibrary::new();
    fonts.load_file(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/fonts/NotoSans-Regular.ttf"),
    )?;
    let render_cache = motion_core::RenderCache::with_limits(motion_core::CacheLimits {
        entries: 8,
        bytes: 1024 * 1024,
    });
    let cache = render_cache
        .partition(motion_core::CachePartition::Paragraph)
        .clone();
    let drawing = DrawContext::default()
        .with_fonts(fonts)
        .with_render_cache(render_cache.clone());
    let layouts = std::thread::scope(|scope| {
        let requests = (0..8)
            .map(|_| scope.spawn(|| Text::new("Concurrent paragraph").resolve(&drawing).unwrap()))
            .collect::<Vec<_>>();
        requests
            .into_iter()
            .map(|request| request.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(
        layouts
            .iter()
            .all(|layout| Arc::ptr_eq(layout, &layouts[0]))
    );
    assert_eq!(cache.len(), 1);
    assert!(cache.retained_bytes() > 0);
    cache.clear();
    assert_eq!(cache.retained_bytes(), 0);
    assert!(!Arc::ptr_eq(
        &layouts[0],
        &Text::new("Concurrent paragraph").resolve(&drawing)?
    ));
    Ok(())
}

#[test]
fn static_text_repeated_configuration_keeps_layout_and_validates_paint() -> motion_core::Result {
    use super::*;
    let font = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/fonts/NotoSans-Regular.ttf");
    let mut fonts = FontLibrary::new();
    fonts.load_file(font)?;
    let text = Text::new("Static geometry")
        .size(32.0)
        .max_width(240.0)
        .fonts(&fonts);
    let first = text.layout(&fonts)?;
    let same = text
        .clone()
        .family("")
        .size(32.0)
        .weight(FontWeight::NORMAL)
        .line_height(1.3)
        .letter_spacing(0.0)
        .max_width(240.0)
        .alignment(TextAlignment::Start)
        .fill(Color::BLACK);
    assert!(Arc::ptr_eq(&text.layout_cache, &same.layout_cache));
    assert!(Arc::ptr_eq(&first, &same.layout(&fonts)?));
    let changed = text.clone().size(33.0);
    assert!(!Arc::ptr_eq(&text.layout_cache, &changed.layout_cache));
    assert!(!Arc::ptr_eq(&first, &changed.layout(&fonts)?));
    let invalid = motion_core::LinearGradient::new(Point::ORIGIN, Point::new(1.0, 0.0))
        .stop(f64::NAN, Color::WHITE)
        .stop(1.0, Color::BLACK);
    assert!(text.clone().fill(invalid).layout(&fonts).is_err());
    assert!(Arc::ptr_eq(&first, &text.layout(&fonts)?));
    Ok(())
}

#[test]
#[ignore = "requires explicit MOTION_TEST_FONT; no window, GPU or audio device"]
fn lazy_parts_use_nested_scene_time_and_replay_without_explicit_context() -> motion_core::Result {
    use super::*;
    use motion_core::{Scene, SceneContext, Size, TimeLiteral, Timeline};
    let mut fonts = FontLibrary::new();
    fonts.load_file(std::env::var("MOTION_TEST_FONT")?)?;
    let samples = Arc::new(Mutex::new(Vec::new()));
    let observed = samples.clone();
    let text =
        Text::new("AB")
            .fonts(&fonts)
            .size(20.0)
            .animate_parts(100.milliseconds(), move |part| {
                observed.lock().unwrap().push((part.index, part.time()));
                PartStyle::new()
            });
    let child = Timeline::new(Size::new(64.0, 32.0), 30)
        .track(|track| {
            track.push(
                (move |_: &SceneContext<'_>| Node::from(text.clone())).with_duration(1.seconds()),
            )
        })
        .build()?;
    let parent = Timeline::new(Size::new(64.0, 32.0), 30)
        .track(|track| {
            track
                .gap(1.seconds())
                .push(child.visual().with_duration(1.seconds()))
        })
        .build()?;
    let first = motion_render::render_still(
        &parent,
        1250.milliseconds().after_origin(),
        &motion_render::RenderOptions::default(),
    )?;
    assert!(
        first
            .bytes()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] != 0)
    );
    for (index, time) in samples.lock().unwrap().iter() {
        assert_eq!(
            *time,
            motion_core::Time::ZERO
                + (250.milliseconds()
                    - motion_core::Duration::from_ticks(
                        100.milliseconds().ticks() * *index as i64
                    ))
        );
    }
    assert!(!samples.lock().unwrap().is_empty());
    assert_eq!(
        first.bytes(),
        motion_render::render_still(
            &parent,
            1250.milliseconds().after_origin(),
            &motion_render::RenderOptions::default()
        )?
        .bytes()
    );
    Ok(())
}

#[test]
fn lazy_effects_do_not_layout_or_run_callbacks_while_building() {
    use super::*;
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = calls.clone();
    let text = Text::new("deferred").animate_parts((-1).seconds(), move |_| {
        observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        PartStyle::new()
    });
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 0);
    assert_eq!(
        Node::from(text)
            .measure(motion_core::ValidationLimits::default())
            .unwrap_err()
            .field,
        "delay"
    );
    let text = Text::new("deferred").write(f64::NAN);
    assert_eq!(
        Node::from(text)
            .measure(motion_core::ValidationLimits::default())
            .unwrap_err()
            .field,
        "progress"
    );
}

#[test]
fn empty_font_environments_fail_without_discovery_and_unicode_parts_are_graphemes() {
    let fonts = FontLibrary::new();
    assert!(Text::new("no implicit fonts").layout(&fonts).is_err());
    assert!(Text::new("").layout(&fonts).unwrap().glyphs().is_empty());
    let layout = ParagraphLayout {
        glyphs: Arc::from([]),
        bounds: Rect::default(),
        baseline: 0.0,
        lines: Arc::from([]),
        line_sources: Arc::from([]),
        text: "e\u{301}👩‍💻 word".into(),
    };
    let parts = layout.parts(TextSplit::Characters);
    assert_eq!(parts.len(), 7);
    assert_eq!(&layout.text[parts[0].source.clone()], "e\u{301}");
    assert_eq!(&layout.text[parts[1].source.clone()], "👩‍💻");
    assert!(Text::new("x").size(f64::NAN).layout(&fonts).is_err());
    assert!(
        Text::rich([TextSpan::new("").size(-1.0)])
            .layout(&fonts)
            .is_err()
    );
    assert!(
        Text::new("x")
            .max_width(f64::MIN_POSITIVE)
            .layout(&fonts)
            .is_err()
    );
    let error = Node::from(Text::new("no implicit fonts"))
        .compile(
            motion_core::Size::new(8.0, 8.0),
            motion_core::PixelSize::new(8, 8),
            motion_core::ValidationLimits::default(),
        )
        .unwrap_err();
    assert_eq!(error.field, "fonts");
    assert_eq!(layout.parts(TextSplit::Words).len(), 2);
    assert!(Text::new("x".repeat(16_385)).layout(&fonts).is_err());
    assert!(
        Text::rich(std::iter::repeat_n(TextSpan::new(""), 257))
            .layout(&fonts)
            .is_err()
    );
    assert!(PartStyle::new().opacity(1.1).validate().is_err());
    assert!(Text::new("").weight(FontWeight(0)).layout(&fonts).is_err());
}

#[test]
#[ignore = "requires explicit MOTION_TEST_FONT; no system discovery"]
fn explicit_line_limits_preserve_rich_spans_graphemes_and_ellipsis() -> motion_core::Result {
    let fonts = fixture();
    let text = Text::rich([TextSpan::new("Alpha beta gamma delta")
        .size(20.0)
        .fill(Color::BLACK)])
    .size(20.0)
    .line_height(1.2)
    .max_width(120.0)
    .alignment(TextAlignment::Center);
    assert!(text.layout(&fonts)?.lines().len() > 1);
    let (limited, truncated) = text.fit_lines(1, &fonts)?;
    assert!(truncated);
    let layout = limited.layout(&fonts)?;
    assert_eq!(layout.lines().len(), 1);
    assert!(layout.bounds().height() > 0.0);
    assert!(layout.text.ends_with('\u{2026}'));
    assert!(layout.bounds().width() <= 120.0);
    assert!(
        Text::new("x")
            .size(f64::NAN)
            .max_width(100.0)
            .fit_lines(1, &fonts)
            .is_err()
    );
    assert!(
        Text::new("oversized")
            .size(100.0)
            .max_width(1.0)
            .fit_lines(1, &fonts)
            .is_err()
    );
    let grapheme = Text::rich([
        TextSpan::new("e"),
        TextSpan::new("\u{301}").fill(Color::BLACK),
    ])
    .size(20.0)
    .max_width(100.0);
    assert_eq!(
        grapheme.layout(&fonts)?.parts(TextSplit::Characters).len(),
        1
    );
    let long = Text::new("e\u{301} e\u{301} e\u{301} e\u{301}")
        .size(20.0)
        .max_width(35.0);
    let (limited, truncated) = long.fit_lines(1, &fonts)?;
    assert!(truncated);
    assert!(
        !limited
            .layout(&fonts)?
            .text
            .trim_end_matches('\u{2026}')
            .ends_with('e')
    );
    Ok(())
}

#[test]
fn explicit_unicode_linebreaks_keep_cjk_punctuation_and_graphemes() -> motion_core::Result {
    let mut fonts = fixture();
    fonts.load_file(
        std::env::var_os("MOTION_TEST_FALLBACK_FONT").unwrap_or_else(|| {
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/fonts/NotoSansCJKsc-Regular.otf"
            )
            .into()
        }),
    )?;
    let text = Text::new(
        "\u{4e2d}\u{6587}\u{ff08}\u{6392}\u{7248}\u{ff09}\u{ff0c}\u{79d1}\u{5b66}\u{3002}",
    )
    .size(20.0)
    .max_width(100.0);
    let layout = text.layout(&fonts)?;
    assert!(layout.lines().len() > 1);
    for range in layout.line_sources.iter() {
        let line = &layout.text[range.clone()];
        assert!(!line.chars().next().is_some_and(|character| {
            ['\u{ff09}', '\u{ff0c}', '\u{3002}'].contains(&character)
        }));
        assert!(!line.ends_with('\u{ff08}'));
    }
    let grapheme = Text::rich([
        TextSpan::new("e"),
        TextSpan::new("\u{301}").fill(Color::BLACK),
    ])
    .size(20.0)
    .max_width(100.0);
    assert_eq!(
        grapheme.layout(&fonts)?.parts(TextSplit::Characters).len(),
        1
    );
    Ok(())
}

fn fixture() -> FontLibrary {
    let mut fonts = FontLibrary::new();
    fonts
        .load_file(std::env::var_os("MOTION_TEST_FONT").unwrap_or_else(|| {
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/fonts/NotoSans-Regular.ttf"
            )
            .into()
        }))
        .unwrap();
    fonts
}

#[test]
#[ignore = "requires explicit MOTION_TEST_FONT; no font discovery"]
fn explicit_paragraph_rows_parts_and_relative_layout_use_actual_baselines() -> motion_core::Result {
    let fonts = fixture();
    let resources = motion_core::Assets::new(".")?.with_fonts(fonts.clone());
    let cache = motion_core::SceneCache::new(0);
    let context = motion_core::SceneContext::new(
        Time::ZERO,
        motion_core::FrameRate::from(30),
        motion_core::Size::new(128.0, 64.0),
        &cache,
    )
    .with_assets(&resources);
    let large = Text::new("A").size(24.0).fonts(&fonts);
    let small = Text::new("A").size(12.0).fonts(&fonts);
    let animated =
        large.try_animate_parts_at(&context, motion_core::Duration::ZERO, |_| PartStyle::new())?;
    let row = motion_core::Row::new()
        .alignment(motion_core::Alignment::Baseline)
        .gap(4.0)
        .children([animated, Node::from(small.clone())]);
    let plain = motion_core::Row::new()
        .alignment(motion_core::Alignment::Baseline)
        .gap(4.0)
        .children([Node::from(large.clone()), Node::from(small.clone())]);
    assert_eq!(context.measure(&row)?, context.measure(&plain)?);
    let actual = row.compile_with_fonts(
        context.size(),
        motion_core::PixelSize::new(128, 64),
        motion_core::ValidationLimits::default(),
        &fonts,
    )?;
    let expected = plain.compile_with_fonts(
        context.size(),
        motion_core::PixelSize::new(128, 64),
        motion_core::ValidationLimits::default(),
        &fonts,
    )?;
    assert_eq!(actual.commands, expected.commands);
    let baselines = actual
        .commands
        .iter()
        .filter_map(|command| {
            if let motion_core::DrawCommand::Fill { transform, .. } = command {
                Some(transform.apply(motion_core::Point::ORIGIN).y)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(baselines.len(), 2);
    assert_eq!(baselines[0], baselines[1]);
    let hidden = large.clone().visible_parts(0).try_animate_parts_at(
        &context,
        motion_core::Duration::ZERO,
        |_| PartStyle::new(),
    )?;
    let hidden_row = motion_core::Row::new()
        .alignment(motion_core::Alignment::Baseline)
        .children([hidden, Node::from(small.clone())]);
    context.measure(&hidden_row)?;
    let title = plain.at(context.center());
    let subtitle = context.try_next_to(
        Node::from(small.clone()),
        &title,
        motion_core::Side::Bottom,
        7.0,
    )?;
    let title_bounds = context.measure(&title)?;
    let subtitle_bounds = context.measure(&subtitle)?;
    assert!((subtitle_bounds.top - title_bounds.bottom - 7.0).abs() < 1e-12);
    assert!((subtitle_bounds.center().x - title_bounds.center().x).abs() < 1e-12);
    let left = context.try_align_to(subtitle, &title, motion_core::Side::Left)?;
    assert!((context.measure(&left)?.left - title_bounds.left).abs() < 1e-12);
    let reflected = motion_core::Row::new()
        .alignment(motion_core::Alignment::Baseline)
        .children([Node::from(small).scale_xy(1.0, -1.0)]);
    assert!(context.measure(&reflected).is_err());
    println!(
        "paragraph layout: actual mixed-size/parts/hidden baselines, relative positioning and reflection diagnostics=pass"
    );
    Ok(())
}

#[test]
fn explicit_paragraph_fonts_shaping_fallback_metrics_parts_and_replay() {
    let mut fonts = fixture();
    fonts
        .load_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/fonts/NotoSansHebrew-Regular.ttf"
        ))
        .unwrap();
    let first = fonts.clone();
    fonts
        .load_file(
            std::env::var_os("MOTION_TEST_FALLBACK_FONT").unwrap_or_else(|| {
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/fonts/NotoSansCJKsc-Regular.otf"
                )
                .into()
            }),
        )
        .unwrap();
    assert_ne!(fonts.snapshot_id(), first.snapshot_id());
    let text = Text::rich([
        TextSpan::new("office e\u{301} ").fill(Color::rgb(240, 40, 60)),
        TextSpan::new("中文 ").size(20.0),
        TextSpan::new("אבג\r\nsecond line wraps naturally").fill(Color::rgb(20, 180, 90)),
    ])
    .size(16.0)
    .max_width(122.0)
    .fonts(&fonts);
    let layout = text.layout(&fonts).unwrap();
    assert!(layout.lines().len() >= 3);
    assert!(layout.baseline() > 0.0 && layout.baseline() < layout.lines()[0].bottom);
    assert!(
        layout
            .glyphs
            .iter()
            .all(|glyph| layout.text.get(glyph.source.clone()).is_some())
    );
    let latin_family = &layout
        .glyphs
        .iter()
        .find(|glyph| layout.text[glyph.source.clone()].contains('o'))
        .unwrap()
        .family;
    assert!(
        layout
            .glyphs
            .iter()
            .any(|glyph| layout.text[glyph.source.clone()].contains('中')
                && &glyph.family != latin_family)
    );
    assert!(
        layout
            .glyphs
            .windows(2)
            .any(|glyphs| glyphs[0].source.start > glyphs[1].source.start)
    );
    assert!(
        layout
            .glyphs
            .iter()
            .any(|glyph| &layout.text[glyph.source.clone()] == "s"
                && glyph.source.start > layout.text.find("\r\n").unwrap())
    );
    assert!(Arc::ptr_eq(
        &layout,
        &text
            .clone()
            .fill(Color::WHITE)
            .visible_parts(2)
            .layout(&fonts)
            .unwrap()
    ));
    assert!(Arc::ptr_eq(
        &Text::new("same content").layout(&fonts).unwrap(),
        &Text::new("same content")
            .fill(Color::BLACK)
            .layout(&fonts)
            .unwrap()
    ));
    assert!(!Arc::ptr_eq(
        &layout,
        &text.clone().size(18.0).layout(&fonts).unwrap()
    ));
    let characters = layout.parts(TextSplit::Characters);
    assert!(
        characters
            .iter()
            .any(|part| &layout.text[part.source.clone()] == "e\u{301}")
    );
    for glyph in layout.glyphs.iter() {
        assert_eq!(characters.iter().filter(|part| part.owns(glyph)).count(), 1);
    }
    assert_eq!(layout.parts(TextSplit::Lines).len(), layout.lines.len());
    let blank_lines = Text::new("A\r\n\r\nB").layout(&fonts).unwrap();
    let blank_parts = blank_lines.parts(TextSplit::Lines);
    assert_eq!(blank_parts[1].source, 3..3);
    assert_eq!(blank_parts[2].source, 5..6);
    let resources = motion_core::Assets::new(".")
        .unwrap()
        .with_fonts(fonts.clone());
    let cache = motion_core::SceneCache::new(0);
    let context = motion_core::SceneContext::new(
        250.milliseconds().after_origin(),
        motion_core::FrameRate::from(30),
        motion_core::Size::new(128.0, 112.0),
        &cache,
    )
    .with_assets(&resources);
    assert_eq!(
        context.measure(&Node::from(text.clone())).unwrap(),
        layout.bounds()
    );
    let probe = std::cell::RefCell::new(Vec::new());
    text.try_animate_parts_at(&context, 100.milliseconds(), |part| {
        probe.borrow_mut().push(part.time());
        PartStyle::new()
    })
    .unwrap();
    assert_eq!(probe.borrow()[0], 250.milliseconds().after_origin());
    assert_eq!(probe.borrow()[3], (-50.milliseconds()).after_origin());
    let composition =
        motion_render::Composition::new(128, 112, motion_render::FrameClock::new(30, 12))
            .with_options(
                motion_render::RenderOptions::default()
                    .background(motion_core::Color::BLACK)
                    .resources(resources.clone()),
            )
            .unwrap();
    let build = |context: &motion_core::SceneContext<'_>| {
        text.try_animate_parts_at(context, 20.milliseconds(), |part| {
            PartStyle::new()
                .offset(
                    0.0,
                    (1.0 - part.sample(&motion_core::Tween::new(0.0, 1.0, 200.milliseconds())))
                        * 3.0,
                )
                .opacity(part.sample(&motion_core::Tween::new(0.0, 1.0, 200.milliseconds())))
        })
        .unwrap()
    };
    let initial = composition.render_frame(0, context.size(), build).unwrap();
    assert!(initial.packed_rgb().iter().all(|pixel| *pixel == 0));
    let later = composition.render_frame(10, context.size(), build).unwrap();
    assert!(later.packed_rgb().iter().any(|pixel| *pixel != 0));
    let replay = composition.render_frame(10, context.size(), build).unwrap();
    assert_eq!(later.packed_rgb(), replay.packed_rgb());
    let hidden = composition
        .render_frame(1, context.size(), |_| text.clone().visible_parts(0).into())
        .unwrap();
    assert!(hidden.packed_rgb().iter().all(|pixel| *pixel == 0));
    println!(
        "paragraph: faces={} lines={} glyphs={} graphemes={} bidi/fallback/baseline/cache/stagger/visibility/replay=pass",
        fonts.len(),
        layout.lines.len(),
        layout.glyphs.len(),
        characters.len()
    );
}

#[test]
#[ignore = "requires explicit MOTION_TEST_FONT and MOTION_TEST_EMOJI_FONT; no discovery"]
fn explicit_bitmap_emoji_uses_image_and_shared_paragraph() {
    let mut fonts = fixture();
    fonts
        .load_file(std::env::var_os("MOTION_TEST_EMOJI_FONT").expect("explicit emoji font"))
        .unwrap();
    let text = Text::new("A 😀 👩‍💻").size(28.0).fonts(&fonts);
    let layout = text.layout(&fonts).unwrap();
    assert!(
        layout
            .glyphs
            .iter()
            .any(|glyph| glyph.artwork.bitmap.is_some())
    );
    assert_eq!(layout.parts(TextSplit::Characters).len(), 5);
    let frame = motion_render::Composition::new(128, 64, motion_render::FrameClock::new(30, 150))
        .render_frame(0, motion_core::Size::new(128.0, 64.0), |_| {
            text.clone().translate(4.0, 4.0)
        })
        .unwrap();
    assert!(
        frame
            .packed_rgb()
            .iter()
            .filter(|pixel| (((**pixel >> 16) & 255) as i32 - (**pixel & 255) as i32).abs() > 30)
            .count()
            > 30
    );
    println!(
        "bitmap emoji: glyphs={} color pixels/replayable layout=pass",
        layout.glyphs.len()
    );
}

fn with_tables(bytes: &[u8], additions: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    let face = ttf_parser::Face::parse(bytes, 0).unwrap();
    let mut tables = BTreeMap::new();
    for record in face.raw_face().table_records {
        tables.insert(
            record.tag.0.to_be_bytes(),
            face.raw_face().table(record.tag).unwrap().to_vec(),
        );
    }
    for (tag, data) in additions {
        tables.insert(tag, data);
    }
    tables.get_mut(b"head").unwrap()[8..12].fill(0);
    let count = tables.len() as u16;
    let power = 1u16 << count.ilog2();
    let mut output = bytes[..4].to_vec();
    for number in [
        count,
        power * 16,
        power.ilog2() as u16,
        count * 16 - power * 16,
    ] {
        output.extend(number.to_be_bytes());
    }
    output.resize(12 + usize::from(count) * 16, 0);
    let checksum = |data: &[u8]| {
        data.chunks(4).fold(0u32, |sum, chunk| {
            let mut bytes = [0; 4];
            bytes[..chunk.len()].copy_from_slice(chunk);
            sum.wrapping_add(u32::from_be_bytes(bytes))
        })
    };
    let mut head = 0;
    for (index, (tag, data)) in tables.into_iter().enumerate() {
        while !output.len().is_multiple_of(4) {
            output.push(0);
        }
        let start = output.len();
        if tag == *b"head" {
            head = start;
        }
        let record = 12 + index * 16;
        output[record..record + 4].copy_from_slice(&tag);
        output[record + 4..record + 8].copy_from_slice(&checksum(&data).to_be_bytes());
        output[record + 8..record + 12].copy_from_slice(&(start as u32).to_be_bytes());
        output[record + 12..record + 16].copy_from_slice(&(data.len() as u32).to_be_bytes());
        output.extend(data);
    }
    let adjustment = 0xB1B0AFBAu32.wrapping_sub(checksum(&output));
    output[head + 8..head + 12].copy_from_slice(&adjustment.to_be_bytes());
    output
}

#[test]
#[ignore = "requires explicit MOTION_TEST_FONT; adds small COLRv0 and CBDT tables only in memory"]
fn explicit_colrv0_and_cbdt_are_actual_color_glyphs() {
    use image::ImageEncoder;
    let bytes = std::fs::read(std::env::var_os("MOTION_TEST_FONT").unwrap()).unwrap();
    let face = ttf_parser::Face::parse(&bytes, 0).unwrap();
    let glyph = face.glyph_index('A').unwrap().0;
    let other = face.glyph_index('B').unwrap().0;
    let mut colr = Vec::new();
    colr.extend(0u16.to_be_bytes());
    colr.extend(1u16.to_be_bytes());
    colr.extend(14u32.to_be_bytes());
    colr.extend(20u32.to_be_bytes());
    colr.extend(2u16.to_be_bytes());
    for number in [glyph, 0, 2, glyph, 0, other, 1] {
        colr.extend(number.to_be_bytes());
    }
    let mut cpal = Vec::new();
    for number in [0u16, 2, 1, 2] {
        cpal.extend(number.to_be_bytes());
    }
    cpal.extend(14u32.to_be_bytes());
    cpal.extend(0u16.to_be_bytes());
    cpal.extend([0, 0, 255, 255, 0, 255, 0, 255]);
    let mut fonts = FontLibrary::new();
    fonts
        .load_data(with_tables(&bytes, vec![(*b"COLR", colr), (*b"CPAL", cpal)]).into())
        .unwrap();
    let text = Text::new("A").size(30.0).fonts(&fonts);
    assert_eq!(
        text.layout(&fonts).unwrap().glyphs[0].artwork.layers.len(),
        2
    );
    let frame = motion_render::Composition::new(48, 48, motion_render::FrameClock::new(30, 150))
        .with_options(motion_render::RenderOptions::default().background(motion_core::Color::BLACK))
        .unwrap()
        .render_frame(0, motion_core::Size::new(48.0, 48.0), |_| {
            text.clone().into()
        })
        .unwrap();
    assert!(
        frame
            .packed_rgb()
            .iter()
            .any(|pixel| pixel & 0x00ff00 > 0x008000)
    );
    assert!(
        frame
            .packed_rgb()
            .iter()
            .any(|pixel| pixel & 0xff0000 > 0x800000)
    );
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &[255, 20, 40, 255].repeat(64),
            8,
            8,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    let mut cbdt = 0x00030000u32.to_be_bytes().to_vec();
    cbdt.extend([8, 8, 0, 8, 8]);
    cbdt.extend((png.len() as u32).to_be_bytes());
    cbdt.extend(png);
    let mut cblc = vec![0u8; 80];
    cblc[..4].copy_from_slice(&0x00030000u32.to_be_bytes());
    cblc[4..8].copy_from_slice(&1u32.to_be_bytes());
    cblc[8..12].copy_from_slice(&56u32.to_be_bytes());
    cblc[12..16].copy_from_slice(&24u32.to_be_bytes());
    cblc[16..20].copy_from_slice(&1u32.to_be_bytes());
    cblc[48..50].copy_from_slice(&glyph.to_be_bytes());
    cblc[50..52].copy_from_slice(&glyph.to_be_bytes());
    cblc[52..56].copy_from_slice(&[32, 32, 32, 1]);
    cblc[56..58].copy_from_slice(&glyph.to_be_bytes());
    cblc[58..60].copy_from_slice(&glyph.to_be_bytes());
    cblc[60..64].copy_from_slice(&8u32.to_be_bytes());
    cblc[64..66].copy_from_slice(&1u16.to_be_bytes());
    cblc[66..68].copy_from_slice(&17u16.to_be_bytes());
    cblc[68..72].copy_from_slice(&4u32.to_be_bytes());
    cblc[76..80].copy_from_slice(&((cbdt.len() - 4) as u32).to_be_bytes());
    let mut fonts = FontLibrary::new();
    fonts
        .load_data(with_tables(&bytes, vec![(*b"CBLC", cblc), (*b"CBDT", cbdt)]).into())
        .unwrap();
    let text = Text::new("A").size(32.0).fonts(&fonts);
    assert!(
        text.layout(&fonts).unwrap().glyphs[0]
            .artwork
            .bitmap
            .is_some()
    );
    let frame = motion_render::Composition::new(48, 48, motion_render::FrameClock::new(30, 150))
        .with_options(motion_render::RenderOptions::default().background(motion_core::Color::BLACK))
        .unwrap()
        .render_frame(0, motion_core::Size::new(48.0, 48.0), |_| {
            text.clone().into()
        })
        .unwrap();
    assert!(
        frame
            .packed_rgb()
            .iter()
            .filter(|pixel| **pixel >> 16 > 200 && (**pixel & 255) < 70)
            .count()
            >= 40
    );
    println!("COLRv0 layers=2 and CBDT PNG strike=8x8, colored pixels=pass");
}

#[test]
#[cfg(feature = "svg")]
#[ignore = "requires explicit MOTION_TEST_FONT; no system discovery"]
fn explicit_fonts_project_memory_svg_and_snapshot_isolation() {
    let bytes: Arc<[u8]> = std::fs::read(std::env::var_os("MOTION_TEST_FONT").unwrap())
        .unwrap()
        .into();
    let mut fonts = FontLibrary::new();
    fonts.load_data(bytes.clone()).unwrap();
    let snapshot = fonts.clone();
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("project.ttf"), &bytes).unwrap();
    assert_eq!(fonts.load_dir(directory.path()).unwrap(), 1);
    assert_eq!(snapshot.len(), 1);
    assert_eq!(fonts.len(), 2);
    assert_ne!(fonts.snapshot_id(), snapshot.snapshot_id());
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="32"><text x="0" y="24" font-family="Arial" font-size="24">Shared</text></svg>"#;
    assert!(crate::SvgAsset::from_bytes_with_fonts(svg, &FontLibrary::new()).is_err());
    let asset = Arc::new(crate::SvgAsset::from_bytes_with_fonts(svg, &snapshot).unwrap());
    let rendered = crate::Svg::from_asset(asset)
        .try_node()
        .unwrap()
        .compile(
            motion_core::Size::new(96.0, 32.0),
            motion_core::PixelSize::new(96, 32),
            motion_core::ValidationLimits::default(),
        )
        .unwrap()
        .prepare(
            motion_core::ValidationLimits::default(),
            motion_core::RasterLimits::default(),
        )
        .unwrap()
        .render_linear(motion_core::RasterLimits::default())
        .unwrap();
    assert!(rendered.linear_pixels().any(|pixel| pixel[3] > 0.5));
    assert!(
        Text::new("Shared")
            .layout(&snapshot)
            .unwrap()
            .bounds
            .width()
            > 0.0
    );
}

#[test]
#[ignore = "requires explicit MOTION_TEST_FONT; geometry only, no discovery"]
fn explicit_underlined_run_is_geometry_and_replays() -> motion_core::Result {
    let mut fonts = FontLibrary::new();
    fonts.load_file(std::env::var("MOTION_TEST_FONT")?)?;
    let text = Text::new("Underlined").size(24.0).fonts(&fonts);
    let layout = text.layout(&fonts)?;
    let underline = motion_core::Rect::new(
        0.0,
        layout.baseline() + 2.0,
        layout.lines()[0].width(),
        layout.baseline() + 3.0,
    );
    let draw = DrawContext::default().with_fonts(fonts);
    let glyphs = text.vector_tree(&draw)?.node();
    let node = Node::group([glyphs, Node::from(underline.fill(Color::WHITE))]);
    let render = || -> motion_core::Result<motion_render::FrameBuffer> {
        Ok(node
            .compile(
                motion_core::Size::new(160.0, 48.0),
                motion_core::PixelSize::new(160, 48),
                motion_core::ValidationLimits::default(),
            )?
            .prepare(
                motion_core::ValidationLimits::default(),
                motion_core::RasterLimits::default(),
            )?
            .render_linear(motion_core::RasterLimits::default())?)
    };
    let frame = render()?;
    assert_eq!(frame, render()?);
    let area = frame
        .linear_pixels()
        .enumerate()
        .filter(|(index, pixel)| {
            let position =
                motion_core::Point::new((index % 160) as f64 + 0.5, (index / 160) as f64 + 0.5);
            underline.contains(position) && pixel[3] > 0.0
        })
        .count();
    assert!(area >= layout.lines()[0].width().floor() as usize);
    assert!(
        node.measure_with(&draw, motion_core::ValidationLimits::default())?
            .bottom
            >= underline.bottom
    );
    Ok(())
}

#[test]
#[cfg(feature = "gpu")]
#[ignore = "requires native GPU and explicit MOTION_TEST_FONT/FALLBACK_FONT/EMOJI_FONT"]
fn explicit_gpu_paragraphs_and_parts_match_cpu_and_replay() {
    let mut fonts = fixture();
    fonts
        .load_file(std::env::var_os("MOTION_TEST_FALLBACK_FONT").unwrap())
        .unwrap();
    fonts
        .load_file(std::env::var_os("MOTION_TEST_EMOJI_FONT").unwrap())
        .unwrap();
    let resources = motion_core::Assets::new(".")
        .unwrap()
        .with_fonts(fonts.clone());
    let cache = motion_core::SceneCache::new(0);
    let design = motion_core::Size::new(128.0, 96.0);
    let validation = motion_core::ValidationLimits::default();
    let limits = motion_core::RasterLimits::default();
    let mut gpu = motion_render::gpu::Gpu::new().unwrap();
    let text = Text::rich([
        TextSpan::new("Hi 中文 ").fill(Color::rgb(235, 30, 75)),
        TextSpan::new("אבג\n😀"),
    ])
    .size(18.0)
    .max_width(110.0);
    let mut difference = 0.0f32;
    let mut bytes = 0u8;
    let mut cases = 0;
    for time in [
        500.milliseconds(),
        motion_core::Duration::ZERO,
        150.milliseconds(),
        500.milliseconds(),
    ] {
        let context = motion_core::SceneContext::new(
            time.after_origin(),
            motion_core::FrameRate::from(30),
            design,
            &cache,
        )
        .with_assets(&resources);
        let nodes = [
            Node::from(text.clone()),
            Node::from(text.clone().stroke(0.4, Color::BLACK))
                .scale_xy(-0.9, 1.1)
                .translate(4.0, 0.0),
            text.clone()
                .fonts(&fonts)
                .try_animate_parts_at(&context, 20.milliseconds(), |part| {
                    let progress =
                        part.sample(&motion_core::Tween::new(0.0, 1.0, 200.milliseconds()));
                    PartStyle::new()
                        .scale(0.7 + progress * 0.3)
                        .offset(0.0, 4.0 * (1.0 - progress))
                        .opacity(progress)
                })
                .unwrap(),
        ];
        for node in nodes {
            let frame = node
                .compile_with_fonts(
                    design,
                    motion_core::PixelSize::new(128, 96),
                    validation,
                    &fonts,
                )
                .unwrap()
                .prepare(validation, limits)
                .unwrap();
            let cpu = frame.render_linear(limits).unwrap();
            let actual = gpu.render_linear(&frame, limits).unwrap();
            for (pixel, (left, right)) in
                actual.linear_pixels().zip(cpu.linear_pixels()).enumerate()
            {
                for (channel, (left, right)) in left.iter().zip(right).enumerate() {
                    difference = difference.max((left - right).abs());
                    assert!(
                        (left - right).abs() <= 4e-6,
                        "case={cases} time={time:?} pixel={pixel} channel={channel} GPU={left} CPU={right} delta={}",
                        (left - right).abs()
                    );
                }
            }
            let actual = actual
                .to_format(
                    motion_render::PixelFormat::SrgbRgba8 {
                        premultiplied: false,
                    },
                    limits,
                )
                .unwrap();
            let expected = cpu
                .to_format(
                    motion_render::PixelFormat::SrgbRgba8 {
                        premultiplied: false,
                    },
                    limits,
                )
                .unwrap();
            for (left, right) in actual.bytes().iter().zip(expected.bytes()) {
                bytes = bytes.max(left.abs_diff(*right));
                assert!(left.abs_diff(*right) <= 1);
            }
            let replay = gpu
                .render_linear(&frame, limits)
                .unwrap()
                .to_format(
                    motion_render::PixelFormat::SrgbRgba8 {
                        premultiplied: false,
                    },
                    limits,
                )
                .unwrap();
            assert_eq!(actual, replay);
            cases += 1;
        }
    }
    println!(
        "paragraph GPU: adapter={} backend={:?} cases={cases} max_linear={difference:e} max_rgba={bytes} replay=pass",
        gpu.info().name,
        gpu.info().backend
    );
}
