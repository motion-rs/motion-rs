//! Integration tests for shapes, strokes and paint rendered through the facade.

mod drawing {

    mod drawing_stroke {
        use motion::extend::*;
        use motion::prelude::*;
        use motion::render::*;

        #[test]
        fn caps_have_analytic_bounds_and_coverage_without_reference_layers() {
            for cap in [LineCap::Butt, LineCap::Round, LineCap::Square] {
                let style = StrokeStyle {
                    width: 2.0,
                    cap,
                    ..StrokeStyle::default()
                };
                let outline = style.outline(&line(), "test").unwrap();
                let expected = if cap == LineCap::Butt {
                    Rect::new(1.0, 0.0, 5.0, 2.0)
                } else {
                    Rect::new(0.0, 0.0, 6.0, 2.0)
                };
                assert_eq!(outline.bounding_box(), expected);
                let frame = render(line().stroke(2.0, Color::WHITE).stroke_style(style));
                assert_eq!(frame.linear_pixel(2, 0).unwrap(), [1.0; 4]);
                let expected = match cap {
                    LineCap::Butt => 0.0,
                    LineCap::Square => 1.0,
                    LineCap::Round => std::f64::consts::FRAC_PI_4,
                };
                assert!(
                    (f64::from(frame.linear_pixel(0, 0).unwrap()[3]) - expected).abs()
                        <= 1.0 / 255.0,
                    "{cap:?}"
                );
            }
        }

        #[test]
        fn closed_rectangles_have_holes_joins_and_no_caps() {
            let source = Path2D::from_rect(Rect::new(1.0, 1.0, 7.0, 7.0)).unwrap();
            for join in [
                LineJoin::Miter { limit: 4.0 },
                LineJoin::Miter { limit: 1.0 },
                LineJoin::Round,
                LineJoin::Bevel,
            ] {
                let style = StrokeStyle {
                    width: 2.0,
                    join,
                    ..StrokeStyle::default()
                };
                let frame = render(source.clone().stroke(2.0, Color::WHITE).stroke_style(style));
                assert_eq!(frame.linear_pixel(4, 4).unwrap(), [0.0; 4]);
                assert_eq!(frame.linear_pixel(0, 4).unwrap(), [1.0; 4]);
                let expected = match join {
                    LineJoin::Miter { limit: 4.0 } => 1.0,
                    LineJoin::Round => std::f64::consts::FRAC_PI_4,
                    _ => 0.5,
                };
                assert!(
                    (f64::from(frame.linear_pixel(0, 0).unwrap()[3]) - expected).abs()
                        <= 1.0 / 255.0,
                    "{join:?}"
                );
                let square = StrokeStyle {
                    cap: LineCap::Square,
                    ..style
                }
                .outline(&source, "test")
                .unwrap();
                assert_eq!(style.outline(&source, "test").unwrap(), square);
            }
        }

