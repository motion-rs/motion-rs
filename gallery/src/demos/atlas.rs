//! Shared page framing for focused capability comparisons in the gallery.

use super::label;
use crate::{DemoMetadata, DemoParameters};
use motion::prelude::*;

pub(super) const PAPER: u32 = 0xf1f3f0;

pub(super) const DARK: u32 = 0x17231f;

pub(super) const MINT: u32 = 0x66e0c2;

pub(super) const BLUE: u32 = 0x8bd5f3;

pub(super) const GOLD: u32 = 0xffc777;

const INK: u32 = 0x1f2723;

const MUTED: u32 = 0x56635c;

const RULE: u32 = 0xc3cbc5;

const ACCENT: u32 = 0xd64b36;

/// Local specimen bounds in design units; clipping wraps all effects.
pub(super) const VIEWPORT: Rect = Rect::new(0.0, 0.0, 512.0, 304.0);

/// Specimen card in the 960x540 page; its 568-unit width keeps the 512:304 viewport aspect.
pub(super) const SPECIMEN_BOUNDS: Rect = Rect::new(344.0, 124.0, 912.0, 461.25);

/// Outer page margin in design units, shared by header, caption column and footer.
const MARGIN: f64 = 48.0;

/// Width of the caption column left of the specimen card, in design units.
const CAPTION_WIDTH: f64 = 268.0;

/// Corner radius of the specimen card in page design units.
const CARD_RADIUS: f64 = 10.0;

/// Scale of each half of a side-by-side comparison about the viewport center.
const PAIR_SCALE: f64 = 0.48;

pub(super) struct Page {
    pub title: &'static str,
    pub lines: [&'static str; 3],
}

/// Everything the shared frame shows around one specimen.
pub(super) struct Sheet<'a> {
    pub metadata: DemoMetadata,
    pub page: usize,
    pub count: usize,
    pub progress: f64,
    pub title: &'a str,
    pub lines: &'a [&'a str],
    pub tag: Option<&'a str>,
    pub next: Option<&'a str>,
}

