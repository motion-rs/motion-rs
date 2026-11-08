//! Stateless planar forward kinematics and timeline-compatible pose values.

use crate::{Interpolate, Node, NodeModifiers, Pivot, Point, Transform, ValidationError};

/// One joint's rest pivot, measured in its parent's rotated local coordinates.
///
/// A bone has no implicit length, artwork, scale, or skinning weights. Attach a
/// child at an offset to establish a limb, and draw between the solved pivots or
/// attach artwork using [`BoneTransform::apply`]. Root offsets use skeleton space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bone {
    parent: Option<usize>,
    offset: (f64, f64),
}

impl Bone {
    /// Stores a parent index and finite rest offset in local pixels.
    /// Validation is deferred to [`Skeleton::new`]. A parent must appear earlier
    /// in the array; `None` starts an independent root in the same coordinate space.
    pub fn new(parent: Option<usize>, offset: (f64, f64)) -> Self {
        Self { parent, offset }
    }
}

/// Local translation/rotation channels for one bone, with an identity default.
///
/// Translation is added to the rest offset before the parent rotation is applied.
/// Rotation is clockwise radians, around this bone's pivot. Rotating a bone moves
/// its descendants but not its own pivot. There is no automatic angle wrapping:
/// `0` to `2 * PI` animates one full turn, not a shortest-arc rotation.
///
/// Interpolation permits overshoot. Nonfinite or overflowing values are rejected
/// by [`Skeleton::solve`], not by construction or timeline sampling.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BonePose {
    /// Additional parent-local translation in pixels; default `(0, 0)`.
    pub translation: (f64, f64),
    /// Additional clockwise local rotation in radians; default zero.
    pub rotation: f64,
}

impl Interpolate for BonePose {
    fn interpolate(&self, target: &Self, progress: f64) -> Self {
        Self {
            translation: self.translation.interpolate(&target.translation, progress),
            rotation: self.rotation.interpolate(&target.rotation, progress),
        }
    }
}

/// A fixed-size set of local channels, indexed exactly like its skeleton.
///
/// The default is the rest pose. Implements [`Interpolate`] so a whole pose can
/// be sampled with [`crate::Keyframes`] or [`crate::Tween`], including repeat,
/// delay, and reverse. The compile-time count prevents mismatched channel lengths;
/// skeletons with equal counts still require the same semantic bone ordering.
#[derive(Clone, Debug, PartialEq)]
pub struct Pose<const BONES: usize> {
    /// Local channels in skeleton index order.
    pub bones: [BonePose; BONES],
}

impl<const BONES: usize> Pose<BONES> {
    /// Creates a pose from local translation/rotation channels; does not validate.
    pub fn new(bones: [BonePose; BONES]) -> Self {
        Self { bones }
    }
}

impl<const BONES: usize> Default for Pose<BONES> {
    fn default() -> Self {
        Self::new([BonePose::default(); BONES])
    }
}

impl<const BONES: usize> Interpolate for Pose<BONES> {
    fn interpolate(&self, target: &Self, progress: f64) -> Self {
        Self::new(std::array::from_fn(|index| {
            self.bones[index].interpolate(&target.bones[index], progress)
        }))
    }
}

/// An immutable, parent-first 2D skeleton evaluated without preceding frames.
///
/// This is forward kinematics only: no IK, constraints, dynamics, or mesh skinning.
/// Multiple roots and empty skeletons are allowed. Solved coordinates are local
/// to the skeleton; put the assembled scene inside a normal transformed group to
/// position, scale, or mirror the complete character. No renderer changes are needed.
///
/// # Example
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// # fn main() -> motion_core::Result {
/// let rig = Skeleton::new([Bone::new(None, (0.0, 0.0)), Bone::new(Some(0), (60.0, 0.0))])?;
/// let mut pose = Pose::default();
/// pose.bones[0].rotation = std::f64::consts::FRAC_PI_2;
/// let joints = rig.solve(&pose)?;
/// assert!((joints[1].position().1 - 60.0).abs() < 0.001);
/// let artwork = joints[0]
///     .apply(Path2D::parse_svg("M0 0L60 0")?.stroke(12.0, Color::from_hex(0x23272c)));
/// # let _ = artwork;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Skeleton<const BONES: usize> {
    bones: [Bone; BONES],
}

impl<const BONES: usize> Skeleton<BONES> {
    /// Validates and stores the rest hierarchy without allocating a graph.
    ///
    /// # Errors
    /// Returns [`std::io::ErrorKind::InvalidInput`] for a nonfinite rest offset or a
    /// parent index that is not smaller than its child index. The latter rejects
    /// self-parenting, cycles, forward references, and out-of-range parents.
    pub fn new(bones: [Bone; BONES]) -> Result<Self, ValidationError> {
        for (index, bone) in bones.iter().enumerate() {
            if bone.parent.is_some_and(|parent| parent >= index)
                || ![bone.offset.0, bone.offset.1]
                    .iter()
                    .all(|value| value.is_finite())
            {
                return Err(invalid(
                    "Skeleton requires finite offsets and parent-first bone indices",
                ));
            }
        }
        Ok(Self { bones })
    }