        #[test]
        fn filling_then_stroking_is_isolated_once_and_style_replacement_is_local() {
            let rect = Rect::new(1.0, 1.0, 7.0, 7.0);
            let original = rect.fill(Color::WHITE);
            let stroked = original
                .clone()
                .stroke(2.0, Color::WHITE)
                .fill_rule(FillRule::EvenOdd);
            let commands = DrawContext::default()
                .record(&stroked, ValidationLimits::default())
                .unwrap();
            assert!(matches!(commands[0], DrawCommand::FillRect { .. }));
            assert!(matches!(
                commands[1],
                DrawCommand::Fill {
                    rule: FillRule::NonZero,
                    ..
                }
            ));
            assert_eq!(
                render(Node::from(stroked.clone()).opacity(0.5))
                    .linear_pixel(1, 1)
                    .unwrap(),
                [0.5; 4]
            );
            assert_eq!(
                DrawContext::default()
                    .record(&original, ValidationLimits::default())
                    .unwrap()
                    .len(),
                1
            );
            let opaque = render(rect.stroke(2.0, Color::WHITE).fill(Color::WHITE));
            assert_eq!(opaque.linear_pixel(4, 4).unwrap(), [1.0; 4]);
            assert!(
                Node::from(stroked)
                    .compile(
                        Size::new(8.0, 8.0),
                        PixelSize::new(8, 8),
                        ValidationLimits::default()
                    )
                    .is_ok()
            );
            assert_eq!(
                StrokeStyle::default(),
                StrokeStyle {
                    width: 1.0,
                    cap: LineCap::Butt,
                    join: LineJoin::Miter { limit: 4.0 }
                }
            );
            let style = StrokeStyle {
                width: 2.0,
                cap: LineCap::Square,
                join: LineJoin::Bevel,
            };
            let replaced = rect
                .stroke(5.0, Color::BLACK)
                .stroke_style(style)
                .stroke(2.0, Color::WHITE);
            let expected = rect.stroke(2.0, Color::WHITE).stroke_style(style);
            assert_eq!(
                DrawContext::default()
                    .record(&replaced, ValidationLimits::default())
                    .unwrap(),
                DrawContext::default()
                    .record(&expected, ValidationLimits::default())
                    .unwrap()
            );
            assert_eq!(
                Node::from(expected)
                    .measure(ValidationLimits::default())
                    .unwrap(),
                Rect::new(
                    1.0 - std::f64::consts::SQRT_2,
                    1.0 - std::f64::consts::SQRT_2,
                    7.0 + std::f64::consts::SQRT_2,
                    7.0 + std::f64::consts::SQRT_2,
                )
            );
            let fill_only = original.stroke_style(StrokeStyle {
                width: f64::NAN,
                ..style
            });
            assert_eq!(
                DrawContext::default()
                    .record(&fill_only, ValidationLimits::default())
                    .unwrap()
                    .len(),
                1
            );
        }

        #[test]
        fn transforms_and_clips_apply_to_the_completed_outline_and_color_stays() {
            let style = StrokeStyle {
                width: 2.0,
                cap: LineCap::Round,
                join: LineJoin::Bevel,
            };
            let color = Color::from_rgb_float(1.25, -0.1, 0.5).with_alpha(0.7);
            let actual = Node::from(line().stroke(2.0, color).stroke_style(style))
                .pivot(motion::Pivot::Point(Point::ORIGIN))
                .scale_xy(1.25, 2.0)
                .rotate(0.2)
                .translate(1.0, 1.0)
                .clip_with(Rect::new(1.0, 0.0, 5.0, 2.0), motion::ClipMode::Geometric)
                .opacity(0.6);
            let expected = Node::from(style.outline(&line(), "test").unwrap().fill(color))
                .pivot(motion::Pivot::Point(Point::ORIGIN))
                .scale_xy(1.25, 2.0)
                .rotate(0.2)
                .translate(1.0, 1.0)
                .clip_with(Rect::new(1.0, 0.0, 5.0, 2.0), motion::ClipMode::Geometric)
                .opacity(0.6);
            assert_eq!(render(actual), render(expected));
            let commands = DrawContext::default()
                .record(&line().stroke(2.0, color), ValidationLimits::default())
                .unwrap();
            assert!(
                matches!(commands[0],DrawCommand::Fill {paint:motion::Paint::Solid(paint),..} if paint==color)
            );
        }

        fn line() -> Path2D {
            let mut path = Path2D::new();
            path.try_move_to(Point::new(1.0, 1.0)).unwrap();
            path.try_line_to(Point::new(5.0, 1.0)).unwrap();
            path
        }

