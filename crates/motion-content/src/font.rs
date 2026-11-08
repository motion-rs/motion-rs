//! Explicit immutable font snapshots shared by text and SVG parsing.

use std::sync::{
    Arc, Mutex, MutexGuard, OnceLock,
    atomic::{AtomicU64, Ordering},
};

#[cfg(test)]
#[path = "font/tests.rs"]
mod explicit_face_tests;

/// Maximum source bytes accepted by one explicitly loaded font.
pub const MAX_FONT_FACE_BYTES: usize = 32 * 1024 * 1024;

/// A face identifier within one immutable font-library snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FontIdentifier(pub(crate) fontdb::ID);

/// A copy-on-write font database. Construction never discovers system fonts.
/// Registered source bytes are retained; cloned libraries share snapshots.
#[derive(Clone)]
pub struct FontLibrary {
    pub(crate) database: Arc<fontdb::Database>,
    bytes: usize,
    snapshot_id: u64,
    font_system: Arc<OnceLock<Mutex<cosmic_text::FontSystem>>>,
    pub(crate) glyph_cache: Arc<motion_core::SceneCache>,
    pub(crate) layout_cache: crate::text::SharedLayouts,
    used_fonts: Arc<
        Mutex<
            std::collections::BTreeMap<cosmic_text::fontdb::ID, (motion_core::FontIdentity, usize)>,
        >,
    >,
}

impl FontLibrary {
    /// Creates an empty library with no filesystem access or discovery.
    pub fn new() -> Self {
        Self {
            database: Arc::new(fontdb::Database::new()),
            bytes: 0,
            snapshot_id: identity(),
            font_system: Arc::new(OnceLock::new()),
            glyph_cache: Arc::new(motion_core::SceneCache::with_limits(
                motion_core::CacheLimits {
                    entries: 2048,
                    bytes: 16 * 1024 * 1024,
                },
            )),
            layout_cache: Default::default(),
            used_fonts: Arc::default(),
        }
    }