    /// Solves joint pivots and orientations in skeleton space in one linear pass.
    ///
    /// Takes only the rest skeleton and this pose; sampling out of order or
    /// concurrently has no hidden state. A bone's translation is applied before
    /// its rotation, so its children inherit both the pivot and orientation.
    ///
    /// # Errors
    /// Returns [`std::io::ErrorKind::InvalidInput`] for nonfinite channels, overflowing
    /// local translations, or nonrepresentable solved positions/angles. The
    /// complete result is discarded on error; no partially solved pose is returned.
    pub fn solve(&self, pose: &Pose<BONES>) -> Result<[BoneTransform; BONES], ValidationError> {
        let mut solved = [BoneTransform {
            position: (0.0, 0.0),
            rotation: 0.0,
        }; BONES];
        for (index, bone) in self.bones.iter().enumerate() {
            let channel = pose.bones[index];
            if ![
                channel.translation.0,
                channel.translation.1,
                channel.rotation,
            ]
            .iter()
            .all(|value| value.is_finite())
            {
                return Err(invalid("Bone pose channels must be finite"));
            }
            let local = (
                bone.offset.0 + channel.translation.0,
                bone.offset.1 + channel.translation.1,
            );
            let joint = if let Some(parent) = bone.parent {
                BoneTransform {
                    position: solved[parent].point(local),
                    rotation: solved[parent].rotation + channel.rotation,
                }
            } else {
                BoneTransform {
                    position: local,
                    rotation: channel.rotation,
                }
            };
            if ![
                local.0,
                local.1,
                joint.position.0,
                joint.position.1,
                joint.rotation,
            ]
            .iter()
            .all(|value| value.is_finite())
            {
                return Err(invalid("Bone pose produces overflowing joint transforms"));
            }
            solved[index] = joint;
        }
        Ok(solved)
    }
}

/// A solved rigid transform from one bone's local frame to skeleton space.
///
/// Produced by [`Skeleton::solve`]. It contains no scale or inverse transform;
/// use a surrounding group for whole-character scaling, reflection, and clipping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneTransform {
    position: (f64, f64),
    rotation: f64,
}

impl BoneTransform {
    /// Returns the joint pivot in skeleton-local pixels, not composition pixels.
    pub fn position(self) -> (f64, f64) {
        self.position
    }

    /// Returns the accumulated clockwise rotation in radians, without wrapping.
    pub fn rotation(self) -> f64 {
        self.rotation
    }

    /// Maps a point from this bone's local coordinates into skeleton space.
    /// Arbitrary caller-supplied points are not validated; extreme points can
    /// overflow even when the solved joint itself is finite.
    pub fn point(self, point: (f64, f64)) -> (f64, f64) {
        let point = Transform::rotate(self.rotation)
            .then(Transform::translate(self.position.0, self.position.1))
            .apply(Point::new(point.0, point.1));
        (point.x, point.y)
    }

    /// Wraps artwork in a group at this joint, preserving its own modifiers.
    /// Call in your desired painter order; hierarchy order need not be draw order.
    /// This is rigid attachment, not path deformation or isolated group blending.
    pub fn apply(self, artwork: impl Into<Node>) -> Node {
        Node::group([artwork.into()])
            .pivot(Pivot::Point(Point::ORIGIN))
            .rotate(self.rotation)
            .translate(self.position.0, self.position.1)
    }
}

