//! Chinese and English typography studies using the supplied system font library.

use super::atlas::{self, Page};
use crate::{DemoMetadata, DemoParameters};
use motion::prelude::*;

const ENGLISH: &str = "Light crosses the room. Words give the moment a shape.";

const CHINESE: &str = "光穿过房间，文字让此刻有了形状。留白连接句子，也让阅读拥有节奏。";

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "typography-atlas",
    "Typography Atlas",
    "Typography",
    &[
        "text",
        "chinese-english",
        "rich-text",
        "paragraph-layout",
        "grapheme-animation",
    ],
);

pub(super) const PAGES: &[Page] = &[
    Page {
        title: "Chinese and English",
        lines: [
            "Two languages share one",
            "paragraph and the supplied",
            "system font library.",
        ],
    },
    Page {
        title: "Rich spans",
        lines: [
            "Size, weight, color and italic",
            "styles inherit from a shared",
            "paragraph without manual gaps.",
        ],
    },
    Page {
        title: "Font weights",
        lines: [
            "Normal, medium and bold",
            "request the installed faces",
            "of the generic default family.",
        ],
    },
    Page {
        title: "Type hierarchy",
        lines: [
            "Three sizes establish a",
            "headline, a supporting line",
            "and a quiet annotation.",
        ],
    },
    Page {
        title: "Width and wrapping",
        lines: [
            "Changing paragraph width",
            "reshapes real line breaks",
            "without stretching glyphs.",
        ],
    },
    Page {
        title: "Line height",
        lines: [
            "The same Chinese paragraph",
            "compares compact and open",
            "baseline spacing.",
        ],
    },
    Page {
        title: "Start alignment",
        lines: [
            "Both language samples align",
            "to the start of their actual",
            "paragraph width.",
        ],
    },
    Page {
        title: "Center alignment",
        lines: [
            "Each wrapped line is centered",
            "inside the same measured",
            "paragraph container.",
        ],
    },
    Page {
        title: "End alignment",
        lines: [
            "Paragraph ends share a",
            "right edge while wrapping",
            "remains language-aware.",
        ],
    },
    Page {
        title: "Justified text",
        lines: [
            "Nonfinal English lines expand",
            "to the paragraph width;",
            "the final line stays natural.",
        ],
    },
    Page {
        title: "Text outlines",
        lines: [
            "Real glyph contours receive",
            "a bounded outline after fill;",
            "no shadow imitates a stroke.",
        ],
    },
    Page {
        title: "Gradient lettering",
        lines: [
            "A glyph-bounds gradient",
            "fills the original contours",
            "in both languages.",
        ],
    },
    Page {
        title: "Grapheme reveal",
        lines: [
            "Visibility advances by Unicode",
            "grapheme clusters and keeps",
            "the complete layout stable.",
        ],
    },
    Page {
        title: "Word reveal",
        lines: [
            "English words reveal in order",
            "while their spaces and wrapped",
            "positions remain unchanged.",
        ],
    },
    Page {
        title: "Line reveal",
        lines: [
            "Wrapped visual lines become",
            "the units of visibility",
            "instead of source newlines.",
        ],
    },
    Page {
        title: "Per-part motion",
        lines: [
            "A callback offsets, rotates",
            "and fades each shaped part",
            "from explicit page progress.",
        ],
    },
    Page {
        title: "Vector handwriting",
        lines: [
            "The writing effect traces",
            "native text outlines before",
            "handing off to their fills.",
        ],
    },
    Page {
        title: "Stable text layout",
        lines: [
            "Revealing more parts does not",
            "reflow the full paragraph",
            "or move the following words.",
        ],
    },
];

pub(super) fn timeline(parameters: DemoParameters) -> Result<Timeline, motion::ErrorReport> {
    atlas::timeline(parameters, METADATA, PAGES, |page, progress, context| {
        let fonts = context
            .assets()
            .fonts()
            .and_then(|fonts| fonts.downcast_ref::<FontLibrary>())
            .ok_or_else(|| {
                std::io::Error::other("Typography Atlas requires the supplied font library")
            })?;
        Ok(draw(page, progress, fonts)?.opacity(math::smoothstep(0.0, 0.3, progress)))
    })
}

fn paragraph(value: &str, size: f64) -> Text {
    Text::new(value.to_owned())
        .family("")
        .size(size)
        .line_height(1.45)
        .max_width(432.0)
        .fill(Color::from_hex(atlas::MINT))
}

