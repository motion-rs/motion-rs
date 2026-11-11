//! Bounded SVG XML import into the shared semantic vector model.
//!
//! ```
//! use motion_content::prelude::*;
//! use motion_core::prelude::*;
//! use motion_core::{
//!     DisplayList, RasterLimits, ResourceStore, ValidatedDisplayList, MAX_PATH_VERBS,
//!     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
//! };
//! use motion_render::{Composition, RenderPrepare, RenderOptions, Renderer, render_still};
//! use motion_content::{Svg, SvgAsset};
//! use std::sync::Arc;
//! let asset = Arc::new(SvgAsset::from_bytes(br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="4"><rect id="mark" width="8" height="4" fill="blue"/></svg>"#)?);
//! let node = Svg::from_asset(asset)
//!     .select("#mark", |node| node.opacity(0.5))
//!     .write(1.0)
//!     .fit(Size::new(8.0, 8.0))
//!     .try_node()?;
//! let frame = node
//!     .compile(
//!         Size::new(8.0, 8.0),
//!         PixelSize::new(8, 8),
//!         ValidationLimits::default(),
//!     )?
//!     .prepare(ValidationLimits::default(), RasterLimits::default())?
//!     .render_linear(RasterLimits::default())?;
//! assert_eq!(frame.linear_pixel(2, 3), Some([0.0, 0.0, 0.5, 0.5]));
//! # Ok::<(), motion_core::Error>(())
//! ```

use crate::{VectorGroup, VectorItem, VectorPath, VectorSelection, VectorTree};