fn invalid(message: &str) -> ValidationError {
    ValidationError::invalid("skeleton", "input", message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Animation;
    use crate::{
        Color, DrawCommand, Easing, Keyframes, PixelSize, Rect, Size, TimeLiteral, ValidationLimits,
    };
    use std::f64::consts::FRAC_PI_2;

    fn transform(node: &Node) -> Transform {
        let list = node
            .compile(
                Size::new(64.0, 64.0),
                PixelSize::new(64, 64),
                ValidationLimits::default(),
            )
            .unwrap();
        list.commands
            .iter()
            .find_map(|command| match command {
                DrawCommand::FillRect { transform, .. } => Some(*transform),
                _ => None,
            })
            .unwrap()
    }

    fn close(actual: (f64, f64), expected: (f64, f64)) {
        assert!(
            (actual.0 - expected.0).abs() < 0.0001 && (actual.1 - expected.1).abs() < 0.0001,
            "{actual:?} != {expected:?}"
        );
    }

    #[test]
    fn child_pivots_inherit_rotation_and_translation_but_not_sibling_pose()
    -> Result<(), ValidationError> {
        let rig = Skeleton::new([
            Bone::new(None, (5.0, 7.0)),
            Bone::new(Some(0), (10.0, 0.0)),
            Bone::new(Some(1), (4.0, 0.0)),
            Bone::new(Some(0), (0.0, 3.0)),
        ])?;
        let mut pose = Pose::default();
        pose.bones[0] = BonePose {
            translation: (2.0, 1.0),
            rotation: FRAC_PI_2,
        };
        pose.bones[1] = BonePose {
            translation: (2.0, 0.0),
            rotation: -FRAC_PI_2,
        };
        let solved = rig.solve(&pose)?;
        close(solved[0].position(), (7.0, 8.0));
        close(solved[1].position(), (7.0, 20.0));
        close(solved[2].position(), (11.0, 20.0));
        close(solved[3].position(), (4.0, 8.0));
        close(solved[1].point((3.0, 0.0)), (10.0, 20.0));
        assert!(solved[1].rotation().abs() < 0.0001);
        Ok(())
    }

    #[test]
    fn pose_tracks_are_repeatable_and_keep_unwrapped_turns() -> Result<(), ValidationError> {
        let rig = Skeleton::new([Bone::new(None, (0.0, 0.0)), Bone::new(Some(0), (10.0, 0.0))])?;
        let start = Pose::default();
        let mut end = start.clone();
        end.bones[0].rotation = std::f64::consts::TAU;
        let track = Keyframes::new(start.clone()).to(1.seconds(), end.clone(), Easing::Linear);
        let time = 250.milliseconds();
        let first = rig.solve(&track.sample(time))?;
        close(first[1].position(), (0.0, 10.0));
        let _later = rig.solve(&track.sample(900.milliseconds()))?;
        assert_eq!(first, rig.solve(&track.sample(time))?);
        assert_eq!(track.sample(crate::Time::ZERO), start);
        assert_eq!(track.sample(1.seconds()), end);
        assert_eq!(
            start.interpolate(&end, 1.5).bones[0].rotation,
            std::f64::consts::TAU * 1.5
        );
        Ok(())
    }

    #[test]
    fn hierarchy_channels_and_overflow_are_validated() -> Result<(), ValidationError> {
        assert!(Skeleton::new([Bone::new(Some(0), (0.0, 0.0))]).is_err());
        assert!(
            Skeleton::new([
                Bone::new(Some(1), (0.0, 0.0)),
                Bone::new(Some(0), (0.0, 0.0))
            ])
            .is_err()
        );
        assert!(Skeleton::new([Bone::new(None, (f64::NAN, 0.0))]).is_err());
        let rig = Skeleton::new([Bone::new(None, (0.0, 0.0))])?;
        for invalid_value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut pose = Pose::default();
            pose.bones[0].rotation = invalid_value;
            assert!(rig.solve(&pose).is_err());
            pose.bones[0] = BonePose {
                translation: (invalid_value, 0.0),
                rotation: 0.0,
            };
            assert!(rig.solve(&pose).is_err());
        }
        let large = Skeleton::new([
            Bone::new(None, (f64::MAX, 0.0)),
            Bone::new(Some(0), (f64::MAX, 0.0)),
        ])?;
        assert!(large.solve(&Pose::default()).is_err());
        let mut pose = Pose::default();
        pose.bones[0].rotation = f64::MAX;
        pose.bones[1].rotation = f64::MAX;
        let rotations =
            Skeleton::new([Bone::new(None, (0.0, 0.0)), Bone::new(Some(0), (0.0, 0.0))])?;
        assert!(rotations.solve(&pose).is_err());
        let mut translated = Pose::default();
        translated.bones[0].translation = (f64::MAX, 0.0);
        assert!(large.solve(&translated).is_err());
        Ok(())
    }

    #[test]
    fn attachments_preserve_artwork_local_transforms() -> Result<(), ValidationError> {
        let rig = Skeleton::new([Bone::new(None, (20.0, 30.0))])?;
        let pose = Pose::new([BonePose {
            translation: (2.0, 3.0),
            rotation: FRAC_PI_2,
        }]);
        let joint = rig.solve(&pose)?[0];
        let artwork = Node::from(Rect::new(0.0, 0.0, 10.0, 4.0).fill(Color::BLACK))
            .pivot(Pivot::Point(Point::new(3.0, 2.0)))
            .scale_xy(-1.2, 0.8)
            .rotate(0.3)
            .translate(7.0, 9.0);
        let point = (4.0, 2.0);
        let local = transform(&artwork).apply(Point::new(point.0, point.1));
        let expected = joint.point((local.x, local.y));
        let attached = joint.apply(artwork);
        let actual = transform(&attached).apply(Point::new(point.0, point.1));
        close((actual.x, actual.y), expected);
        Ok(())
    }

    #[test]
    fn independent_roots_empty_rigs_and_artwork_are_supported() -> Result<(), ValidationError> {
        let rig = Skeleton::new([Bone::new(None, (2.0, 3.0)), Bone::new(None, (8.0, 9.0))])?;
        let solved = rig.solve(&Pose::default())?;
        close(solved[0].position(), (2.0, 3.0));
        close(solved[1].position(), (8.0, 9.0));
        let artwork = Node::from(Rect::new(0.0, 0.0, 10.0, 4.0).fill(Color::BLACK))
            .rotate(0.5)
            .translate(2.0, 3.0);
        assert!(
            solved[1]
                .apply(artwork)
                .compile(
                    Size::new(64.0, 64.0),
                    PixelSize::new(64, 64),
                    ValidationLimits::default()
                )
                .is_ok()
        );
        let empty = Skeleton::new([])?;
        assert_eq!(empty.solve(&Pose::default())?, []);
        Ok(())
    }
}
