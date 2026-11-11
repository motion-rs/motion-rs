//! Optional in-memory Typst compilation with bundled fonts and no external files.

use std::{io, sync::Arc};

use typst::{
    Library, LibraryExt, World,
    diag::{FileError, FileResult},
    foundations::{Bytes, Datetime, Duration},
    syntax::{FileId, Source},
    text::{Font, FontBook},
    utils::LazyHash,
};

use motion_core::Size;

const MAX_SOURCE_BYTES: usize = 64 * 1024;

const MAX_SVG_BYTES: usize = 64 * 1024 * 1024;

/// In-process Typst compiler, available with feature `typst`.
///
/// Uses bundled fonts without system font discovery or a CLI. Keep this compiler
/// and its compiled documents outside frame callbacks. Only the supplied source
/// is available: imports, external files/packages and current-date access fail.
/// Compile trusted source only; input/output limits do not bound compiler time,
/// temporary memory or Typst's internal memoization.
#[derive(Clone)]
pub struct TypstCompiler {
    environment: Arc<Environment>,
}

struct Environment {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
}

/// A single compiled Typst page with outlined SVG artwork and warnings.
///
/// Clones share SVG bytes; [`Self::node`] enters the vector pipeline.
#[derive(Clone, Debug)]
pub struct TypstDocument {
    svg: Arc<str>,
    size: Size,
    warnings: Arc<[String]>,
}

impl Default for TypstCompiler {
    fn default() -> Self {
        Self::new()
    }
}

impl TypstCompiler {
    /// Loads the bundled font book and standard library in memory.
    /// Cloning the compiler shares both; no filesystem or network access occurs.
    pub fn new() -> Self {
        let fonts: Vec<_> = typst_assets::fonts()
            .flat_map(|data| Font::iter(Bytes::new(data)))
            .collect();
        let book = LazyHash::new(FontBook::from_fonts(fonts.iter()));
        Self {
            environment: Arc::new(Environment {
                library: LazyHash::new(Library::default()),
                book,
                fonts,
            }),
        }
    }

    /// Creates a new frozen compiler environment extended with explicitly loaded
    /// FontLibrary bytes. Intrinsic family names are used; no system discovery or
    /// file reads occur here. Existing compilers/documents remain unchanged.
    pub fn with_fonts(&self, library: &motion_content::FontLibrary) -> motion_core::Result<Self> {
        let mut fonts = Vec::new();
        let mut buffers = std::collections::BTreeMap::new();
        for face in library.database().faces() {
            let fontdb::Source::Binary(data) = &face.source else {
                return Err(invalid(
                    "Typst font extension requires frozen binary FontLibrary sources",
                )
                .into());
            };
            let source = data.as_ref().as_ref();
            let bytes = buffers
                .entry((source.as_ptr() as usize, source.len()))
                .or_insert_with(|| Bytes::new(source.to_vec()))
                .clone();
            let font = Font::new(bytes, face.index)
                .ok_or_else(|| invalid("Invalid explicit Typst font face"))?;
            fonts.push(font);
        }
        fonts.extend(self.environment.fonts.iter().cloned());
        let book = LazyHash::new(FontBook::from_fonts(fonts.iter()));
        Ok(Self {
            environment: Arc::new(Environment {
                library: self.environment.library.clone(),
                book,
                fonts,
            }),
        })
    }

