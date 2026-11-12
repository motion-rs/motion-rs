//! Validated opaque instance records shared by content and render packages.

use crate::drawing::BlendSpace;
use crate::{Point, Rect, ValidationError};
use std::{any::Any, fmt::Debug};

mod analytic;

pub use analytic::{
    ANALYTIC_HEADER_WORDS, ANALYTIC_STATE_WORDS, AnalyticInstanceState, AnalyticInstanceUpload,
    AnalyticTrajectoryKind,
};

/// Composites one already-covered linear-premultiplied instance sample.
pub type InstanceComposite = fn(&mut [f32; 4], [f32; 4], BlendSpace) -> Result<(), ValidationError>;

/// Optional ordered CPU sampling with conservative per-instance support.
/// Outside a declared support rectangle an instance must produce an all-zero source.
pub trait SpatialInstances: Debug + Send + Sync {
    /// Local support, including antialiasing and numeric guards, for one instance.
    fn support(&self, index: usize, antialias: f64) -> Rect;

    /// Samples only the ascending, in-range instance indices supplied by a consumer.
    /// The compositor must treat all-zero sources as infallible identities.
    fn sample_indices(
        &self,
        indices: &[usize],
        point: Point,
        antialias: f64,
        destination: [f32; 4],
        space: BlendSpace,
        composite: InstanceComposite,
    ) -> Result<[f32; 4], ValidationError>;
}

/// Maximum portable batch size; allocations and sampling remain budget-limited.
pub const MAX_INSTANCE_COUNT: usize = 1_048_576;

/// Fewest sides accepted for polygon and bladed bokeh instances.
const MIN_POLYGON_SIDES: f32 = 3.0;

/// Most sides accepted for polygon and bladed bokeh instances.
const MAX_POLYGON_SIDES: f32 = 32.0;

/// Largest stretch or aspect ratio accepted by elongated, quad and sprite-sheet instances.
const MAX_ASPECT: f32 = 64.0;

/// Coverage and compositing of one evaluated primitive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum InstanceKind {
    /// Antialiased filled circle using SourceOver.
    Disc,
    /// Gaussian circle using linear additive compositing.
    Glow,
    /// Rotated square sampling the batch's shared RGBA image.
    Sprite,
    /// Antialiased segment from start to position using SourceOver.
    Segment,
    /// Sprite with a normalized UV rectangle in the extension words.
    SpriteSheet,
    /// Segment with endpoint colors and radii.
    TaperedSegment,
    /// Regular polygon; extension contains side count and aspect.
    Polygon,
    /// Annular disc; extension contains relative thickness.
    Ring,
    /// Velocity-oriented capsule.
    Capsule,
    /// Disc with adjustable edge hardness.
    Soft,
    /// Rotated rectangular particle.
    Quad,
    /// Energy-compensated aperture with an optional bright rim.
    Bokeh,
    /// Flat-ended tapered strip between two historical positions.
    Ribbon,
}

/// Batch-level compositing applied to every primitive in a batch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum InstanceBlend {
    /// Premultiplied source-over compositing.
    #[default]
    SourceOver,
    /// Additive linear RGB with bounded alpha.
    Plus,
    /// Screen compositing.
    Screen,
    /// Multiply compositing.
    Multiply,
}

/// Backend-independent evaluated geometry, encoded as twelve device words.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InstanceRecord {
    /// Circle center or segment endpoint in local coordinates.
    pub position: [f32; 2],
    /// Segment start; ignored by other primitive kinds.
    pub start: [f32; 2],
    /// Linear-premultiplied RGBA multiplier; finite HDR RGB is supported.
    pub color: [f32; 4],
    /// Circle radius or sprite half extent; zero disables the primitive.
    pub radius: f32,
    /// Evaluated sprite angle in radians, including velocity alignment.
    pub rotation: f32,
    /// Full segment width in local coordinates.
    pub width: f32,
    /// Evaluated coverage and compositing rule.
    pub kind: InstanceKind,
}

impl InstanceRecord {
    fn words(self) -> Result<[f32; 12], ValidationError> {
        let words = [
            self.position[0],
            self.position[1],
            self.start[0],
            self.start[1],
            self.color[0],
            self.color[1],
            self.color[2],
            self.color[3],
            self.radius,
            self.rotation,
            self.width,
            self.kind as u32 as f32,
        ];
        if words.iter().any(|value| !value.is_finite())
            || self.radius < 0.0
            || self.width < 0.0
            || self.color[3] < 0.0
        {
            return Err(ValidationError::invalid(
                "instances.upload",
                "record",
                "Primitive geometry and color must be finite with nonnegative size and alpha",
            ));
        }
        Ok(words)
    }
}