fn draw(page: usize, progress: f64, fonts: &FontLibrary) -> motion::Result<Node> {
    let reveal = (progress * 1.3).min(1.0);
    Ok(match page {
        0 => paragraph("文字与光\nWords and light\n清晨 08:30 / A new day", 32.0)
            .translate(36.0, 50.0),
        1 => Text::rich([
            TextSpan::new("Words / 文字\n")
                .size(38.0)
                .weight(FontWeight::BOLD),
            TextSpan::new("A quiet ").italic(true),
            TextSpan::new("moment")
                .fill(Color::from_hex(atlas::GOLD))
                .weight(FontWeight::BOLD),
            TextSpan::new(".\n安静的"),
            TextSpan::new("时刻").fill(Color::from_hex(atlas::GOLD)),
            TextSpan::new("，从这里开始。"),
        ])
        .family("")
        .size(25.0)
        .line_height(1.5)
        .max_width(432.0)
        .fill(Color::from_hex(atlas::MINT))
        .translate(36.0, 48.0),
        2 => Node::group(
            [FontWeight::NORMAL, FontWeight::MEDIUM, FontWeight::BOLD]
                .into_iter()
                .enumerate()
                .map(|(row, weight)| {
                    paragraph("Light / 光与影", 32.0)
                        .weight(weight)
                        .translate(36.0, 42.0 + row as f64 * 77.0)
                }),
        ),
        3 => Node::group([
            paragraph("Morning / 清晨", 38.0)
                .weight(FontWeight::BOLD)
                .translate(36.0, 36.0),
            paragraph("Words find their place.", 25.0).translate(36.0, 116.0),
            paragraph("让文字找到自己的位置。", 19.0)
                .fill(Color::from_hex(atlas::GOLD))
                .translate(36.0, 179.0),
        ]),
        4 => {
            let width = 270.0 + 162.0 * progress;
            Node::group([
                Rect::new(36.0, 32.0, 36.0 + width, 267.0)
                    .stroke(1.0, Color::from_hex(0x648377))
                    .into(),
                paragraph(&format!("{ENGLISH}\n{CHINESE}"), 22.0)
                    .max_width(width - 16.0)
                    .translate(44.0, 40.0),
            ])
        }
        5 => Node::group([1.15, 1.8].into_iter().enumerate().map(|(column, height)| {
            paragraph("光穿过房间。\n文字留下形状。\n留白连接彼此。", 24.0)
                .max_width(205.0)
                .line_height(height)
                .translate(30.0 + column as f64 * 248.0, 54.0)
        })),
        6..=9 => {
            let alignment = [
                TextAlignment::Start,
                TextAlignment::Center,
                TextAlignment::End,
                TextAlignment::Justified,
            ][page - 6];
            let value = if page == 9 {
                format!("{ENGLISH} A little space lets every sentence breathe.")
            } else {
                "Light finds a place.\n清晨，文字与光相遇。\nA quiet beginning.".to_owned()
            };
            Node::group([
                Rect::new(36.0, 38.0, 476.0, 260.0)
                    .stroke(1.0, Color::from_hex(0x648377))
                    .into(),
                paragraph(&value, 25.0)
                    .max_width(420.0)
                    .alignment(alignment)
                    .translate(46.0, 53.0),
            ])
        }
        10 => paragraph("LIGHT\n光与文字", 58.0)
            .fill(Color::from_hex(atlas::DARK))
            .stroke(1.1 + progress, Color::from_hex(atlas::GOLD))
            .translate(34.0, 35.0),
        11 => paragraph("FLOW\n流动的文字", 51.0)
            .weight(FontWeight::BOLD)
            .fill(
                LinearGradient::new(Point::new(0.0, 0.0), Point::new(0.0, 1.0))
                    .units(GradientUnits::ObjectBoundingBox)
                    .stop(0.0, Color::from_hex(atlas::BLUE))
                    .stop(0.35, Color::from_hex(atlas::MINT))
                    .stop(0.7, Color::from_hex(atlas::GOLD))
                    .stop(1.0, Color::from_hex(atlas::GOLD)),
            )
            .translate(34.0, 41.0),
        12 => visible(
            paragraph("文字，让此刻有了形状。\nLight gives words a home.", 28.0),
            TextSplit::Characters,
            reveal,
            fonts,
        )?
        .translate(36.0, 72.0),
        13 => visible(
            paragraph(
                "Light crosses the room and every word finds its own quiet place.",
                29.0,
            ),
            TextSplit::Words,
            reveal,
            fonts,
        )?
        .translate(36.0, 54.0),
        14 => visible(
            paragraph(&format!("{CHINESE}\n{ENGLISH}"), 25.0).max_width(400.0),
            TextSplit::Lines,
            reveal,
            fonts,
        )?
        .translate(36.0, 36.0),
        15 => paragraph("Words in motion\n文字正在起舞", 34.0)
            .split(TextSplit::Characters)
            .animate_parts(0.seconds(), move |part| {
                let phase = (progress * 2.4 - part.index as f64 * 0.045).clamp(0.0, 1.0);
                let settle = Easing::CubicOut.apply(phase);
                PartStyle::new()
                    .offset(0.0, (1.0 - settle) * 55.0)
                    .rotate((1.0 - settle) * 0.25)
                    .opacity(phase)
            })
            .translate(36.0, 65.0),
        16 => paragraph("光 / LIGHT", 48.0)
            .write(reveal)
            .translate(35.0, 102.0),
        _ => {
            let value = "A quiet morning.\n安静的清晨。\nWords stay in place.";
            let text = paragraph(value, 27.0);
            Node::group([
                text.clone()
                    .fill(Color::from_hex(0x466455))
                    .translate(36.0, 49.0),
                visible(text, TextSplit::Characters, reveal, fonts)?.translate(36.0, 49.0),
            ])
        }
    })
}