    /// Compiles at most 64 KiB of trusted Typst markup into exactly one page.
    ///
    /// Use `#set page(width: auto, height: auto, margin: 0pt, fill: none)`
    /// for compact transparent formulas. Source uses Typst units; [`Size`] is
    /// reported at 96 pixels/inch (one point is 4/3 pixel). No math-baseline
    /// metadata is inferred from the page bounds. Inspect document warnings for
    /// missing fonts, glyphs and other nonfatal compilation diagnostics.
    ///
    /// # Errors
    /// Returns malformed/oversized source, unavailable resources, compilation,
    /// multiple-page, invalid page-dimension or oversized-SVG errors. Compiler
    /// allocations happen before output limits can be checked.
    pub fn compile(&self, source: &str) -> motion_core::Result<TypstDocument> {
        if source.len() > MAX_SOURCE_BYTES {
            return Err(invalid("Typst source exceeds 64 KiB").into());
        }
        let compiled = self.layout(Source::detached(source));
        let document = compiled.output.map_err(|diagnostics| {
            motion_core::Error::formula(format!(
                "Typst compilation failed: {}",
                diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.message.to_string())
                    .collect::<Vec<_>>()
                    .join("; ")
            ))
        })?;
        if document.pages().len() != 1 {
            return Err(invalid("Typst assets must contain exactly one page").into());
        }
        let page = &document.pages()[0];
        let dimensions = page.frame.size();
        let width = dimensions.x.to_pt() * (96.0 / 72.0);
        let height = dimensions.y.to_pt() * (96.0 / 72.0);
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err(invalid("Typst page dimensions must be finite and positive").into());
        }
        let size = Size::new(width, height);
        if !size.width.is_finite() || !size.height.is_finite() {
            return Err(invalid("Typst page dimensions exceed layout range").into());
        }
        let svg = typst_svg::svg(page, &typst_svg::SvgOptions::default());
        if svg.len() > MAX_SVG_BYTES {
            return Err(invalid("Typst SVG exceeds 64 MiB").into());
        }
        Ok(TypstDocument {
            svg: svg.into(),
            size,
            warnings: compiled
                .warnings
                .iter()
                .map(|diagnostic| diagnostic.message.to_string())
                .collect::<Vec<_>>()
                .into(),
        })
    }

    pub(crate) fn layout(
        &self,
        source: Source,
    ) -> typst::diag::Warned<typst::diag::SourceResult<typst_layout::PagedDocument>> {
        typst::compile::<typst_layout::PagedDocument>(&DocumentWorld {
            environment: self.environment.clone(),
            source,
        })
    }
}

impl TypstDocument {
    /// Returns self-contained SVG with outlined glyphs, without rasterization.
    pub fn svg(&self) -> &str {
        &self.svg
    }

    /// Returns page dimensions in pixels at 96 pixels/inch, not tight ink bounds.
    pub fn size(&self) -> Size {
        self.size
    }

    /// Returns nonfatal compiler diagnostics, including missing-font warnings.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Converts the outlined page through the SVG/VectorTree importer.
    /// Cache the returned node outside frame callbacks for repeated rendering.
    /// Unsupported SVG output returns its structured import error, not a raster fallback.
    pub fn node(&self) -> motion_core::Result<motion_core::Node> {
        motion_content::Svg::from_asset(Arc::new(motion_content::SvgAsset::from_bytes(
            self.svg.as_bytes(),
        )?))
        .try_node()
    }
}

struct DocumentWorld {
    environment: Arc<Environment>,
    source: Source,
}

impl World for DocumentWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.environment.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.environment.book
    }

    fn main(&self) -> FileId {
        self.source.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main() {
            Ok(self.source.clone())
        } else {
            Err(FileError::AccessDenied)
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        if id == self.main() {
            Ok(Bytes::from_string(self.source.text().to_owned()))
        } else {
            Err(FileError::AccessDenied)
        }
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.environment.fonts.get(index).cloned()
    }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        None
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_render::RenderPrepare;

    #[test]
    fn outlined_page_keeps_shared_svg_dimensions_pixels_and_bounded_sources() -> motion_core::Result
    {
        let compiler = TypstCompiler::new();
        let page = compiler.compile(
            "#set page(width: auto, height: auto, margin: 0pt, fill: none)\n$frac(1, 2) + sqrt(x)$",
        )?;
        assert!(page.size().width > 0.0 && page.size().height > 0.0);
        assert!(Arc::ptr_eq(&page.svg, &page.clone().svg));
        assert!(page.svg().contains("<svg"));
        let size = motion_core::PixelSize::new(
            page.size().width.ceil() as u32,
            page.size().height.ceil() as u32,
        );
        let node = page.node()?;
        let render = || -> motion_core::Result<motion_render::FrameBuffer> {
            Ok(node
                .compile(page.size(), size, motion_core::ValidationLimits::default())?
                .prepare(
                    motion_core::ValidationLimits::default(),
                    motion_core::RasterLimits::default(),
                )?
                .render_linear(motion_core::RasterLimits::default())?)
        };
        let frame = render()?;
        assert!(frame.linear_pixels().any(|pixel| pixel[3] > 0.0));
        assert_eq!(frame, render()?);
        for source in [
            "$frac($",
            "#read(\"unavailable\")",
            "#import \"unavailable\": *",
            "#datetime.today()",
            "First#pagebreak()Second",
        ] {
            assert!(compiler.compile(source).is_err(), "{source}");
        }
        assert!(compiler.compile(&" ".repeat(MAX_SOURCE_BYTES + 1)).is_err());
        Ok(())
    }
}
