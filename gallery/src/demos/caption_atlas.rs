//! Chinese and English subtitle layout, timing and SRT studies with system fonts.

use super::atlas::{self, Page};
use crate::{DemoMetadata, DemoParameters};
use motion::captions::{CaptionCue, CaptionStyle, CaptionTrack, CueSheet};
use motion::prelude::*;
use std::time::Duration;

const SRT: &str = "1\n00:00:00,000 --> 00:00:01,500\n光穿过房间。\nLight crosses the room.\n\n2\n00:00:01,500 --> 00:00:02,700\n文字留下形状。\nWords leave a shape.\n\n3\n00:00:02,700 --> 00:00:04,000\n新的故事开始了。\nA new story begins.\n";

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "caption-atlas",
    "Caption Atlas",
    "Captions",
    &[
        "captions",
        "chinese-english",
        "safe-area",
        "srt",
        "cue-markers",
    ],
);

pub(super) const PAGES: &[Page] = &[
    Page {
        title: "Timed cues",
        lines: [
            "Three subtitle cues follow",
            "their own explicit clock",
            "without playback state.",
        ],
    },
    Page {
        title: "Bilingual subtitles",
        lines: [
            "Chinese and English share",
            "one cue with an explicit",
            "line break between them.",
        ],
    },
    Page {
        title: "Automatic wrapping",
        lines: [
            "A measured paragraph wraps",
            "inside the safe width",
            "without manual word spacing.",
        ],
    },
    Page {
        title: "Safe-area layout",
        lines: [
            "A narrower safe rectangle",
            "constrains both the text",
            "and its shared backplate.",
        ],
    },
    Page {
        title: "Subtitle padding",
        lines: [
            "Explicit horizontal and",
            "vertical padding separates",
            "glyphs from the backplate.",
        ],
    },
    Page {
        title: "Contrast backplate",
        lines: [
            "A translucent background",
            "keeps captions readable over",
            "moving light and dark shapes.",
        ],
    },
    Page {
        title: "Keyword highlights",
        lines: [
            "Literal Chinese and English",
            "keywords color whole",
            "grapheme-safe matches.",
        ],
    },
    Page {
        title: "Overlapping cues",
        lines: [
            "Simultaneous cues stack",
            "bottom-up in input order",
            "with an explicit gap.",
        ],
    },
    Page {
        title: "Exact boundaries",
        lines: [
            "The first cue ends exactly",
            "where the second begins:",
            "start inclusive, end exclusive.",
        ],
    },
    Page {
        title: "SRT round trip",
        lines: [
            "Real SRT parsing and export",
            "preserve these bilingual cues",
            "and millisecond timestamps.",
        ],
    },
    Page {
        title: "Shared cue markers",
        lines: [
            "Named markers drive motion",
            "on the same composition clock",
            "as the subtitle track.",
        ],
    },
    Page {
        title: "Uniform retiming",
        lines: [
            "Four seconds become six.",
            "Text stays unchanged while",
            "cue boundaries scale together.",
        ],
    },
];

struct Study {
    sheet: CueSheet,
    style: CaptionStyle,
    original: Option<CueSheet>,
}

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    let studies = (0..PAGES.len())
        .map(study)
        .collect::<motion::Result<Vec<_>>>()?;
    atlas::timeline(
        parameters,
        METADATA,
        PAGES,
        move |page, progress, context| draw(&studies[page], page, progress, context),
    )
}

