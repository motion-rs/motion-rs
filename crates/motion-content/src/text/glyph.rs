//! Glyph outline and COLR color-glyph extraction into vector artwork.

use motion_core::{Path2D, Point, ValidationError};
use skrifa::{
    MetadataProvider,
    instance::{LocationRef, Size},
    outline::{DrawSettings, OutlinePen},
};
use std::sync::Arc;
use ttf_parser::colr;

#[derive(Clone, Debug)]
pub(crate) struct Artwork {
    pub font: motion_core::FontIdentity,
    pub path: Arc<Path2D>,
    pub units: f64,
    pub layers: Vec<(Arc<Path2D>, Option<motion_core::Color>)>,
    pub bitmap: Option<(Arc<crate::ImageAsset>, motion_core::Transform)>,
}

impl Artwork {
    pub(crate) fn load(
        library: &crate::FontLibrary,
        system: &mut cosmic_text::FontSystem,
        id: cosmic_text::fontdb::ID,
        weight: cosmic_text::Weight,
        glyph: u16,
    ) -> Result<Arc<Self>, ValidationError> {
        let key = (library.snapshot_id(), id, weight.0, glyph);
        if let Some(value) = library.glyph_cache.lookup(&key) {
            return Ok(value);
        }
        let index = system
            .db()
            .face(id)
            .ok_or_else(|| super::invalid("font", "Layout referenced an unknown font"))?
            .index;
        let font = system.get_font(id, weight).ok_or_else(|| {
            super::invalid("font", "Unable to read an explicitly registered font")
        })?;
        let source = library.record_font(system, id, font.data());
        let parser = ttf_parser::Face::parse(font.data(), index)
            .map_err(|_| super::invalid("font", "Invalid shaped font face"))?;
        let reference = skrifa::FontRef::from_index(font.data(), index)
            .map_err(|_| super::invalid("font", "Invalid outline font face"))?;
        let mut path = Path2D::new();
        let mut layers = Vec::new();
        let mut bitmap = None;
        let units = f64::from(parser.units_per_em());
        if parser.is_color_glyph(ttf_parser::GlyphId(glyph)) {
            let table = parser
                .raw_face()
                .table(ttf_parser::Tag::from_bytes(b"COLR"))
                .unwrap_or_default();
            if table.get(..2) != Some(&[0, 0][..]) {
                return Err(ValidationError::unsupported(
                    "text",
                    "color_glyph",
                    "Only COLRv0 is supported; COLRv1 is not silently discarded",
                ));
            }
            let mut first = Colors::default();
            let mut second = Colors::default();
            parser
                .paint_color_glyph(
                    ttf_parser::GlyphId(glyph),
                    0,
                    ttf_parser::RgbaColor::new(0, 0, 0, 0),
                    &mut first,
                )
                .ok_or_else(|| super::invalid("color_glyph", "Malformed COLRv0 glyph"))?;
            parser
                .paint_color_glyph(
                    ttf_parser::GlyphId(glyph),
                    0,
                    ttf_parser::RgbaColor::new(255, 255, 255, 255),
                    &mut second,
                )
                .ok_or_else(|| super::invalid("color_glyph", "Malformed COLRv0 glyph"))?;
            if first.error || second.error || first.layers.len() != second.layers.len() {
                return Err(super::limited(
                    "Color glyph exceeds its 256-layer allowance",
                ));
            }
            for ((glyph, color), (_, alternate)) in first.layers.into_iter().zip(second.layers) {
                let paint = if color == alternate {
                    Some(motion_core::Color::rgba(
                        color.red,
                        color.green,
                        color.blue,
                        color.alpha,
                    ))
                } else {
                    None
                };
                layers.push((Arc::new(outline(&reference, weight, glyph.0)?), paint));
            }
        } else if let Some(image) = parser.glyph_raster_image(ttf_parser::GlyphId(glyph), 128) {
            if image.pixels_per_em == 0
                || image.width == 0
                || image.height == 0
                || u32::from(image.width) * u32::from(image.height) > 1024 * 1024
            {
                return Err(super::limited("Invalid or oversized bitmap glyph strike"));
            }
            let decoded = match image.format {
                ttf_parser::RasterImageFormat::PNG => crate::ImageAsset::from_bytes(image.data)
                    .map_err(|_| super::invalid("bitmap", "Invalid bitmap glyph PNG"))?,
                ttf_parser::RasterImageFormat::BitmapPremulBgra32 => {
                    if image.data.len() != usize::from(image.width) * usize::from(image.height) * 4
                    {
                        return Err(super::invalid("bitmap", "Invalid BGRA glyph payload"));
                    }
                    let mut pixels = Vec::with_capacity(image.data.len());
                    for pixel in image.data.as_chunks::<4>().0 {
                        let alpha = u32::from(pixel[3]);
                        for channel in [pixel[2], pixel[1], pixel[0]] {
                            pixels.push(
                                (u32::from(channel) * 255 + alpha / 2)
                                    .checked_div(alpha)
                                    .unwrap_or(0)
                                    .min(255) as u8,
                            );
                        }
                        pixels.push(pixel[3]);
                    }
                    crate::ImageAsset::from_rgba8(
                        u32::from(image.width),
                        u32::from(image.height),
                        pixels,
                    )
                    .map_err(|_| super::invalid("bitmap", "Invalid BGRA glyph"))?
                }
                _ => {
                    return Err(ValidationError::unsupported(
                        "text",
                        "bitmap",
                        "Only PNG and premultiplied BGRA color glyph strikes are supported",
                    ));
                }
            };
            if decoded.width() != u32::from(image.width)
                || decoded.height() != u32::from(image.height)
            {
                return Err(super::invalid(
                    "bitmap",
                    "Bitmap glyph dimensions disagree with strike metrics",
                ));
            }
            let scale = units / f64::from(image.pixels_per_em);
            let transform = motion_core::Transform::scale_non_uniform(scale, -scale).then(
                motion_core::Transform::translate(
                    f64::from(image.x) * scale,
                    (f64::from(image.y) + f64::from(image.height)) * scale,
                ),
            );
            bitmap = Some((Arc::new(decoded), transform));
        } else {
            path = outline(&reference, weight, glyph)?;
            if path.verbs().is_empty()
                && parser.glyph_svg_image(ttf_parser::GlyphId(glyph)).is_some()
            {
                return Err(ValidationError::unsupported(
                    "text",
                    "color_glyph",
                    "SVG-in-OpenType glyphs are not yet supported",
                ));
            }
        }
        let bytes = (path.verbs().len()
            + layers
                .iter()
                .map(|(path, _)| path.verbs().len())
                .sum::<usize>())
            * std::mem::size_of::<motion_core::PathVerb>()
            + bitmap
                .as_ref()
                .map_or(0, |(image, _)| image.retained_bytes());
        if bytes > 64 * 1024 * 1024 {
            return Err(super::invalid(
                "outline",
                "Glyph outline exceeds cache allowance",
            ));
        }
        let result = Self {
            font: source,
            path: Arc::new(path),
            units,
            layers,
            bitmap,
        };
        Ok(library
            .glyph_cache
            .insert_with_cost(key, result, |_| bytes + std::mem::size_of::<Self>()))
    }