/// Versioned sixteen-word primitive, leaving the original record layout intact.
/// SpriteSheet uses UV bounds; tapered segments use the other endpoint color;
/// analytic shapes use kind-specific finite parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExtendedInstanceRecord {
    /// Common twelve-word geometry. Tapered width is the start diameter and
    /// radius is the endpoint radius.
    pub base: InstanceRecord,
    /// UV bounds, endpoint RGBA, or analytic shape parameters.
    pub extra: [f32; 4],
}

impl ExtendedInstanceRecord {
    /// Validates extended geometry without allocating or encoding a batch.
    pub fn validate(self) -> Result<(), ValidationError> {
        self.words().map(|_| ())
    }

    fn words(self) -> Result<[f32; 16], ValidationError> {
        let mut words = [0.0; 16];
        words[..12].copy_from_slice(&self.base.words()?);
        let parameters = match self.base.kind {
            InstanceKind::Polygon => {
                (MIN_POLYGON_SIDES..=MAX_POLYGON_SIDES).contains(&self.extra[0])
                    && self.extra[0].fract() == 0.0
            }
            InstanceKind::Ring => self.extra[0] > 0.0 && self.extra[0] <= 1.0,
            InstanceKind::Soft => (0.0..=1.0).contains(&self.extra[0]),
            InstanceKind::Capsule | InstanceKind::Sprite => {
                (1.0..=MAX_ASPECT).contains(&self.extra[2])
            }
            InstanceKind::Quad => self.extra[2] > 0.0 && self.extra[2] <= MAX_ASPECT,
            InstanceKind::Bokeh => {
                (self.extra[0] == 0.0
                    || ((MIN_POLYGON_SIDES..=MAX_POLYGON_SIDES).contains(&self.extra[0])
                        && self.extra[0].fract() == 0.0))
                    && (0.0..=1.0).contains(&self.extra[1])
                    && (0.0..=1.0).contains(&self.extra[2])
            }
            InstanceKind::SpriteSheet => self.base.width > 0.0 && self.base.width <= MAX_ASPECT,
            _ => true,
        };
        if self.extra.iter().any(|value| !value.is_finite())
            || !parameters
            || self.base.color[3] > 1.0
            || (self.base.kind == InstanceKind::SpriteSheet
                && (self.extra.iter().any(|value| !(0.0..=1.0).contains(value))
                    || self.extra[0] >= self.extra[2]
                    || self.extra[1] >= self.extra[3]))
            || (matches!(
                self.base.kind,
                InstanceKind::TaperedSegment | InstanceKind::Ribbon
            ) && !(0.0..=1.0).contains(&self.extra[3]))
        {
            return Err(ValidationError::invalid(
                "instances.upload",
                "extension",
                "Invalid finite extension, UV bounds or endpoint alpha",
            ));
        }
        words[12..].copy_from_slice(&self.extra);
        Ok(words)
    }
}

/// Checked immutable device payload without graphics API handles.
#[derive(Debug)]
pub struct InstanceUpload {
    words: Vec<f32>,
}

impl InstanceUpload {
    /// Encodes checked twelve-word source-over primitives followed by a shared
    /// row-major linear RGBA image. A zero image size requires no texels; sprite
    /// records require a nonempty image.
    pub fn new(
        records: impl ExactSizeIterator<Item = InstanceRecord>,
        sprite_size: [u32; 2],
        texels: impl IntoIterator<Item = [f32; 4]>,
    ) -> Result<Self, ValidationError> {
        Self::new_blended(records, sprite_size, texels, InstanceBlend::SourceOver)
    }

    /// Like [`Self::new`], with an explicit batch compositing rule.
    pub fn new_blended(
        records: impl ExactSizeIterator<Item = InstanceRecord>,
        sprite_size: [u32; 2],
        texels: impl IntoIterator<Item = [f32; 4]>,
        blend: InstanceBlend,
    ) -> Result<Self, ValidationError> {
        Self::encode(
            records.map(|record| Ok((record.kind, record.words()?))),
            sprite_size,
            texels,
            blend,
        )
    }