        #[test]
        fn curves_dashes_growth_and_cusps_use_fill_outlines() {
            let curve = Path2D::parse_svg("M1 1Q3 1 5 1").unwrap();
            let cusp = Path2D::parse_svg("M1 4C7 4 -1 4 5 4").unwrap();
            for cap in [LineCap::Butt, LineCap::Round, LineCap::Square] {
                for join in [
                    LineJoin::Miter { limit: 4.0 },
                    LineJoin::Round,
                    LineJoin::Bevel,
                ] {
                    let style = StrokeStyle {
                        width: 2.0,
                        cap,
                        join,
                    };
                    let curve_frame =
                        render(curve.clone().stroke(2.0, Color::WHITE).stroke_style(style));
                    let reference = render(line().stroke(2.0, Color::WHITE).stroke_style(style));
                    for row in 0..8 {
                        for column in 0..8 {
                            assert!(
                                (curve_frame.linear_pixel(column, row).unwrap()[3]
                                    - reference.linear_pixel(column, row).unwrap()[3])
                                    .abs()
                                    <= 1.0 / 255.0
                            );
                        }
                    }
                    let outline = style.outline(&cusp, "test").unwrap();
                    assert!(outline.bounding_box().width().is_finite());
                    assert!(!outline.verbs().is_empty());
                }
            }
            let original = Path2D::parse_svg("M1 4C1 1 7 1 7 4")
                .unwrap()
                .stroke(1.0, Color::WHITE);
            let growing = original.clone().trim(0.0, 0.5).dash(&[1.0, 0.5], -0.25);
            let commands = DrawContext::default()
                .record(&growing, ValidationLimits::default())
                .unwrap();
            assert!(commands.iter().all(|command| matches!(
                command,
                DrawCommand::Fill {
                    rule: FillRule::NonZero,
                    ..
                }
            )));
            let empty = original.clone().trim(0.0, 0.0);
            assert!(
                DrawContext::default()
                    .record(&empty, ValidationLimits::default())
                    .unwrap()
                    .is_empty()
            );
            assert!(
                !DrawContext::default()
                    .record(&original, ValidationLimits::default())
                    .unwrap()
                    .is_empty()
            );
            assert!(
                DrawContext::default()
                    .record(
                        &original.clone().dash(&[0.0], 0.0),
                        ValidationLimits::default()
                    )
                    .is_err()
            );
            assert!(
                DrawContext::default()
                    .record(&original.trim(f64::NAN, 1.0), ValidationLimits::default())
                    .is_err()
            );
            let dashed = render(line().stroke(2.0, Color::WHITE).dash(&[2.0, 2.0], 0.0));
            assert_eq!(dashed.linear_pixel(1, 0).unwrap()[3], 1.0);
            assert_eq!(dashed.linear_pixel(3, 0).unwrap()[3], 0.0);
            let grown = render(line().stroke(2.0, Color::WHITE).trim(0.0, 0.5));
            assert_eq!(grown.linear_pixel(1, 0).unwrap()[3], 1.0);
            assert_eq!(grown.linear_pixel(3, 0).unwrap()[3], 0.0);
            let point = Path2D::parse_svg("M4 4C4 4 4 4 4 4").unwrap();
            assert!(
                StrokeStyle {
                    width: 2.0,
                    cap: LineCap::Round,
                    join: LineJoin::Round
                }
                .outline(&point, "test")
                .unwrap()
                .verbs()
                .is_empty()
            );
            let mixed = Path2D::parse_svg("M4 5C4 5 4 5 4 5 M1 1L5 1").unwrap();
            let mixed = render(mixed.stroke(2.0, Color::WHITE).stroke_style(StrokeStyle {
                width: 2.0,
                cap: LineCap::Round,
                join: LineJoin::Round,
            }));
            assert_eq!(mixed.linear_pixel(4, 5).unwrap()[3], 0.0);
            assert!(
                StrokeStyle {
                    width: 1e-40,
                    ..StrokeStyle::default()
                }
                .outline(&point, "test")
                .is_err()
            );
        }

        fn render(node: impl Into<Node>) -> FrameBuffer {
            node.into()
                .compile(
                    Size::new(8.0, 8.0),
                    PixelSize::new(8, 8),
                    ValidationLimits::default(),
                )
                .unwrap()
                .prepare(ValidationLimits::default(), RasterLimits::default())
                .unwrap()
                .render_linear(RasterLimits::default())
                .unwrap()
        }