    pub(crate) fn bounds(
        &self,
        transform: motion_core::Transform,
    ) -> Result<Option<motion_core::Rect>, ValidationError> {
        let mut bounds = None;
        for path in std::iter::once(&self.path)
            .chain(self.layers.iter().map(|(path, _)| path))
            .filter(|path| !path.verbs().is_empty())
        {
            let mapped = motion_core::display_list::mapped_bounds(
                path.bounding_box(),
                transform,
                "text.glyph",
            )?;
            bounds = Some(bounds.map_or(mapped, |bounds: motion_core::Rect| bounds.union(mapped)));
        }
        if let Some((image, local)) = &self.bitmap {
            let mapped = motion_core::display_list::mapped_bounds(
                motion_core::Rect::new(
                    0.0,
                    0.0,
                    f64::from(image.width()),
                    f64::from(image.height()),
                ),
                local.then(transform),
                "text.bitmap",
            )?;
            bounds = Some(bounds.map_or(mapped, |bounds| bounds.union(mapped)));
        }
        Ok(bounds)
    }
}

fn outline(
    reference: &skrifa::FontRef<'_>,
    weight: cosmic_text::Weight,
    glyph: u16,
) -> Result<Path2D, ValidationError> {
    let mut pen = Pen::default();
    if let Some(outline) = reference
        .outline_glyphs()
        .get(skrifa::GlyphId::new(u32::from(glyph)))
    {
        let location = reference.axes().location(&[("wght", f32::from(weight.0))]);
        outline
            .draw(
                DrawSettings::unhinted(Size::unscaled(), LocationRef::new(location.coords())),
                &mut pen,
            )
            .map_err(|_| super::invalid("outline", "Unable to extract unhinted glyph outline"))?;
    }
    if let Some(error) = pen.error {
        return Err(error);
    }
    Ok(pen.path)
}