    /// Encodes versioned sixteen-word primitives and a batch compositing rule.
    pub fn new_extended(
        records: impl ExactSizeIterator<Item = ExtendedInstanceRecord>,
        sprite_size: [u32; 2],
        texels: impl IntoIterator<Item = [f32; 4]>,
        blend: InstanceBlend,
    ) -> Result<Self, ValidationError> {
        Self::encode(
            records.map(|record| Ok((record.base.kind, record.words()?))),
            sprite_size,
            texels,
            blend,
        )
    }

    fn encode<const WORDS: usize>(
        records: impl ExactSizeIterator<Item = Result<(InstanceKind, [f32; WORDS]), ValidationError>>,
        sprite_size: [u32; 2],
        texels: impl IntoIterator<Item = [f32; 4]>,
        blend: InstanceBlend,
    ) -> Result<Self, ValidationError> {
        let invalid = || {
            crate::ValidationError::invalid(
                "instances.upload",
                "layout",
                "Primitive upload must match its bounded count, geometry and image dimensions",
            )
        };
        let instances = records.len();
        let [width, height] = sprite_size;
        if instances > MAX_INSTANCE_COUNT
            || width > 16_384
            || height > 16_384
            || (width == 0) != (height == 0)
        {
            return Err(invalid());
        }
        let pixels = (width as usize)
            .checked_mul(height as usize)
            .ok_or_else(invalid)?;
        let expected = instances
            .checked_mul(WORDS)
            .and_then(|records| {
                pixels
                    .checked_mul(4)
                    .and_then(|pixels| records.checked_add(pixels))
            })
            .and_then(|payload| payload.checked_add(4))
            .ok_or_else(invalid)?;
        let mut words = Vec::new();
        words.try_reserve_exact(expected).map_err(|_| invalid())?;
        // Header protocol: record stride in the low five bits, blend rule above them.
        let layout = WORDS as u32 + ((blend as u32) << 5);
        words.extend([instances as f32, width as f32, height as f32, layout as f32]);
        for (index, record) in records.enumerate() {
            let (kind, record) = record?;
            if index >= instances
                || (matches!(kind, InstanceKind::Sprite | InstanceKind::SpriteSheet) && pixels == 0)
                || (WORDS == 12 && kind as u32 > InstanceKind::Segment as u32)
            {
                return Err(invalid());
            }
            words.extend(record);
        }
        if words.len() != 4 + instances * WORDS {
            return Err(invalid());
        }
        for (index, texel) in texels.into_iter().enumerate() {
            if index >= pixels || texel.iter().any(|value| !value.is_finite()) || texel[3] < 0.0 {
                return Err(invalid());
            }
            words.extend(texel);
        }
        if words.len() != expected {
            return Err(invalid());
        }
        Ok(Self { words })
    }

    /// Number of encoded primitives.
    pub fn len(&self) -> usize {
        self.words[0] as usize
    }

    /// Whether no primitives are encoded.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Exact logical byte charge, including the header and shared image.
    pub fn bytes(&self) -> u64 {
        self.words.len() as u64 * 4
    }

    /// Transfers the validated words to the device planner without another allocation.
    pub fn into_words(self) -> Vec<f32> {
        self.words
    }
}

/// Immutable instance data, independent of a particular renderer or simulator.
pub trait InstanceBatch: Any + Debug + Send + Sync {
    /// Number of independently painted instances.
    fn len(&self) -> usize;

    /// Whether no instances are present.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Batch compositing policy; the default is source-over.
    fn blend(&self) -> InstanceBlend {
        InstanceBlend::SourceOver
    }

    /// Conservative local bounds.
    fn bounds(&self) -> Rect;

    /// Retained logical bytes charged during preparation.
    fn bytes(&self) -> u64;

    /// Conservative sampling work per output pixel.
    fn work(&self) -> u64;

    /// Optional conservative supports and ordered sampling; defaults to full-batch traversal.
    fn spatial(&self) -> Option<&dyn SpatialInstances> {
        None
    }

    /// Validates all stored instance/style values, including hidden instances.
    fn validate(&self) -> Result<(), ValidationError>;

    /// Samples instances in their declared order using the consumer's compositor.
    fn sample(
        &self,
        point: Point,
        antialias: f64,
        destination: [f32; 4],
        space: BlendSpace,
        composite: InstanceComposite,
    ) -> Result<[f32; 4], ValidationError>;