fn study(page: usize) -> motion::Result<Study> {
    let mut style = CaptionStyle::new(Rect::new(28.0, 136.0, 484.0, 272.0))
        .font_size(22.0)
        .max_lines(3)
        .padding(14.0, 10.0)
        .gap(12.0)
        .background(Color::rgba(0, 0, 0, 205));
    let descriptions: &[(u64, u64, &str)] = match page {
        0 => &[
            (0, 1400, "光穿过房间。"),
            (1400, 2700, "Words leave a shape."),
            (2700, 4000, "新的故事开始了。"),
        ],
        1 => &[(
            0,
            4000,
            "光穿过房间，文字留下形状。\nLight gives words a home.",
        )],
        2 => &[(
            0,
            4000,
            "A little space between the words lets every sentence breathe and gives the moment a shape.",
        )],
        3 => &[(0, 4000, "文字留在安全区。\nWords stay inside.")],
        4 => &[(0, 4000, "留白也有声音。\nA little room to breathe.")],
        5 => &[(0, 4000, "光与影，都在流动。\nLight and shadow move.")],
        6 => &[(0, 4000, "Light / 光\nWords / 文字")],
        7 => &[
            (0, 3000, "第一句仍在这里。"),
            (1000, 4000, "A second voice joins."),
        ],
        8 => &[
            (0, 2000, "第一句 / First cue"),
            (2000, 4000, "第二句 / Second cue"),
        ],
        10 => &[
            (0, 2000, "从这里开始。 / Begin here."),
            (2000, 4000, "向光而行。 / Follow the light."),
        ],
        _ => &[
            (0, 2000, "第一句 / First cue"),
            (2000, 4000, "第二句 / Second cue"),
        ],
    };
    if page == 3 {
        style = CaptionStyle::new(Rect::new(78.0, 108.0, 434.0, 253.0))
            .font_size(22.0)
            .padding(18.0, 12.0)
            .max_lines(3);
    } else if page == 4 {
        style = style.padding(30.0, 24.0);
    } else if page == 6 {
        style = style
            .keyword("Light", Color::from_hex(atlas::GOLD))
            .keyword("光", Color::from_hex(atlas::GOLD))
            .keyword("Words", Color::from_hex(atlas::MINT))
            .keyword("文字", Color::from_hex(atlas::MINT));
    } else if page == 11 {
        style = CaptionStyle::new(Rect::new(28.0, 202.0, 484.0, 272.0))
            .font_size(22.0)
            .padding(14.0, 8.0);
    }
    let track = if page == 9 {
        let parsed = CaptionTrack::from_srt(SRT)?;
        CaptionTrack::from_srt(&parsed.to_srt()?)?
    } else {
        CaptionTrack::new(
            descriptions
                .iter()
                .map(|(start, end, text)| {
                    CaptionCue::new(
                        Duration::from_millis(*start),
                        Duration::from_millis(*end),
                        *text,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
        )?
    };
    let mut sheet = CueSheet::new(Duration::from_secs(4), track)?;
    if page == 10 {
        sheet = sheet
            .mark("in", Duration::from_millis(500))?
            .mark("out", Duration::from_millis(3500))?;
    }
    let original = if page == 11 {
        let original = sheet.clone();
        sheet = sheet.retimed(Duration::from_secs(6))?;
        Some(original)
    } else {
        None
    };
    Ok(Study {
        sheet,
        style,
        original,
    })
}

fn draw(
    study: &Study,
    page: usize,
    progress: f64,
    context: &SceneContext<'_>,
) -> motion::Result<Node> {
    let fonts = context
        .assets()
        .fonts()
        .and_then(|fonts| fonts.downcast_ref::<FontLibrary>())
        .ok_or_else(|| std::io::Error::other("Caption Atlas requires the supplied font library"))?;
    let layout = context.cache(format!("caption-atlas-layout-{page}"), || {
        study
            .style
            .layout(study.sheet.captions(), Size::new(512.0, 304.0), fonts)
            .map_err(|error| error.to_string())
    });
    let layout = layout
        .as_ref()
        .as_ref()
        .map_err(|message| std::io::Error::other(message.clone()))?;
    let seconds = progress * study.sheet.duration().as_secs_f64();
    let time = seconds.seconds();
    let safe = layout.safe_area();
    let mut nodes = vec![
        Circle::new(Point::new(64.0 + progress * 384.0, 89.0), 33.0)
            .fill(Color::from_hex(atlas::GOLD))
            .into(),
        Rect::new(292.0, 72.0, 432.0, 272.0)
            .fill(Color::from_hex(atlas::BLUE).with_alpha(if page == 5 { 0.9 } else { 0.12 }))
            .into(),
        Rect::new(24.0, 36.0, 488.0, 38.0)
            .fill(Color::from_hex(0x648377))
            .into(),
        safe.stroke(1.0, Color::from_hex(0x648377)).into(),
        atlas::text(&format!("{seconds:.2} s"), 13.0, 0xb9cec5, 415.0, 12.0),
        layout.node(time),
    ];
    let duration = study.sheet.duration().as_secs_f64();
    for (index, cue) in study.sheet.captions().cues().iter().enumerate() {
        let left = 24.0 + cue.start().as_secs_f64() / duration * 464.0;
        let right = 24.0 + cue.end().as_secs_f64() / duration * 464.0;
        let top = 43.0 + index as f64 * 5.0;
        nodes.push(
            Rect::new(left, top, right, top + 3.0)
                .fill(Color::from_hex(atlas::MINT))
                .into(),
        );
    }
    nodes.push(
        Rect::new(23.0 + progress * 464.0, 32.0, 25.0 + progress * 464.0, 62.0)
            .fill(Color::from_hex(atlas::GOLD))
            .into(),
    );
    if page == 10 {
        let amount = study.sheet.progress(time, "in", "out")?;
        nodes.push(
            Circle::new(Point::new(50.0 + amount * 412.0, 118.0), 8.0)
                .fill(Color::from_hex(atlas::MINT))
                .into(),
        );
        nodes.push(atlas::text("in / 0.5 s", 12.0, 0xb9cec5, 28.0, 69.0));
        nodes.push(atlas::text("out / 3.5 s", 12.0, 0xb9cec5, 393.0, 69.0));
    }
    if let Some(original) = &study.original {
        let original_layout = context.cache("caption-atlas-original", || {
            CaptionStyle::new(Rect::new(28.0, 100.0, 484.0, 168.0))
                .font_size(22.0)
                .padding(14.0, 8.0)
                .layout(original.captions(), Size::new(512.0, 304.0), fonts)
                .map_err(|error| error.to_string())
        });
        let original_layout = original_layout
            .as_ref()
            .as_ref()
            .map_err(|message| std::io::Error::other(message.clone()))?;
        nodes.push(original_layout.node(time));
        nodes.push(atlas::text("Original / 4 s", 13.0, 0xb9cec5, 28.0, 78.0));
        nodes.push(atlas::text("Retimed / 6 s", 13.0, 0xb9cec5, 28.0, 180.0));
    }
    Ok(Node::group(nodes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caption_atlas_uses_real_boundaries_overlaps_srt_and_retiming() -> motion::Result {
        let boundary = study(8)?;
        let track = boundary.sheet.captions();
        assert_eq!(
            track
                .active_at_time(1999.milliseconds())
                .next()
                .map(CaptionCue::text),
            Some("第一句 / First cue")
        );
        assert_eq!(
            track
                .active_at_time(2.seconds())
                .next()
                .map(CaptionCue::text),
            Some("第二句 / Second cue")
        );
        assert_eq!(track.active_at_time(4.seconds()).count(), 0);
        assert_eq!(
            study(7)?
                .sheet
                .captions()
                .active_at_time(1500.milliseconds())
                .count(),
            2
        );
        let parsed = study(9)?;
        assert_eq!(parsed.sheet.captions(), &CaptionTrack::from_srt(SRT)?);
        let retimed = study(11)?;
        assert_eq!(retimed.sheet.duration(), Duration::from_secs(6));
        assert_eq!(
            retimed.sheet.captions().cues()[0].end(),
            Duration::from_secs(3)
        );
        assert_eq!(
            retimed.sheet.captions().cues()[1].start(),
            Duration::from_secs(3)
        );
        assert_eq!(study(10)?.sheet.progress(2.seconds(), "in", "out")?, 0.5);
        Ok(())
    }
}