#[derive(Default)]
struct Colors {
    current: Option<ttf_parser::GlyphId>,
    layers: Vec<(ttf_parser::GlyphId, ttf_parser::RgbaColor)>,
    error: bool,
}

impl<'a> colr::Painter<'a> for Colors {
    fn outline_glyph(&mut self, glyph: ttf_parser::GlyphId) {
        self.current = Some(glyph);
    }

    fn paint(&mut self, paint: colr::Paint<'a>) {
        if let (Some(glyph), colr::Paint::Solid(color)) = (self.current, paint) {
            if self.layers.len() < 256 {
                self.layers.push((glyph, color));
            } else {
                self.error = true;
            }
        } else {
            self.error = true;
        }
    }

    fn push_clip(&mut self) {
        self.error = true;
    }

    fn push_clip_box(&mut self, _: colr::ClipBox) {
        self.error = true;
    }

    fn pop_clip(&mut self) {
        self.error = true;
    }

    fn push_layer(&mut self, _: colr::CompositeMode) {
        self.error = true;
    }

    fn pop_layer(&mut self) {
        self.error = true;
    }

    fn push_transform(&mut self, _: ttf_parser::Transform) {
        self.error = true;
    }

    fn pop_transform(&mut self) {
        self.error = true;
    }
}

#[derive(Default)]
pub(crate) struct Pen {
    pub path: Path2D,
    pub error: Option<ValidationError>,
}

impl Pen {
    fn push(&mut self, verb: motion_core::PathVerb) {
        if self.error.is_some() {
            return;
        }
        self.error = match verb {
            motion_core::PathVerb::MoveTo(value) => self.path.try_move_to(value),
            motion_core::PathVerb::LineTo(value) => self.path.try_line_to(value),
            motion_core::PathVerb::QuadraticTo(control, end) => {
                self.path.try_quadratic_to(control, end)
            }
            motion_core::PathVerb::CubicTo(first, second, end) => {
                self.path.try_cubic_to(first, second, end)
            }
            motion_core::PathVerb::Close => self.path.try_close(),
        }
        .err();
    }
}

impl OutlinePen for Pen {
    fn move_to(&mut self, horizontal: f32, vertical: f32) {
        self.push(motion_core::PathVerb::MoveTo(Point::new(
            f64::from(horizontal),
            f64::from(vertical),
        )));
    }

    fn line_to(&mut self, horizontal: f32, vertical: f32) {
        self.push(motion_core::PathVerb::LineTo(Point::new(
            f64::from(horizontal),
            f64::from(vertical),
        )));
    }

    fn quad_to(&mut self, control_x: f32, control_y: f32, horizontal: f32, vertical: f32) {
        self.push(motion_core::PathVerb::QuadraticTo(
            Point::new(f64::from(control_x), f64::from(control_y)),
            Point::new(f64::from(horizontal), f64::from(vertical)),
        ));
    }

    fn curve_to(
        &mut self,
        first_x: f32,
        first_y: f32,
        second_x: f32,
        second_y: f32,
        horizontal: f32,
        vertical: f32,
    ) {
        self.push(motion_core::PathVerb::CubicTo(
            Point::new(f64::from(first_x), f64::from(first_y)),
            Point::new(f64::from(second_x), f64::from(second_y)),
            Point::new(f64::from(horizontal), f64::from(vertical)),
        ));
    }

    fn close(&mut self) {
        self.push(motion_core::PathVerb::Close);
    }
}