    /// Loads one explicit font file into immutable memory, never a lazy file source.
    pub fn load_file(
        &mut self,
        path: impl AsRef<std::path::Path>,
    ) -> motion_core::Result<Vec<FontIdentifier>> {
        use std::io::Read;
        let path = path.as_ref();
        let metadata = std::fs::metadata(path)?;
        if !metadata.is_file() || metadata.len() > 256 * 1024 * 1024 {
            return Err(invalid("Font file must be regular and at most 256 MiB"));
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(256 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        self.load_data(bytes.into())
            .map_err(|error| motion_core::Error::asset("font", path.display().to_string(), error))
    }

    /// Registers bounded in-memory OpenType/TrueType data atomically, including
    /// collections and color fonts. At most 4096 faces / 256 MiB are admitted.
    pub fn load_data(&mut self, data: Arc<[u8]>) -> motion_core::Result<Vec<FontIdentifier>> {
        let bytes = self
            .bytes
            .checked_add(data.len())
            .filter(|bytes| *bytes <= 256 * 1024 * 1024)
            .ok_or_else(|| invalid("Font-library source payload exceeds 256 MiB"))?;
        let mut database = self.database.as_ref().clone();
        let source: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(data);
        let ids = database.load_font_source(fontdb::Source::Binary(source));
        if ids.is_empty() {
            return Err(invalid("No valid font faces found"));
        }
        if database.faces().count() > 4096 {
            return Err(invalid("Font library exceeds 4096 faces"));
        }
        if self.is_empty() {
            let family = database
                .faces()
                .next()
                .and_then(|face| face.families.first())
                .map(|family| family.0.clone());
            if let Some(family) = family {
                database.set_sans_serif_family(family.clone());
                database.set_serif_family(family.clone());
                database.set_monospace_family(family);
            }
        }
        self.database = Arc::new(database);
        self.bytes = bytes;
        self.snapshot_id = identity();
        self.font_system = Arc::new(OnceLock::new());
        self.glyph_cache = Arc::new(motion_core::SceneCache::with_limits(
            motion_core::CacheLimits {
                entries: 2048,
                bytes: 16 * 1024 * 1024,
            },
        ));
        self.layout_cache = Default::default();
        self.used_fonts = Arc::default();
        Ok(ids.into_iter().map(FontIdentifier).collect())
    }

    /// Loads a sorted project directory tree atomically; symlink directories are
    /// not followed. At most 4096 entries are visited, independent of face count.
    pub fn load_dir(
        &mut self,
        directory: impl AsRef<std::path::Path>,
    ) -> motion_core::Result<usize> {
        let mut pending = vec![directory.as_ref().to_path_buf()];
        let mut files = Vec::new();
        let mut visited = 0;
        while let Some(directory) = pending.pop() {
            let mut entries = std::fs::read_dir(directory)?
                .take(4097 - visited)
                .collect::<Result<Vec<_>, _>>()?;
            entries.sort_by_key(|entry| entry.path());
            for entry in entries {
                visited += 1;
                if visited > 4096 {
                    return Err(invalid("Font directory exceeds 4096 visited entries"));
                }
                let kind = entry.file_type()?;
                if kind.is_dir() {
                    pending.push(entry.path());
                } else if kind.is_file()
                    && entry
                        .path()
                        .extension()
                        .and_then(|value| value.to_str())
                        .is_some_and(|value| {
                            matches!(
                                value.to_ascii_lowercase().as_str(),
                                "ttf" | "otf" | "ttc" | "otc"
                            )
                        })
                {
                    files.push(entry.path());
                }
            }
        }
        files.sort();
        let mut updated = self.clone();
        let before = updated.len();
        for file in files {
            updated.load_file(file)?;
        }
        let added = updated.len() - before;
        *self = updated;
        Ok(added)
    }

    /// Explicitly indexes installed font metadata without retaining every file.
    /// Font discovery still performs I/O; bytes are loaded on first use.
    pub fn load_system_fonts(&mut self) -> motion_core::Result<usize> {
        let mut discovered = fontdb::Database::new();
        discovered.load_system_fonts();
        self.index_faces(discovered.faces().cloned())
    }

    /// Explicitly discovers installed fonts, indexing only the requested family.
    pub fn load_system_family(&mut self, family: &str) -> motion_core::Result<usize> {
        if family.is_empty() || family.len() > 1024 {
            return Err(invalid("System font family must have 1..=1024 UTF-8 bytes"));
        }
        let mut discovered = fontdb::Database::new();
        discovered.load_system_fonts();
        let faces = discovered
            .faces()
            .filter(|face| {
                face.families
                    .iter()
                    .any(|(name, _)| name.eq_ignore_ascii_case(family))
            })
            .cloned()
            .collect::<Vec<_>>();
        if faces.is_empty() {
            return Err(invalid("Requested installed font family was not found"));
        }
        self.index_faces(faces)
    }

    /// Sets generic family aliases in a new snapshot, without font discovery.
    pub fn default_family(mut self, family: impl Into<String>) -> Self {
        let family = family.into();
        let mut database = self.database.as_ref().clone();
        database.set_sans_serif_family(family.clone());
        database.set_serif_family(family.clone());
        database.set_monospace_family(family);
        self.database = Arc::new(database);
        self.snapshot_id = identity();
        self.font_system = Arc::new(OnceLock::new());
        self.glyph_cache = Arc::new(motion_core::SceneCache::with_limits(
            motion_core::CacheLimits {
                entries: 2048,
                bytes: 16 * 1024 * 1024,
            },
        ));
        self.layout_cache = Default::default();
        self.used_fonts = Arc::default();
        self
    }

    /// Borrows the frozen parser database for SVG/formula import adapters.
    /// Mutation still requires a new FontLibrary snapshot.
    pub fn database(&self) -> &fontdb::Database {
        self.database.as_ref()
    }

    /// Identity of this immutable environment for resource/layout cache keys.
    pub fn snapshot_id(&self) -> u64 {
        self.snapshot_id
    }

    /// Registered face count, including collection faces.
    pub fn len(&self) -> usize {
        self.database.faces().count()
    }

    /// Whether no fonts have been registered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn index_faces(
        &mut self,
        faces: impl IntoIterator<Item = fontdb::FaceInfo>,
    ) -> motion_core::Result<usize> {
        let mut database = self.database.as_ref().clone();
        let before = self.len();
        for mut face in faces {
            if let fontdb::Source::SharedFile(path, _) = face.source {
                face.source = fontdb::Source::File(path);
            }
            if database.faces().count() >= 65_536 {
                return Err(invalid("Font metadata exceeds 65536 faces"));
            }
            database.push_face_info(face);
        }
        let added = database.faces().count() - before;
        self.database = Arc::new(database);
        self.snapshot_id = identity();
        self.font_system = Arc::new(OnceLock::new());
        self.layout_cache = Default::default();
        self.used_fonts = Arc::default();
        Ok(added)
    }

    pub(crate) fn with_render_cache(&self, cache: &motion_core::RenderCache) -> Self {
        let mut fonts = self.clone();
        fonts.glyph_cache = cache.partition(motion_core::CachePartition::Glyph).clone();
        fonts
    }

    /// Locks the lazily built shaping engine; the mutex itself is not exposed.
    pub(crate) fn lock_font_system(
        &self,
    ) -> Result<MutexGuard<'_, cosmic_text::FontSystem>, motion_core::ValidationError> {
        self.font_system_cell().lock().map_err(|_| {
            motion_core::ValidationError::invalid("fonts", "library", "Font engine is poisoned")
        })
    }