pub(super) fn builder(
    parameters: DemoParameters,
    metadata: DemoMetadata,
    pages: &'static [Page],
    draw: impl Fn(usize, f64, &SceneContext<'_>) -> motion::Result<Node> + Send + Sync + 'static,
) -> TimelineBuilder {
    Timeline::new(Size::new(960.0, 540.0), 30)
        .background(Color::from_hex(PAPER))
        .track(|track| {
            track.scene(parameters.seconds.seconds(), move |context| {
                let (index, progress) = page_at(context.progress(), pages.len());
                let page = &pages[index];
                let specimen = Node::group([stage(), draw(index, progress, context)?]);
                sheet(
                    &Sheet {
                        metadata,
                        page: index,
                        count: pages.len(),
                        progress: context.progress(),
                        title: page.title,
                        lines: &page.lines,
                        tag: None,
                        next: pages.get(index + 1).map(|next| next.title),
                    },
                    specimen,
                )
            })
        })
}

pub(super) fn timeline(
    parameters: DemoParameters,
    metadata: DemoMetadata,
    pages: &'static [Page],
    draw: impl Fn(usize, f64, &SceneContext<'_>) -> motion::Result<Node> + Send + Sync + 'static,
) -> motion::Result<Timeline> {
    builder(parameters, metadata, pages, draw).build()
}

/// Composes the page frame, the caption column and a specimen drawn in [`VIEWPORT`] units.
pub(super) fn sheet(sheet: &Sheet<'_>, specimen: Node) -> motion::Result<Node> {
    let mut nodes = header(sheet);
    nodes.extend(caption(sheet));
    nodes.push(card(specimen)?);
    nodes.extend(footer(sheet.page, sheet.count, sheet.progress));
    Ok(Node::group(nodes))
}

fn header(sheet: &Sheet<'_>) -> Vec<Node> {
    let right = 960.0 - MARGIN;
    vec![
        tracked(
            &format!(
                "MOTION GALLERY  /  {}",
                sheet.metadata.category.to_uppercase()
            ),
            11.0,
            MUTED,
            MARGIN,
            28.0,
        ),
        heading(sheet.metadata.title, 32.0, MARGIN - 1.0, 44.0),
        label(
            &format!("{:02}", sheet.page + 1),
            "",
            24.0,
            0.0,
            FontWeight::SEMIBOLD,
            Color::from_hex(INK),
        )
        .max_width(120.0)
        .alignment(TextAlignment::End)
        .translate(right - 168.0, 50.0),
        label(
            &format!("/ {:02}", sheet.count),
            "",
            15.0,
            0.0,
            FontWeight::MEDIUM,
            Color::from_hex(MUTED),
        )
        .max_width(44.0)
        .alignment(TextAlignment::End)
        .translate(right - 44.0, 58.0),
        Rect::new(MARGIN, 99.5, right, 100.5)
            .fill(Color::from_hex(RULE))
            .into(),
    ]
}

fn caption(sheet: &Sheet<'_>) -> Vec<Node> {
    let top = SPECIMEN_BOUNDS.top;
    let size = if sheet.title.chars().count() > 16 {
        22.0
    } else {
        26.0
    };
    let mut nodes = vec![heading(sheet.title, size, MARGIN - 1.0, top - 4.0)];
    let body = top + 52.0;
    for (row, line) in sheet.lines.iter().enumerate() {
        nodes.push(text(line, 17.0, INK, MARGIN, body + row as f64 * 26.0));
    }
    let rule = body + sheet.lines.len() as f64 * 26.0 + 18.0;
    nodes.push(
        Rect::new(MARGIN, rule, MARGIN + 28.0, rule + 3.0)
            .fill(Color::from_hex(ACCENT))
            .into(),
    );
    if let Some(tag) = sheet.tag {
        nodes.push(tracked(tag, 11.0, MUTED, MARGIN, rule + 16.0));
    }
    let next = sheet.next.map_or_else(
        || "End of atlas".to_owned(),
        |title| format!("Next  /  {title}"),
    );
    nodes.push(
        label(
            &next,
            "",
            13.0,
            0.0,
            FontWeight::MEDIUM,
            Color::from_hex(MUTED),
        )
        .max_width(CAPTION_WIDTH)
        .translate(MARGIN, SPECIMEN_BOUNDS.bottom - 18.0),
    );
    nodes
}

fn card(specimen: Node) -> motion::Result<Node> {
    let scale = SPECIMEN_BOUNDS.width() / VIEWPORT.width();
    Ok(Node::group([
        RoundedRect::new(SPECIMEN_BOUNDS, CARD_RADIUS)?
            .fill(Color::from_hex(DARK))
            .filter(Filter::DropShadow {
                offset: Vector2::new(0.0, 8.0),
                sigma: 12.0,
                color: Color::from_hex(DARK).with_alpha(0.22),
            }),
        specimen
            .clip_with(
                RoundedRect::new(VIEWPORT, CARD_RADIUS / scale)?,
                ClipMode::Geometric,
            )
            .pivot(Pivot::Point(Point::ORIGIN))
            .scale(scale)
            .translate(SPECIMEN_BOUNDS.left, SPECIMEN_BOUNDS.top),
    ]))
}

fn footer(page: usize, count: usize, progress: f64) -> Vec<Node> {
    let top = 494.0;
    let gap = if count > 24 { 2.0 } else { 4.0 };
    let width = (960.0 - 2.0 * MARGIN - gap * (count - 1) as f64) / count as f64;
    let mut nodes = Vec::with_capacity(count * 2);
    for index in 0..count {
        let left = MARGIN + index as f64 * (width + gap);
        let filled = (progress * count as f64 - index as f64).clamp(0.0, 1.0);
        let height = if index == page { 4.0 } else { 2.0 };
        nodes.push(
            Rect::new(left, top - height * 0.5, left + width, top + height * 0.5)
                .fill(Color::from_hex(RULE))
                .into(),
        );
        if filled > 0.0 {
            nodes.push(
                Rect::new(
                    left,
                    top - height * 0.5,
                    left + width * filled,
                    top + height * 0.5,
                )
                .fill(Color::from_hex(ACCENT))
                .into(),
            );
        }
    }
    nodes
}

pub(super) fn page_at(progress: f64, count: usize) -> (usize, f64) {
    let scaled = progress * count as f64;
    let page = (scaled as usize).min(count - 1);
    (page, (scaled - page as f64).clamp(0.0, 1.0))
}

pub(super) fn text(value: &str, size: f64, color: u32, left: f64, top: f64) -> Node {
    label(
        value,
        "",
        size,
        0.0,
        FontWeight::MEDIUM,
        Color::from_hex(color),
    )
    .translate(left, top)
}

fn heading(value: &str, size: f64, left: f64, top: f64) -> Node {
    label(
        value,
        "",
        size,
        0.0,
        FontWeight::SEMIBOLD,
        Color::from_hex(INK),
    )
    .translate(left, top)
}

fn tracked(value: &str, size: f64, color: u32, left: f64, top: f64) -> Node {
    label(
        value,
        "",
        size,
        1.2,
        FontWeight::SEMIBOLD,
        Color::from_hex(color),
    )
    .translate(left, top)
}

pub(super) fn stage() -> Node {
    let mut nodes: Vec<Node> = vec![VIEWPORT.fill(Color::from_hex(DARK)).into()];
    for column in 1..8 {
        let left = column as f64 * 64.0;
        nodes.push(
            Rect::new(left, 0.0, left + 0.5, 304.0)
                .fill(Color::from_hex(0x26372f))
                .into(),
        );
    }
    for row in 1..5 {
        let top = row as f64 * 64.0;
        nodes.push(
            Rect::new(0.0, top, 512.0, top + 0.5)
                .fill(Color::from_hex(0x26372f))
                .into(),
        );
    }
    Node::group(nodes)
}

pub(super) fn specimen(progress: f64) -> motion::Result<Node> {
    let phase = progress * std::f64::consts::TAU;
    Ok(Node::group([
        Node::from(VIEWPORT.fill(Color::from_hex(DARK).with_alpha(0.0))),
        Rect::new(72.0, 64.0, 440.0, 240.0)
            .fill(
                LinearGradient::new(Point::new(72.0, 64.0), Point::new(440.0, 240.0))
                    .units(GradientUnits::UserSpace)
                    .stop(0.0, Color::from_hex(BLUE))
                    .stop(1.0, Color::from_hex(0xd64b36)),
            )
            .into(),
        Circle::new(Point::new(256.0 + math::sin(phase) * 55.0, 152.0), 64.0)?
            .fill(Color::from_hex(GOLD))
            .into(),
        Circle::new(Point::new(250.0, 127.0), 12.0)?
            .fill(Color::WHITE)
            .filter(Filter::ColorAdjust {
                exposure: 2.0,
                contrast: 1.0,
                saturation: 1.0,
                hue_degrees: 0.0,
            }),
        Rect::new(184.0, 177.0, 330.0, 184.0)
            .fill(Color::WHITE)
            .into(),
    ]))
}

pub(super) fn comparison(source: Node, processed: Node) -> Node {
    pair(source, processed, "Original", "Processed")
}

pub(super) fn pair(source: Node, processed: Node, left: &str, right: &str) -> Node {
    let center = VIEWPORT.center();
    let half = VIEWPORT.width() * 0.25;
    let caption = |value: &str, x: f64| {
        label(
            value,
            "",
            14.0,
            0.6,
            FontWeight::MEDIUM,
            Color::from_hex(0xb9cec5),
        )
        .max_width(2.0 * half - 16.0)
        .alignment(TextAlignment::Center)
        .translate(x - half + 8.0, 222.0)
    };
    Node::group([
        Rect::new(center.x - 0.5, 56.0, center.x + 0.5, 248.0)
            .fill(Color::from_hex(0x3a4d44))
            .into(),
        source.scale(PAIR_SCALE).translate(-half, 0.0),
        processed.scale(PAIR_SCALE).translate(half, 0.0),
        caption(left, center.x - half),
        caption(right, center.x + half),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_pages_have_stable_boundaries() {
        for count in [1, 6, 12, 24] {
            assert_eq!(page_at(0.0, count), (0, 0.0));
            assert_eq!(page_at(1.0, count), (count - 1, 1.0));
            for page in 0..count {
                let (actual, local) = page_at((page as f64 + 0.5) / count as f64, count);
                assert_eq!(actual, page);
                assert!((local - 0.5).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn capability_atlases_have_unique_pages_and_exact_durations() -> motion::Result {
        let project = crate::project(RenderOptions::default())?;
        for (id, _, pages) in cases() {
            let mut titles = std::collections::BTreeSet::new();
            for page in pages {
                assert!(titles.insert(page.title), "duplicate title in {id}");
                assert!(page.lines.iter().all(|line| !line.is_empty()));
            }
            for seconds in [1, 8, 60] {
                let timeline = project.build(id, Some(serde_json::json!({"seconds": seconds})))?;
                assert_eq!(timeline.duration(), seconds.seconds(), "{id}");
                assert_eq!(timeline.frame_count(), seconds as u64 * 30, "{id}");
            }
        }
        assert_eq!(crate::catalog()[0].id, "animation-atlas");
        Ok(())
    }

    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT; every new capability page, CPU subjects and replay"]
    fn capability_atlases_render_animate_clip_and_replay() -> motion::Result {
        let font = std::env::var_os("MOTION_TEST_FONT")
            .ok_or_else(|| std::io::Error::other("Set MOTION_TEST_FONT"))?;
        let options = crate::render_options(Some(std::path::Path::new(&font)))?
            .backend(BackendPreference::Cpu)
            .size(PixelSize::new(288, 162));
        let system = crate::render_options(None)?
            .backend(BackendPreference::Cpu)
            .size(PixelSize::new(288, 162));
        let project = crate::project(RenderOptions::default())?;
        let parameters = DemoParameters {
            seconds: 60,
            ..DemoParameters::default()
        };
        for (id, metadata, pages) in cases() {
            let options = if matches!(id, "typography-atlas" | "caption-atlas") {
                &system
            } else {
                &options
            };
            let film = project.build(id, Some(serde_json::json!({"seconds": 60})))?;
            let empty = timeline(parameters, metadata, pages, |_, _, _| Ok(Node::empty()))?;
            for (page, description) in pages.iter().enumerate() {
                let moment =
                    |local: f64| ((page as f64 + local) * 60.0 / pages.len() as f64).seconds();
                let early = render_still(&film, moment(0.15).after_origin(), options)?;
                let late = render_still(&film, moment(0.7).after_origin(), options)?;
                for local in [0.5, 0.95] {
                    render_still(&film, moment(local).after_origin(), options)?;
                }
                let replay = render_still(&film, moment(0.15).after_origin(), options)?;
                assert_eq!(
                    early.bytes(),
                    replay.bytes(),
                    "{id} / {} replay",
                    description.title
                );
                let background = render_still(&empty, moment(0.15).after_origin(), options)?;
                let left = (SPECIMEN_BOUNDS.left * 0.3).ceil() as usize + 1;
                let right = (SPECIMEN_BOUNDS.right * 0.3).floor() as usize;
                let top = (SPECIMEN_BOUNDS.top * 0.3).ceil() as usize + 1;
                let bottom = (SPECIMEN_BOUNDS.bottom * 0.3).floor() as usize;
                let offsets = || {
                    (top..bottom)
                        .flat_map(|row| (left..right).map(move |column| (row * 288 + column) * 4))
                };
                let changed = offsets()
                    .filter(|offset| {
                        early.bytes()[*offset..*offset + 3] != late.bytes()[*offset..*offset + 3]
                    })
                    .count();
                assert!(changed > 6, "{id} / {} did not animate", description.title);
                let painted = offsets()
                    .filter(|offset| {
                        early.bytes()[*offset..*offset + 3]
                            .iter()
                            .zip(&background.bytes()[*offset..*offset + 3])
                            .any(|(actual, empty)| actual.abs_diff(*empty) > 8)
                    })
                    .count();
                assert!(painted > 6, "{id} / {} missing subject", description.title);
                for row in 0..162 {
                    for column in 0..288 {
                        if (top - 3..bottom + 3).contains(&row)
                            && (left - 3..right + 3).contains(&column)
                        {
                            continue;
                        }
                        let offset = (row * 288 + column) * 4;
                        assert!(
                            early.bytes()[offset..offset + 4]
                                .iter()
                                .zip(&background.bytes()[offset..offset + 4])
                                .all(|(actual, empty)| actual.abs_diff(*empty) <= 1),
                            "{id} / {} leaked outside viewport",
                            description.title
                        );
                    }
                }
            }
            eprintln!(
                "{id}: {} visible, animated, clipped and replayed pages",
                pages.len()
            );
        }
        Ok(())
    }

    #[cfg(feature = "gpu")]
    #[test]
    #[ignore = "requires explicit MOTION_TEST_FONT, system Chinese fonts and a native GPU"]
    fn capability_atlases_match_cpu_gpu_and_replay() -> motion::Result {
        let font = std::env::var_os("MOTION_TEST_FONT")
            .ok_or_else(|| std::io::Error::other("Set MOTION_TEST_FONT"))?;
        let cpu = crate::render_options(Some(std::path::Path::new(&font)))?
            .backend(BackendPreference::Cpu)
            .size(PixelSize::new(288, 162));
        let gpu = cpu
            .clone()
            .backend(BackendPreference::gpu())
            .gpu_context(motion::render::GpuContext::new()?);
        let system_cpu = crate::render_options(None)?
            .backend(BackendPreference::Cpu)
            .size(PixelSize::new(288, 162));
        let system_gpu = system_cpu
            .clone()
            .backend(BackendPreference::gpu())
            .gpu_context(motion::render::GpuContext::new()?);
        let project = crate::project(RenderOptions::default())?;
        for (id, _, pages) in cases() {
            let (cpu, gpu) = if matches!(id, "typography-atlas" | "caption-atlas") {
                (&system_cpu, &system_gpu)
            } else {
                (&cpu, &gpu)
            };
            let film = project.build(id, Some(serde_json::json!({"seconds": 60})))?;
            let mut maximum = 0;
            for (page, description) in pages.iter().enumerate() {
                for local in [0.15, 0.7] {
                    let time = ((page as f64 + local) * 60.0 / pages.len() as f64).seconds();
                    let expected = render_still(&film, time.after_origin(), cpu)?;
                    let actual = render_still(&film, time.after_origin(), gpu)?;
                    let replay = render_still(&film, time.after_origin(), gpu)?;
                    assert_eq!(
                        actual.bytes(),
                        replay.bytes(),
                        "{id} / {} GPU replay",
                        description.title
                    );
                    let difference = expected
                        .bytes()
                        .iter()
                        .zip(actual.bytes())
                        .map(|(expected, actual)| expected.abs_diff(*actual))
                        .max()
                        .unwrap_or(0);
                    maximum = maximum.max(difference);
                    assert!(
                        difference <= 2,
                        "{id} / {} CPU/GPU difference: {difference}",
                        description.title
                    );
                }
            }
            eprintln!(
                "{id}: {} native GPU pages, max_byte={maximum}, replay=exact",
                pages.len()
            );
        }
        Ok(())
    }

    fn cases() -> [(&'static str, DemoMetadata, &'static [Page]); 10] {
        use super::super::{
            animation_atlas, audio_atlas, brush_atlas, caption_atlas, compositing_atlas,
            filter_atlas, ink_features, noise_atlas, typography_atlas, vector_atlas,
        };
        [
            (animation_atlas::METADATA, animation_atlas::PAGES),
            (brush_atlas::METADATA, brush_atlas::PAGES),
            (filter_atlas::METADATA, filter_atlas::PAGES),
            (compositing_atlas::METADATA, compositing_atlas::PAGES),
            (vector_atlas::METADATA, vector_atlas::PAGES),
            (ink_features::METADATA, ink_features::PAGES),
            (noise_atlas::METADATA, noise_atlas::PAGES),
            (audio_atlas::METADATA, audio_atlas::PAGES),
            (typography_atlas::METADATA, typography_atlas::PAGES),
            (caption_atlas::METADATA, caption_atlas::PAGES),
        ]
        .map(|(metadata, pages)| (metadata.id, metadata, pages))
    }
}