use motion_core::{Color, FillRule, Path2D, Point, Rect, Size, Transform};
use std::{
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

mod convert;
mod filters;
#[cfg(test)]
mod tests;
mod view;

use convert::{group, rect};
use filters::filter_group;

pub use view::Svg;

/// Maximum accepted SVG source size in bytes.
const MAX_SOURCE: usize = 8 * 1024 * 1024;

/// Immutable SVG viewport and semantic content. Parser coordinates are
/// f32 (usvg); converted geometry/authoring matrices are retained as f64.
#[derive(Clone, Debug)]
pub struct SvgAsset {
    /// Authored viewBox, or the intrinsic viewport when omitted.
    pub view_box: Rect,
    /// Canonical viewport-normalized content with selection IDs.
    pub tree: Arc<VectorTree>,
    diagnostics: Arc<[SvgDiagnostic]>,
    size: Size,
}

/// A retained unsupported-feature warning, never a silent raster substitution.
#[derive(Clone, Debug)]
pub struct SvgDiagnostic {
    /// Referenced resource or element ID, if authored.
    pub element: Option<Arc<str>>,
    /// Explicit reason the resource could not be mapped.
    pub message: Arc<str>,
}

impl SvgAsset {
    /// Parses without system discovery, files, scripts or external resources.
    pub fn from_bytes(bytes: &[u8]) -> motion_core::Result<Self> {
        Self::from_bytes_with_fonts(bytes, &crate::FontLibrary::new())
    }

    /// Parses using the same immutable font database as paragraph text.
    pub fn from_bytes_with_fonts(
        bytes: &[u8],
        fonts: &crate::FontLibrary,
    ) -> motion_core::Result<Self> {
        Self::decode(bytes, fonts.database.clone())
    }

    pub(crate) fn decode(bytes: &[u8], fonts: Arc<fontdb::Database>) -> motion_core::Result<Self> {
        if bytes.len() > MAX_SOURCE {
            return Err(unsupported("SVG source exceeds its encoded byte allowance"));
        }
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        let text =
            std::str::from_utf8(bytes).map_err(|error| motion_core::Error::Svg(Box::new(error)))?;
        let document = usvg::roxmltree::Document::parse_with_options(
            text,
            usvg::roxmltree::ParsingOptions {
                allow_dtd: false,
                nodes_limit: 100000,
                ..Default::default()
            },
        )
        .map_err(|error| motion_core::Error::Svg(Box::new(error)))?;
        for node in document.descendants() {
            if node.ancestors().count() > 64 {
                return Err(unsupported("SVG XML exceeds depth 64"));
            }
            if node.is_element() {
                if matches!(
                    node.tag_name().name(),
                    "script"
                        | "foreignObject"
                        | "animate"
                        | "animateMotion"
                        | "animateTransform"
                        | "set"
                ) {
                    return Err(unsupported(
                        "Only static SVG without scripts or foreign objects is supported",
                    ));
                }
                if node.attributes().any(|attribute| {
                    attribute.name() == "href"
                        && !attribute.value().is_empty()
                        && !attribute.value().starts_with('#')
                        && !attribute.value().starts_with("data:")
                }) {
                    return Err(unsupported("External SVG references are not supported"));
                }
            }
        }
        let has_text = document.descendants().any(|node| node.has_tag_name("text"));
        if fonts.faces().next().is_none() && has_text {
            return Err(unsupported(
                "SVG text requires an explicitly registered FontLibrary",
            ));
        }
        let failed = AtomicBool::new(false);
        let pixels = AtomicUsize::new(0);
        let nesting = AtomicUsize::new(0);
        let options = usvg::Options {
            fontdb: fonts,
            image_href_resolver: usvg::ImageHrefResolver {
                resolve_string: Box::new(|_, _| {
                    failed.store(true, Ordering::Relaxed);
                    None
                }),
                resolve_data: Box::new(|mime, data, options| {
                    let result = (|| {
                        if data.len() > MAX_SOURCE {
                            return None;
                        }
                        if mime == "image/svg+xml" {
                            let depth = nesting.fetch_add(1, Ordering::Relaxed);
                            if depth >= 8 {
                                nesting.fetch_sub(1, Ordering::Relaxed);
                                return None;
                            }
                            let parsed = (|| {
                                let text = std::str::from_utf8(&data).ok()?;
                                let doc = usvg::roxmltree::Document::parse_with_options(
                                    text,
                                    usvg::roxmltree::ParsingOptions {
                                        allow_dtd: false,
                                        nodes_limit: 20000,
                                        ..Default::default()
                                    },
                                )
                                .ok()?;
                                for node in doc.descendants().filter(|node| node.is_element()) {
                                    if node.ancestors().count() > 64
                                        || matches!(
                                            node.tag_name().name(),
                                            "script"
                                                | "foreignObject"
                                                | "animate"
                                                | "animateMotion"
                                                | "animateTransform"
                                                | "set"
                                        )
                                        || node.attributes().any(|attribute| {
                                            attribute.name() == "href"
                                                && !attribute.value().is_empty()
                                                && !attribute.value().starts_with('#')
                                                && !attribute.value().starts_with("data:")
                                        })
                                    {
                                        return None;
                                    }
                                }
                                Some(usvg::ImageKind::SVG(
                                    usvg::Tree::from_xmltree(&doc, options).ok()?,
                                ))
                            })();
                            nesting.fetch_sub(1, Ordering::Relaxed);
                            return parsed;
                        }
                        let reader = image::ImageReader::new(std::io::Cursor::new(data.as_slice()))
                            .with_guessed_format()
                            .ok()?;
                        let format = reader.format()?;

                        use image::ImageDecoder as _;
                        let dimensions = reader.into_decoder().ok()?.dimensions();
                        let count = dimensions.0 as usize * dimensions.1 as usize;
                        pixels
                            .try_update(Ordering::Relaxed, Ordering::Relaxed, |total| {
                                total
                                    .checked_add(count)
                                    .filter(|total| *total <= 16_777_216)
                            })
                            .ok()?;
                        Some(match format {
                            image::ImageFormat::Png => usvg::ImageKind::PNG(data),
                            image::ImageFormat::Jpeg => usvg::ImageKind::JPEG(data),
                            image::ImageFormat::WebP => usvg::ImageKind::WEBP(data),
                            _ => return None,
                        })
                    })();
                    if result.is_none() {
                        failed.store(true, Ordering::Relaxed);
                    }
                    result
                }),
            },
            ..Default::default()
        };
        let parsed = usvg::Tree::from_xmltree(&document, &options)
            .map_err(|error| motion_core::Error::Svg(Box::new(error)))?;
        if failed.load(Ordering::Relaxed) {
            return Err(unsupported(
                "Invalid, external, oversized or unsupported SVG image resource",
            ));
        }
        let size = Size::new(
            f64::from(parsed.size().width()),
            f64::from(parsed.size().height()),
        );
        let viewport = size.to_rect();
        let view_box = if let Some(value) = document.root_element().attribute("viewBox") {
            let value = svgtypes::ViewBox::from_str(value)
                .map_err(|error| motion_core::Error::Svg(Box::new(error)))?;
            Rect::new(value.x, value.y, value.x + value.w, value.y + value.h)
        } else {
            viewport
        };
        let mut count = 0;
        let mut diagnostics = Vec::new();
        let root = group(parsed.root(), 0, &mut count, &mut diagnostics)?;
        let mut tree = VectorTree::new(viewport, root)?;
        if let Some(root_id) = document
            .root_element()
            .attribute("id")
            .filter(|id| !id.is_empty())
            && !tree.ids().contains_key(root_id)
        {
            let mut root = tree.root().clone();
            root.id = Some(root_id.into());
            tree = VectorTree::new(viewport, root)?;
        }
        Ok(Self {
            view_box,
            tree: Arc::new(tree),
            diagnostics: diagnostics.into(),
            size,
        })
    }

    /// Intrinsic viewport size after SVG units and viewBox mapping.
    pub fn size(&self) -> Size {
        self.size
    }

    /// Unsupported filter resources skipped as a whole, with retained reasons.
    pub fn diagnostics(&self) -> &[SvgDiagnostic] {
        &self.diagnostics
    }
}

impl motion_core::Asset for SvgAsset {
    const KIND: &'static str = "svg";

    fn decode(bytes: &[u8], context: &motion_core::LoadContext) -> motion_core::Result<Self> {
        if let Some(fonts) = context.fonts() {
            let fonts = fonts.downcast_ref::<crate::FontLibrary>().ok_or_else(|| {
                unsupported("SVG received an incompatible frozen font environment")
            })?;
            Self::from_bytes_with_fonts(bytes, fonts)
        } else {
            Self::from_bytes(bytes)
        }
    }

    fn retained_bytes(&self) -> usize {
        fn paint_bytes(paint: &motion_core::Paint) -> usize {
            paint.gradient().map_or(0, |gradient| {
                gradient.stops.len() * std::mem::size_of::<(f64, Color)>()
            })
        }

        fn bytes(group: &VectorGroup) -> usize {
            let mut total = std::mem::size_of::<VectorGroup>()
                + group
                    .filters
                    .iter()
                    .map(|filter| {
                        std::mem::size_of::<motion_core::Filter>() + filter.heap_bytes() as usize
                    })
                    .sum::<usize>()
                + group.id.as_ref().map_or(0, |id| id.len())
                + group.children.len() * std::mem::size_of::<VectorItem>();
            if let Some(clip) = &group.clip {
                total += std::mem::size_of_val(clip.path.verbs());
            }
            if let Some(mask) = &group.mask {
                total += bytes(mask.tree.root());
            }
            for item in &group.children {
                total += match item {
                    VectorItem::Group(group) => bytes(group),
                    VectorItem::Path(path) => {
                        std::mem::size_of_val(path.path.verbs())
                            + path.id.as_ref().map_or(0, |id| id.len())
                            + path
                                .fill
                                .as_ref()
                                .map_or(0, |(paint, _)| paint_bytes(paint))
                            + path
                                .stroke
                                .as_ref()
                                .map_or(0, |(_, paint)| paint_bytes(paint))
                    }
                    VectorItem::Image(image) => {
                        image.image.retained_bytes() + image.id.as_ref().map_or(0, |id| id.len())
                    }
                };
            }
            total
        }
        bytes(self.tree.root())
            + self
                .tree
                .ids()
                .iter()
                .map(|(id, address)| id.len() + address.len() * std::mem::size_of::<usize>())
                .sum::<usize>()
            + self
                .diagnostics
                .iter()
                .map(|diagnostic| {
                    std::mem::size_of::<SvgDiagnostic>()
                        + diagnostic.message.len()
                        + diagnostic.element.as_ref().map_or(0, |id| id.len())
                })
                .sum::<usize>()
    }
}

fn unsupported(message: impl Into<String>) -> motion_core::Error {
    motion_core::ValidationError::unsupported("svg", "content", message.into()).into()
}

fn matrix(value: usvg::Transform) -> Transform {
    Transform::from_coefficients([
        f64::from(value.sx),
        f64::from(value.ky),
        f64::from(value.kx),
        f64::from(value.sy),
        f64::from(value.tx),
        f64::from(value.ty),
    ])
}

fn geometry(source: &usvg::tiny_skia_path::Path) -> Result<Path2D, motion_core::ValidationError> {
    use usvg::tiny_skia_path::PathSegment;
    let mut path = Path2D::new();
    let point =
        |point: usvg::tiny_skia_path::Point| Point::new(f64::from(point.x), f64::from(point.y));
    for segment in source.segments() {
        match segment {
            PathSegment::MoveTo(end) => path.try_move_to(point(end)),
            PathSegment::LineTo(end) => path.try_line_to(point(end)),
            PathSegment::QuadTo(control, end) => path.try_quadratic_to(point(control), point(end)),
            PathSegment::CubicTo(first, second, end) => {
                path.try_cubic_to(point(first), point(second), point(end))
            }
            PathSegment::Close => path.try_close(),
        }?;
    }
    Ok(path)
}

fn id(value: &str) -> Option<Arc<str>> {
    (!value.is_empty()).then(|| Arc::from(value))
}