    fn font_system_cell(&self) -> &Mutex<cosmic_text::FontSystem> {
        self.font_system.get_or_init(|| {
            let mut database = cosmic_text::fontdb::Database::new();
            for face in self.database.faces() {
                let source = match &face.source {
                    fontdb::Source::Binary(data) => {
                        cosmic_text::fontdb::Source::Binary(data.clone())
                    }
                    fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => {
                        cosmic_text::fontdb::Source::File(path.clone())
                    }
                };
                database.push_face_info(cosmic_text::fontdb::FaceInfo {
                    id: cosmic_text::fontdb::ID::dummy(),
                    source,
                    index: face.index,
                    families: face
                        .families
                        .iter()
                        .map(|(name, language)| {
                            let language = if language.primary_language() == "English"
                                && language.region() == "United States"
                            {
                                cosmic_text::fontdb::Language::English_UnitedStates
                            } else {
                                cosmic_text::fontdb::Language::Unknown
                            };
                            (name.clone(), language)
                        })
                        .collect(),
                    post_script_name: face.post_script_name.clone(),
                    style: match face.style {
                        fontdb::Style::Normal => cosmic_text::fontdb::Style::Normal,
                        fontdb::Style::Italic => cosmic_text::fontdb::Style::Italic,
                        fontdb::Style::Oblique => cosmic_text::fontdb::Style::Oblique,
                    },
                    weight: cosmic_text::fontdb::Weight(face.weight.0),
                    stretch: match face.stretch.to_number() {
                        1 => cosmic_text::fontdb::Stretch::UltraCondensed,
                        2 => cosmic_text::fontdb::Stretch::ExtraCondensed,
                        3 => cosmic_text::fontdb::Stretch::Condensed,
                        4 => cosmic_text::fontdb::Stretch::SemiCondensed,
                        6 => cosmic_text::fontdb::Stretch::SemiExpanded,
                        7 => cosmic_text::fontdb::Stretch::Expanded,
                        8 => cosmic_text::fontdb::Stretch::ExtraExpanded,
                        9 => cosmic_text::fontdb::Stretch::UltraExpanded,
                        _ => cosmic_text::fontdb::Stretch::Normal,
                    },
                    monospaced: face.monospaced,
                });
            }
            let families = [
                fontdb::Family::SansSerif,
                fontdb::Family::Serif,
                fontdb::Family::Monospace,
            ]
            .map(|family| {
                self.database
                    .query(&fontdb::Query {
                        families: &[family],
                        ..fontdb::Query::default()
                    })
                    .and_then(|id| self.database.face(id))
                    .and_then(|face| face.families.first())
                    .map(|family| family.0.clone())
            });
            if let Some(family) = &families[0] {
                database.set_sans_serif_family(family.clone());
            }
            if let Some(family) = &families[1] {
                database.set_serif_family(family.clone());
            }
            if let Some(family) = &families[2] {
                database.set_monospace_family(family.clone());
            }
            Mutex::new(cosmic_text::FontSystem::new_with_locale_and_db(
                "en-US".to_owned(),
                database,
            ))
        })
    }