        #[test]
        #[ignore = "requires a native GPU; no window or video export"]
        fn stroke_gpu_matches_cpu_and_replays() -> motion::Result {
            use motion::Color;
            use motion::render::Composition;
            use motion::render::Renderer;
            let composition = Composition::new(16, 16, motion::render::FrameClock::new(30, 12))
                .with_options(motion::render::RenderOptions::default().background(Color::BLACK))?;
            let mut cpu = composition.frame_renderer(Size::new(8.0, 8.0))?;
            let mut gpu = composition
                .configure(|options| options.backend(Renderer::Gpu.into()))
                .frame_renderer(Size::new(8.0, 8.0))?;
            let build = |context: &motion::SceneContext<'_>| {
                let index = context.time().to_frame(context.frame_rate());
                let cap = [LineCap::Butt, LineCap::Round, LineCap::Square][index as usize % 3];
                let join = [
                    LineJoin::Miter { limit: 4.0 },
                    LineJoin::Round,
                    LineJoin::Bevel,
                    LineJoin::Miter { limit: 1.0 },
                ][index as usize / 3];
                let path =
                    Path2D::from_rect(Rect::new(1.0, 1.0, 6.0, 5.0)).expect("fixed rectangle");
                Node::group([
                    Node::from(
                        path.fill(Color::from_hex(0x467c93))
                            .stroke(1.0, Color::WHITE)
                            .stroke_style(StrokeStyle {
                                width: 1.0,
                                cap,
                                join,
                            }),
                    ),
                    line()
                        .stroke(1.5, Color::from_hex(0xdd6845))
                        .stroke_style(StrokeStyle {
                            width: 1.5,
                            cap,
                            join,
                        })
                        .translate(0.25, 3.0),
                ])
                .opacity(0.65)
                .clip_with(Rect::new(0.5, 0.5, 7.5, 7.5), motion::ClipMode::Geometric)
            };
            let mut first = None;
            let mut max_difference = 0;
            for index in [0, 11, 8, 5, 2, 9, 1, 10, 7, 4, 3, 6, 0] {
                let expected = cpu.render_frame(index, build)?;
                let actual = gpu.render_frame(index, build)?;
                for (expected, actual) in expected.packed_rgb().iter().zip(actual.packed_rgb()) {
                    for shift in [0, 8, 16] {
                        max_difference = max_difference
                            .max(((expected >> shift) & 255).abs_diff((actual >> shift) & 255));
                    }
                }
                assert!(
                    max_difference <= 1,
                    "stroke CPU/GPU RGB difference {max_difference}"
                );
                if let Some(first) = &first {
                    if index == 0 {
                        assert!(actual.packed_rgb() == *first, "GPU stroke replay differs");
                    }
                } else {
                    first = Some(actual.packed_rgb());
                }
            }
            assert!(gpu.gpu_info().is_some());
            eprintln!(
                "stroke GPU: {:?}; max RGB difference={max_difference}",
                gpu.gpu_info()
            );
            Ok(())
        }
    }

    mod path2d {
        use motion::extend::*;
        use motion::prelude::*;

        #[test]
        fn svg_compilation_matches_manual_curves_with_paint_transform_and_clip() {
            use motion::extend::DrawContext;
            use motion::render::{RasterLimits, ValidationLimits};
            use motion::{Color, Node, NodeModifiers, PixelSize, Shape, Size};
            let parsed = Path2D::parse_svg("M1 1q2 4 4 0z").unwrap();
            let snapshot = parsed.clone();
            let mut manual = Path2D::new();
            manual.try_move_to(Point::new(1.0, 1.0)).unwrap();
            manual
                .try_quadratic_to(Point::new(3.0, 5.0), Point::new(5.0, 1.0))
                .unwrap();
            manual.try_close().unwrap();
            let color = Color::from_rgb_float(1.25, -0.1, 0.5);
            let styled = parsed.clone().fill(color);
            let limits = ValidationLimits {
                max_nodes: 2 + parsed.verbs().len(),
                max_depth: 2,
            };
            assert!(DrawContext::default().record(&styled, limits).is_ok());
            assert_eq!(
                DrawContext::default()
                    .record(
                        &styled,
                        ValidationLimits {
                            max_nodes: limits.max_nodes - 1,
                            ..limits
                        }
                    )
                    .unwrap_err()
                    .field,
                "max_nodes"
            );
            let compile = |path: Path2D| {
                Node::from(path.fill(color))
                    .pivot(Pivot::Point(Point::ORIGIN))
                    .scale_xy(1.25, 1.5)
                    .translate(0.25, 0.5)
                    .opacity(0.5)
                    .clip_with(Rect::new(0.0, 0.0, 5.0, 5.0), motion::ClipMode::Geometric)
                    .compile(
                        Size::new(8.0, 8.0),
                        PixelSize::new(8, 8),
                        ValidationLimits::default(),
                    )
                    .unwrap()
            };
            let actual = compile(parsed.clone());
            let expected = compile(manual);
            assert_eq!(actual.commands, expected.commands);
            let render = |list: motion::render::ValidatedDisplayList| {
                list.prepare(ValidationLimits::default(), RasterLimits::default())
                    .unwrap()
                    .render_linear(RasterLimits::default())
                    .unwrap()
            };
            assert_eq!(render(actual), render(expected));
            let _ = parsed.bounding_box();
            let mut changed = parsed.clone();
            changed.try_line_to(Point::new(7.0, 7.0)).unwrap();
            assert_eq!(parsed, snapshot);
            assert_ne!(changed, parsed);
        }

        #[test]
        fn svg_subpaths_retain_nonzero_and_evenodd_fill_rules() {
            use motion::render::{RasterLimits, ValidationLimits};
            use motion::{Color, FillRule, Node, PixelSize, Shape, Size};
            let path = Path2D::parse_svg("M1 1h6v6H1z m2 2h2v2H3z").unwrap();
            for rule in [FillRule::NonZero, FillRule::EvenOdd] {
                let frame = Node::from(path.clone().fill(Color::WHITE).fill_rule(rule))
                    .compile(
                        Size::new(8.0, 8.0),
                        PixelSize::new(8, 8),
                        ValidationLimits::default(),
                    )
                    .unwrap()
                    .prepare(ValidationLimits::default(), RasterLimits::default())
                    .unwrap()
                    .render_linear(RasterLimits::default())
                    .unwrap();
                assert_eq!(frame.linear_pixel(2, 2), Some([1.0; 4]));
                assert_eq!(
                    frame.linear_pixel(4, 4),
                    Some([if rule == FillRule::EvenOdd { 0.0 } else { 1.0 }; 4])
                );
            }
        }

        #[cfg(feature = "gpu")]
        #[test]
        #[ignore = "requires a native GPU; no window or video export"]
        fn svg_gpu_matches_cpu_and_replays() -> motion::Result {
            use motion::render::Composition;
            use motion::render::Renderer;
            use motion::{Color, FillRule, Node, NodeModifiers, Shape, Size};
            let cases = [
                ("M1 1q2 4 4 0t-4 0z", FillRule::EvenOdd),
                ("M1 4c0-3 6-3 6 0s-6 3-6 0z", FillRule::NonZero),
                ("M1 1h6v6H1z m2 2h2v2H3z", FillRule::EvenOdd),
                ("M1 1h6v6H1z m2 2h2v2H3z", FillRule::NonZero),
                ("M1 1 7 1 4 7z", FillRule::NonZero),
            ];
            let composition = Composition::new(
                16,
                16,
                motion::render::FrameClock::new(30, cases.len() as u64),
            )
            .with_options(motion::render::RenderOptions::default().background(Color::BLACK))?;
            let mut cpu = composition.frame_renderer(Size::new(8.0, 8.0))?;
            let mut gpu = composition
                .configure(|options| options.backend(Renderer::Gpu.into()))
                .frame_renderer(Size::new(8.0, 8.0))?;
            let build = |context: &motion::SceneContext<'_>| {
                let index = context.time().to_frame(context.frame_rate()) as usize;
                let (data, rule) = cases[index];
                let styled = Path2D::parse_svg(data)
                    .expect("fixed SVG fixture")
                    .fill(Color::from_hex(0x467c93))
                    .fill_rule(rule);
                let styled = if index == 4 {
                    styled.stroke(0.5, Color::WHITE)
                } else {
                    styled
                };
                Node::from(styled)
                    .opacity(0.65)
                    .clip_with(Rect::new(0.5, 0.5, 7.5, 7.5), motion::ClipMode::Geometric)
            };
            let mut first = None;
            let mut maximum = 0;
            for index in [0, 4, 1, 3, 2, 0] {
                let expected = cpu.render_frame(index, build)?;
                let actual = gpu.render_frame(index, build)?;
                for (expected, actual) in expected.packed_rgb().iter().zip(actual.packed_rgb()) {
                    for shift in [0, 8, 16] {
                        maximum = maximum
                            .max(((expected >> shift) & 255).abs_diff((actual >> shift) & 255));
                    }
                }
                assert!(maximum <= 1, "SVG CPU/GPU RGB difference {maximum}");
                if let Some(first) = &first {
                    if index == 0 {
                        assert!(actual.packed_rgb() == *first, "SVG GPU replay differs");
                    }
                } else {
                    first = Some(actual.packed_rgb());
                }
            }
            assert!(gpu.gpu_info().is_some());
            eprintln!(
                "SVG GPU: {:?}; max RGB difference={maximum}",
                gpu.gpu_info()
            );
            Ok(())
        }
    }

    mod shapes {
        use motion::ArcShape as Arc;
        use motion::extend::*;
        use motion::prelude::*;
        use motion::render::*;

        fn sample_paths() -> Result<Vec<Path2D>, ValidationError> {
            let center = Point::new(4.0, 4.0);
            let paths = [
                Circle::new(center, 2.0)?.path(),
                Ellipse::new(center, Vector2::new(3.0, 2.0))?.path(),
                Arc::new(center, 2.0, 0.0, 2.0)?.path(),
                RoundedRect::new(Rect::new(1.0, 1.0, 7.0, 7.0), 2.0)?.path(),
                Ring::new(center, 1.0, 3.0).path(),
                Line::new(Point::new(1.0, 2.0), Point::new(7.0, 6.0))?.path(),
                Polyline::new([Point::new(1.0, 2.0), center, Point::new(7.0, 2.0)])?.path(),
                Polygon::new([
                    Point::new(1.0, 1.0),
                    Point::new(7.0, 1.0),
                    Point::new(4.0, 7.0),
                ])?
                .path(),
                RegularPolygon::new(center, 3.0, 5)?.path(),
                Star::new(center, 5, 3.0, 1.5)?.path(),
                Arrow::new(Point::new(1.0, 4.0), Point::new(7.0, 4.0))
                    .head_size(2.0)
                    .path(),
                Spline::through([
                    Point::new(1.0, 6.0),
                    Point::new(4.0, 1.0),
                    Point::new(7.0, 6.0),
                ])?
                .path(),
                FunctionGraph::new(|value| 4.0 + math::sin(value), 1.0..7.0)
                    .samples(16)
                    .path(),
                FunctionGraph::parametric(|value| Point::new(value, value), 1.0..7.0)
                    .samples(8)
                    .path(),
            ];
            paths.into_iter().collect()
        }

        #[test]
        fn all_shapes_record_fill_and_stroke() {
            let center = Point::new(4.0, 4.0);
            for path in sample_paths().unwrap() {
                assert!(!path.verbs().is_empty());
                let frame = Node::from(path.stroke(0.5, Color::WHITE))
                    .compile(
                        Size::new(8.0, 8.0),
                        PixelSize::new(8, 8),
                        ValidationLimits::default(),
                    )
                    .unwrap()
                    .prepare(ValidationLimits::default(), RasterLimits::default())
                    .unwrap()
                    .render_linear(RasterLimits::default())
                    .unwrap();
                assert!((0..8).any(|row| {
                    (0..8).any(|column| frame.linear_pixel(column, row).unwrap()[3] > 0.0)
                }));
            }
            let ring = Node::from(Ring::new(center, 2.0, 3.0).fill(Color::WHITE))
                .compile(
                    Size::new(8.0, 8.0),
                    PixelSize::new(8, 8),
                    ValidationLimits::default(),
                )
                .unwrap()
                .prepare(ValidationLimits::default(), RasterLimits::default())
                .unwrap()
                .render_linear(RasterLimits::default())
                .unwrap();
            assert_eq!(ring.linear_pixel(4, 4).unwrap(), [0.0; 4]);
            assert!(ring.linear_pixel(4, 1).unwrap()[3] > 0.5);
        }

        #[test]
        #[cfg(feature = "gpu")]
        #[ignore = "requires native GPU; no font discovery, encoder or window"]
        fn shapes_trim_dash_and_curves_match_gpu_and_replay() -> motion::Result {
            use motion::{LineCap, LineJoin, NodeModifiers, StrokeStyle};
            let limits = RasterLimits::default();
            let mut gpu = motion::render::gpu::Gpu::new()?;
            let info = gpu.info().clone();
            let mut nodes: Vec<Node> = sample_paths()?
                .into_iter()
                .map(|path| {
                    Node::from(
                        path.fill(Color::from_rgb_float(0.1, 0.7, 1.2))
                            .stroke(0.5, Color::WHITE),
                    )
                })
                .collect();
            for cap in [LineCap::Butt, LineCap::Round, LineCap::Square] {
                let path = Path2D::parse_svg("M1 6C1 1 7 1 7 6")?;
                nodes.push(
                    Node::from(
                        path.stroke(0.7, Color::WHITE)
                            .stroke_style(StrokeStyle {
                                width: 0.7,
                                cap,
                                join: LineJoin::Round,
                            })
                            .trim(0.1, 0.9)
                            .dash(&[1.0, 0.4], -0.3),
                    )
                    .rotate(0.1),
                );
            }
            let mut maximum = 0.0_f32;
            let mut saved = Vec::new();
            for node in &nodes {
                let prepared = node
                    .compile(
                        Size::new(8.0, 8.0),
                        PixelSize::new(8, 8),
                        ValidationLimits::default(),
                    )?
                    .prepare(ValidationLimits::default(), limits)?;
                let cpu = prepared.render_linear(limits)?;
                let rendered = gpu.render_linear(&prepared, limits)?;
                for row in 0..8 {
                    for column in 0..8 {
                        for (first, second) in cpu
                            .linear_pixel(column, row)
                            .unwrap()
                            .into_iter()
                            .zip(rendered.linear_pixel(column, row).unwrap())
                        {
                            maximum = maximum.max((first - second).abs());
                        }
                    }
                }
                let cpu = cpu.to_format(
                    motion::render::PixelFormat::SrgbRgba8 {
                        premultiplied: false,
                    },
                    limits,
                )?;
                let rendered = rendered.to_format(
                    motion::render::PixelFormat::SrgbRgba8 {
                        premultiplied: false,
                    },
                    limits,
                )?;
                for (first, second) in cpu.bytes().iter().zip(rendered.bytes()) {
                    assert!(first.abs_diff(*second) <= 1);
                }
                saved.push(rendered.bytes().to_vec());
            }
            assert!(maximum <= 2e-6, "maximum linear difference {maximum}");
            for (index, node) in nodes.iter().enumerate().rev() {
                let prepared = node
                    .compile(
                        Size::new(8.0, 8.0),
                        PixelSize::new(8, 8),
                        ValidationLimits::default(),
                    )?
                    .prepare(ValidationLimits::default(), limits)?;
                let replay = gpu.render_linear(&prepared, limits)?.to_format(
                    motion::render::PixelFormat::SrgbRgba8 {
                        premultiplied: false,
                    },
                    limits,
                )?;
                assert_eq!(replay.bytes(), saved[index]);
            }
            eprintln!(
                "_shapes_gpu adapter={} backend={:?} cases={} max_linear_difference={maximum} rgba_tolerance=1 reverse_replay=exact",
                info.name,
                info.backend,
                nodes.len()
            );
            Ok(())
        }
    }
}