    /// Samples with a compositor for which every all-zero source, including
    /// signed zeros, is an infallible identity without observable side effects.
    /// Implementations may omit those calls, but must preserve nonzero sample
    /// order and errors. The default retains the ordinary sampling behavior.
    /// Callers with an arbitrary compositor must use [`Self::sample`] instead.
    fn sample_with_zero_identity(
        &self,
        point: Point,
        antialias: f64,
        destination: [f32; 4],
        space: BlendSpace,
        composite: InstanceComposite,
    ) -> Result<[f32; 4], ValidationError> {
        self.sample(point, antialias, destination, space, composite)
    }

    /// Exact upload charge for preflight admission; zero by default for CPU-only batches.
    fn upload_bytes(&self) -> u64 {
        0
    }

    /// Checked evaluated primitives, or None when only CPU sampling is available.
    fn upload(&self) -> Result<Option<InstanceUpload>, ValidationError> {
        Ok(None)
    }

    /// Optional checked seeded trajectory program replacing evaluated primitive positions.
    /// Existing generic batches and fixed-step simulations retain their ordinary uploads.
    fn analytic_upload(&self) -> Option<&AnalyticInstanceUpload> {
        None
    }

    /// Explicit reason an otherwise analytic batch retains evaluated CPU primitives.
    fn analytic_fallback(&self) -> Option<&ValidationError> {
        None
    }

    /// Resolves concrete content only within its owning implementation.
    fn as_any(&self) -> &dyn Any;

    /// Compares the logical payload of two instance records.
    fn content_eq(&self, other: &dyn InstanceBatch) -> bool;
}