    /// Logical retained source bytes, excluding parser and platform overhead.
    pub fn retained_bytes(&self) -> usize {
        let used = self
            .used_fonts
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let sources = used
            .values()
            .map(|(font, bytes)| (font.file_hash, *bytes))
            .collect::<std::collections::BTreeMap<_, _>>();
        self.bytes + sources.values().sum::<usize>()
    }

    pub(crate) fn record_font(
        &self,
        system: &cosmic_text::FontSystem,
        id: cosmic_text::fontdb::ID,
        bytes: &[u8],
    ) -> motion_core::FontIdentity {
        use std::hash::{Hash, Hasher};

        let mut used = self
            .used_fonts
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        used.entry(id)
            .or_insert_with(|| {
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                bytes.hash(&mut hash);
                let face = system.db().face(id);
                let retained = face
                    .filter(|face| !matches!(face.source, cosmic_text::fontdb::Source::Binary(_)))
                    .map_or(0, |_| bytes.len());
                (
                    motion_core::FontIdentity {
                        name: face.map(|face| face.post_script_name.clone()),
                        file_hash: hash.finish(),
                    },
                    retained,
                )
            })
            .0
            .clone()
    }

    /// Actual selected faces and non-cryptographic source hashes, in stable order.
    pub fn used_fonts(&self) -> Vec<motion_core::FontIdentity> {
        let used = self
            .used_fonts
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut fonts = used
            .values()
            .map(|(font, _)| font.clone())
            .collect::<Vec<_>>();
        fonts.sort_by(|left, right| {
            (&left.name, left.file_hash).cmp(&(&right.name, right.file_hash))
        });
        fonts
    }
}

impl Default for FontLibrary {
    fn default() -> Self {
        Self::new()
    }
}

impl std::panic::RefUnwindSafe for FontLibrary {}

impl From<FontLibrary> for motion_core::asset::environment::FontEnvironment {
    fn from(fonts: FontLibrary) -> Self {
        let identity = fonts.snapshot_id();
        let usage = fonts.clone();
        Self::new(fonts, identity).with_usage(move || usage.used_fonts())
    }
}

impl From<&FontLibrary> for motion_core::asset::environment::FontEnvironment {
    fn from(fonts: &FontLibrary) -> Self {
        fonts.clone().into()
    }
}

impl std::panic::UnwindSafe for FontLibrary {}

impl std::fmt::Debug for FontLibrary {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FontLibrary")
            .field("faces", &self.len())
            .field("bytes", &self.bytes)
            .field("snapshot", &self.snapshot_id)
            .finish()
    }
}

/// Immutable font bytes and collection face index, shared across scene snapshots.
///
/// Loading never performs font discovery or I/O. Layout reparses the retained
/// bytes; parser allocations and caller input are not a hard RSS budget.
#[derive(Clone, Debug)]
pub struct FontFace {
    bytes: Arc<[u8]>,
    index: u32,
}

impl FontFace {
    /// Validates an OpenType/TrueType face before retaining its bytes.
    /// Invalid data or face indices fail without constructing a resource.
    pub fn from_bytes(
        bytes: Vec<u8>,
        face_index: u32,
    ) -> Result<Self, motion_core::ValidationError> {
        if bytes.len() > MAX_FONT_FACE_BYTES {
            return Err(motion_core::ValidationError::limit(
                "font",
                "font_bytes",
                "font exceeds 32 MiB",
            ));
        }
        skrifa::FontRef::from_index(&bytes, face_index).map_err(|_| {
            crate::text::invalid("font", "Invalid font data or collection face index")
        })?;
        Ok(Self {
            bytes: bytes.into(),
            index: face_index,
        })
    }

