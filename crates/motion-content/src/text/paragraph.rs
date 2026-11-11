//! Paragraph description: text, style, width, alignment and fonts, keyed for shaping.

use super::*;
use crate::vector::CompileCache;

pub(crate) type SharedLayouts = Arc<Mutex<BTreeMap<ParagraphSpec, Arc<ParagraphLayout>>>>;

/// Shaping-relevant part of one span; paints are not part of the key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SpanSpec {
    pub(super) text: Arc<str>,
    pub(super) family: Option<Arc<str>>,
    pub(super) size: Option<u64>,
    pub(super) weight: Option<u16>,
    pub(super) italic: bool,
}

impl From<&TextSpan> for SpanSpec {
    fn from(span: &TextSpan) -> Self {
        Self {
            text: span.text.clone(),
            family: span.family.clone(),
            size: span.size.map(f64::to_bits),
            weight: span.weight.map(|weight| weight.0),
            italic: span.italic,
        }
    }
}

/// The immutable compile key of a paragraph: everything that determines glyph geometry.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ParagraphSpec {
    pub(super) spans: Vec<SpanSpec>,
    pub(super) family: Arc<str>,
    pub(super) size: u64,
    pub(super) weight: u16,
    pub(super) line_height: u64,
    pub(super) spacing: u64,
    pub(super) width: Option<u64>,
    pub(super) alignment: TextAlignment,
}

/// Paragraph description: rich text, base style, width, alignment and optional fonts.
///
/// Shaping it yields a [`ParagraphLayout`], the single intermediate
/// representation behind static drawing, reveal effects and captions. Changing a
/// shaping parameter gives a new cache; unchanged ones keep sharing the layout.
#[derive(Clone, Debug)]
pub struct ParagraphStyle {
    pub(super) cache: CompileCache<ParagraphSpec, Result<Arc<ParagraphLayout>, ValidationError>>,
    pub(super) fills: Arc<[Option<Paint>]>,
    pub(super) fonts: Option<FontLibrary>,
}

impl ParagraphStyle {
    /// Creates a paragraph of one inheriting span with size 24 and line height 1.3.
    pub fn new(text: impl Into<Arc<str>>) -> Self {
        Self::rich([TextSpan::new(text)])
    }

    /// Creates rich spans, whose unset styles inherit paragraph defaults.
    pub fn rich(spans: impl IntoIterator<Item = TextSpan>) -> Self {
        let spans = spans.into_iter().take(257).collect::<Vec<_>>();
        Self {
            fills: spans.iter().map(|span| span.fill.clone()).collect(),
            cache: CompileCache::new(ParagraphSpec {
                spans: spans.iter().map(SpanSpec::from).collect(),
                family: "".into(),
                size: 24.0f64.to_bits(),
                weight: FontWeight::NORMAL.0,
                line_height: 1.3f64.to_bits(),
                spacing: 0.0f64.to_bits(),
                width: None,
                alignment: TextAlignment::Start,
            }),
            fonts: None,
        }
    }

    pub(super) fn spec(&self) -> &ParagraphSpec {
        self.cache.key()
    }

    pub(super) fn with_spans(&self, spans: Vec<SpanSpec>, fills: Vec<Option<Paint>>) -> Self {
        Self {
            cache: self.cache.map_key(|spec| spec.spans = spans),
            fills: fills.into(),
            fonts: self.fonts.clone(),
        }
    }

    fn rekey(mut self, change: impl FnOnce(&mut ParagraphSpec)) -> Self {
        self.cache = self.cache.map_key(change);
        self
    }

    /// Selects an explicit family; empty uses the library's generic default.
    pub fn family(self, family: impl Into<Arc<str>>) -> Self {
        let family = family.into();
        self.rekey(|spec| spec.family = family)
    }

    /// Sets finite logical size in (0,4096].
    pub fn size(self, size: f64) -> Self {
        self.rekey(|spec| spec.size = size.to_bits())
    }

    /// Selects OpenType weight; variable weight participates in glyph identity.
    pub fn weight(self, weight: FontWeight) -> Self {
        self.rekey(|spec| spec.weight = weight.0)
    }

    /// Sets a positive relative line-height multiplier.
    pub fn line_height(self, height: f64) -> Self {
        self.rekey(|spec| spec.line_height = height.to_bits())
    }

    /// Sets logical tracking, converted to em internally for shaping.
    pub fn letter_spacing(self, spacing: f64) -> Self {
        self.rekey(|spec| spec.spacing = spacing.to_bits())
    }

    /// Wraps to finite positive logical width.
    pub fn max_width(self, width: f64) -> Self {
        self.rekey(|spec| spec.width = Some(width.to_bits()))
    }

    /// Selects bidirectional-aware paragraph alignment.
    pub fn alignment(self, alignment: TextAlignment) -> Self {
        self.rekey(|spec| spec.alignment = alignment)
    }

    /// Binds an explicit font snapshot, overriding a DrawContext environment.
    pub fn fonts(mut self, fonts: &FontLibrary) -> Self {
        self.fonts = Some(fonts.clone());
        self
    }
}