impl PartialEq for dyn InstanceBatch {
    fn eq(&self, other: &Self) -> bool {
        self.content_eq(other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versioned_uploads_share_the_header_and_validate_analytic_extensions() {
        let base = InstanceRecord {
            position: [1.0, 2.0],
            start: [0.0; 2],
            color: [2.0, 0.1, 0.0, 0.5],
            radius: 2.0,
            rotation: 0.0,
            width: 1.0,
            kind: InstanceKind::Disc,
        };
        let old = InstanceUpload::new(std::iter::once(base), [0, 0], [])
            .unwrap()
            .into_words();
        let extended = ExtendedInstanceRecord {
            base,
            extra: [0.0, 0.0, 1.0, 0.0],
        };
        let new = InstanceUpload::new_extended(
            std::iter::once(extended),
            [0, 0],
            [],
            InstanceBlend::Screen,
        )
        .unwrap()
        .into_words();
        assert_eq!(old[3], 12.0);
        assert_eq!(new[3], 16.0 + ((InstanceBlend::Screen as u32) << 5) as f32);
        assert_eq!(&old[4..], &new[4..16]);
        assert_eq!(new.len(), 20);
        for (kind, parameters) in [
            (InstanceKind::Polygon, [2.0, 0.0, 1.0, 0.0]),
            (InstanceKind::Ring, [0.0; 4]),
            (InstanceKind::Quad, [0.0; 4]),
            (InstanceKind::Soft, [2.0, 0.0, 1.0, 0.0]),
            (InstanceKind::Bokeh, [4.5, 0.0, 1.0, 0.0]),
            (InstanceKind::TaperedSegment, [0.0, 0.0, 0.0, 1.1]),
            (InstanceKind::SpriteSheet, [0.5, 0.0, 0.4, 1.0]),
        ] {
            let record = ExtendedInstanceRecord {
                base: InstanceRecord { kind, ..base },
                extra: parameters,
            };
            assert!(record.validate().is_err(), "{kind:?}");
            assert!(InstanceUpload::new(std::iter::once(record.base), [0, 0], []).is_err());
        }
    }

    #[test]
    fn primitive_records_are_compact_and_validate_hidden_geometry() {
        let record = InstanceRecord {
            position: [3.0, 4.0],
            start: [-1.0, 2.0],
            color: [-0.5, 2.0, 0.0, 0.5],
            radius: 2.0,
            rotation: 0.75,
            width: 1.0,
            kind: InstanceKind::Segment,
        };
        let words = record.words().unwrap();
        assert_eq!(std::mem::size_of_val(&words), 48);
        assert_eq!(&words[..4], &[3.0, 4.0, -1.0, 2.0]);
        assert_eq!(words[11], 3.0);
        for invalid in [
            InstanceRecord {
                position: [f32::NAN, 0.0],
                ..record
            },
            InstanceRecord {
                start: [f32::INFINITY, 0.0],
                ..record
            },
            InstanceRecord {
                color: [0.0, 0.0, 0.0, -1.0],
                ..record
            },
            InstanceRecord {
                radius: -1.0,
                ..record
            },
            InstanceRecord {
                rotation: f32::NAN,
                ..record
            },
            InstanceRecord {
                width: -1.0,
                ..record
            },
        ] {
            assert!(invalid.words().is_err());
        }
    }

    #[test]
    fn primitive_upload_checks_sprite_texels_and_exact_byte_charge() {
        let record = InstanceRecord {
            position: [1.0, 2.0],
            start: [0.0; 2],
            color: [0.5; 4],
            radius: 2.0,
            rotation: 0.0,
            width: 1.0,
            kind: InstanceKind::Sprite,
        };
        let upload = InstanceUpload::new([record].into_iter(), [2, 1], [[0.5; 4]; 2]).unwrap();
        assert_eq!(upload.len(), 1);
        assert_eq!(upload.bytes(), (4 + 12 + 8) * 4);
        assert_eq!(&upload.into_words()[..4], &[1.0, 2.0, 1.0, 12.0]);
        assert!(InstanceUpload::new([record].into_iter(), [0, 0], []).is_err());
        assert!(InstanceUpload::new([record].into_iter(), [2, 0], []).is_err());
        assert!(InstanceUpload::new([record].into_iter(), [16_385, 1], []).is_err());
        assert!(InstanceUpload::new([record].into_iter(), [2, 1], [[0.0; 4]]).is_err());
        assert!(InstanceUpload::new([record].into_iter(), [1, 1], [[0.0; 4]; 2]).is_err());
        assert!(InstanceUpload::new([record].into_iter(), [1, 1], [[f32::NAN; 4]]).is_err());
        let record = InstanceRecord {
            kind: InstanceKind::Disc,
            ..record
        };
        let upload = InstanceUpload::new(std::iter::repeat_n(record, 65_537), [0, 0], []).unwrap();
        assert_eq!(upload.len(), 65_537);
        assert_eq!(upload.bytes(), 16 + 65_537 * 48);
        assert!(
            InstanceUpload::new(
                std::iter::repeat_n(record, MAX_INSTANCE_COUNT + 1),
                [0, 0],
                []
            )
            .is_err()
        );
        let empty = InstanceUpload::new(std::iter::empty(), [0, 0], []).unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.bytes(), 16);
    }

    #[test]
    fn zero_identity_default_preserves_external_batch_sampling() {
        #[derive(Debug)]
        struct ExternalBatch;

        impl InstanceBatch for ExternalBatch {
            fn len(&self) -> usize {
                1
            }

            fn bounds(&self) -> Rect {
                Rect::default()
            }

            fn bytes(&self) -> u64 {
                0
            }

            fn work(&self) -> u64 {
                1
            }

            fn validate(&self) -> Result<(), ValidationError> {
                Ok(())
            }

            fn sample(
                &self,
                _: Point,
                _: f64,
                mut destination: [f32; 4],
                space: BlendSpace,
                composite: InstanceComposite,
            ) -> Result<[f32; 4], ValidationError> {
                composite(&mut destination, [0.25, 0.125, 0.0, 0.5], space)?;
                Ok(destination)
            }

            fn as_any(&self) -> &dyn Any {
                self
            }

            fn content_eq(&self, other: &dyn InstanceBatch) -> bool {
                other.as_any().is::<Self>()
            }
        }

        fn capture(
            destination: &mut [f32; 4],
            source: [f32; 4],
            _: BlendSpace,
        ) -> Result<(), ValidationError> {
            if source != [0.0; 4] {
                *destination = source;
            }
            Ok(())
        }
        let batch: &dyn InstanceBatch = &ExternalBatch;
        assert_eq!(
            batch.sample_with_zero_identity(
                Point::ORIGIN,
                1.0,
                [0.0; 4],
                BlendSpace::LinearRgb,
                capture
            ),
            batch.sample(Point::ORIGIN, 1.0, [0.0; 4], BlendSpace::LinearRgb, capture)
        );
        assert_eq!(
            batch
                .sample_with_zero_identity(Point::ORIGIN, 1.0, [0.0; 4], BlendSpace::Srgb, capture)
                .unwrap(),
            [0.25, 0.125, 0.0, 0.5]
        );
    }
}