    /// Retained source payload, excluding allocation overhead and parser scratch.
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }

    /// Index of the retained face in its source collection.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// Creates an explicit paragraph font snapshot without system discovery.
    pub fn library(&self) -> Result<FontLibrary, motion_core::ValidationError> {
        let mut fonts = FontLibrary::new();
        fonts
            .load_data(self.bytes.clone())
            .map_err(|error| crate::text::invalid("font", &error.to_string()))?;
        let family = fonts
            .database()
            .faces()
            .find(|face| face.index == self.index)
            .and_then(|face| face.families.first().map(|family| family.0.clone()))
            .ok_or_else(|| crate::text::invalid("font", "Explicit font face has no family"))?;
        Ok(fonts.default_family(family))
    }
}

fn identity() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn invalid(message: &str) -> motion_core::Error {
    motion_core::Error::asset(
        "font",
        "<library>",
        std::io::Error::new(std::io::ErrorKind::InvalidData, message),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_file_sources_remain_lazy_until_layout() -> motion_core::Result<()> {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/fonts/NotoSans-Regular.ttf");
        let mut discovered = fontdb::Database::new();
        discovered.load_font_file(fixture)?;
        let mut fonts = FontLibrary::new();
        let empty = fonts.clone();
        assert!(fonts.index_faces(discovered.faces().cloned())? > 0);
        assert!(empty.is_empty());
        assert_eq!(fonts.retained_bytes(), 0);
        assert!(
            fonts
                .lock_font_system()?
                .db()
                .faces()
                .all(|face| matches!(face.source, cosmic_text::fontdb::Source::File(_)))
        );
        let paragraph = crate::Text::new("Lazy font")
            .family("Noto Sans")
            .layout(&fonts)?;
        assert!(paragraph.bounds().width() > 0.0);
        assert!(!paragraph.glyphs().is_empty());
        assert!(fonts.retained_bytes() > 0);
        assert_eq!(fonts.used_fonts().len(), 1);
        assert_eq!(
            motion_core::FontEnvironment::from(&fonts).used_fonts(),
            fonts.used_fonts()
        );
        Ok(())
    }

    #[test]
    fn empty_font_libraries_are_explicit_and_invalid_registration_is_atomic() {
        let mut fonts = FontLibrary::new();
        assert!(fonts.is_empty());
        assert_eq!(fonts.retained_bytes(), 0);
        let snapshot = fonts.clone();
        assert!(Arc::ptr_eq(&fonts.database, &snapshot.database));
        assert!(fonts.load_data(Arc::from(&b"invalid font"[..])).is_err());
        assert!(Arc::ptr_eq(&fonts.database, &snapshot.database));
        assert_eq!(fonts.snapshot_id(), snapshot.snapshot_id());
        assert_eq!(fonts.len(), 0);
        assert!(fonts.load_system_family("").is_err());
        assert_eq!(fonts.snapshot_id(), snapshot.snapshot_id());
    }

    #[test]
    #[ignore = "explicitly discovers installed fonts; requires MOTION_TEST_SYSTEM_FAMILY"]
    fn explicitly_requested_installed_family_is_frozen_and_usable() {
        let mut fonts = FontLibrary::new();
        let empty = fonts.clone();
        let family = std::env::var("MOTION_TEST_SYSTEM_FAMILY").expect("explicit installed family");
        let added = fonts.load_system_family(&family).unwrap();
        assert!(added > 0);
        assert_eq!(fonts.retained_bytes(), 0);
        assert!(empty.is_empty());
        let paragraph = crate::Text::new("Installed family")
            .family(family)
            .layout(&fonts)
            .unwrap();
        assert!(paragraph.bounds().width() > 0.0);
        println!(
            "explicit installed family: faces={added} immutable_bytes={} shape=pass",
            fonts.retained_bytes()
        );
    }
}