fn visible(
    text: Text,
    split: TextSplit,
    progress: f64,
    fonts: &FontLibrary,
) -> motion::Result<Text> {
    let count = text.layout(fonts)?.parts(split).len();
    Ok(text
        .split(split)
        .visible_parts((progress * count as f64).ceil() as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires explicitly discovered system fonts with Chinese and English glyphs"]
    fn typography_atlas_system_fonts_keep_graphemes_and_layout_stable() -> motion::Result {
        let mut fonts = FontLibrary::new();
        fonts.load_system_fonts()?;
        let source = "Cafe\u{301} / 文字";
        let text = paragraph(source, 32.0);
        let layout = text.layout(&fonts)?;
        let parts = layout.parts(TextSplit::Characters);
        assert_eq!(parts.len(), 9);
        assert!(
            parts
                .iter()
                .any(|part| &source[part.source.clone()] == "e\u{301}")
        );
        let partial = visible(text.clone(), TextSplit::Characters, 0.5, &fonts)?.layout(&fonts)?;
        assert_eq!(layout.bounds(), partial.bounds());
        assert_eq!(layout.lines(), partial.lines());
        assert_eq!(layout.glyphs().len(), partial.glyphs().len());
        let complete = visible(text, TextSplit::Characters, 1.0, &fonts)?;
        assert_eq!(complete.layout(&fonts)?.bounds(), layout.bounds());
        assert_eq!(
            paragraph("Words\n文字", 24.0).layout(&fonts)?.lines().len(),
            2
        );
        Ok(())
    }

    #[test]
    #[ignore = "requires explicitly discovered system fonts with Chinese and English glyphs"]
    fn typography_atlas_gradient_contains_cool_and_warm_glyph_pixels() -> motion::Result {
        let options = crate::render_options(None)?.size(PixelSize::new(288, 162));
        let film = timeline(DemoParameters {
            seconds: 60,
            ..DemoParameters::default()
        })?;
        let frame = render_still(
            &film,
            (11.5 * 60.0 / PAGES.len() as f64).seconds().after_origin(),
            &options,
        )?;
        let mut cool = 0;
        let mut warm = 0;
        for row in 46..136 {
            for column in 117..269 {
                let offset = (row * 288 + column) * 4;
                let red = i16::from(frame.bytes()[offset]);
                let green = i16::from(frame.bytes()[offset + 1]);
                let blue = i16::from(frame.bytes()[offset + 2]);
                cool += usize::from(blue > red + 45 && green > red + 30);
                warm += usize::from(red > green + 20 && green > blue + 25);
            }
        }
        assert!(cool > 20, "missing cool gradient pixels: {cool}");
        assert!(warm > 20, "missing warm gradient pixels: {warm}");
        Ok(())
    }
}
