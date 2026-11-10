//! Stateless world-to-viewport mapping built from ordinary scene groups.

/// f64 inverse-view parameters; scene time remains caller-owned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// World point placed at the current design viewport's center.
    pub center: crate::Point,
    /// Positive logical units per world unit.
    pub zoom: f64,
    /// Clockwise camera orientation in radians; content rotates oppositely.
    pub rotation: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: crate::Point::ORIGIN,
            zoom: 1.0,
            rotation: 0.0,
        }
    }
}

impl Camera {
    fn transform(
        self,
        context: &crate::SceneContext<'_>,
    ) -> Result<crate::Transform, crate::ValidationError> {
        if ![self.center.x, self.center.y, self.zoom, self.rotation]
            .iter()
            .all(|value| value.is_finite())
            || self.zoom <= 0.0
        {
            return Err(invalid(
                "Camera requires finite coordinates, rotation and positive zoom",
            ));
        }
        let center = context.center();
        let transform = crate::Transform::translate(-self.center.x, -self.center.y)
            .then(crate::Transform::scale(self.zoom))
            .then(crate::Transform::rotate(-self.rotation))
            .then(crate::Transform::translate(center.x, center.y));
        if transform
            .coefficients()
            .iter()
            .any(|value| !value.is_finite())
            || transform.inverse().is_none()
        {
            return Err(invalid("Camera mapping must be finite and invertible"));
        }
        Ok(transform)
    }

    /// Projects a world point without clipping or including ancestor transforms.
    pub fn world_to_screen(
        self,
        context: &crate::SceneContext<'_>,
        point: crate::Point,
    ) -> Result<crate::Point, crate::ValidationError> {
        map(self.transform(context)?, point)
    }

    /// Unprojects a design-viewport point, including points outside the viewport.
    pub fn screen_to_world(
        self,
        context: &crate::SceneContext<'_>,
        point: crate::Point,
    ) -> Result<crate::Point, crate::ValidationError> {
        map(
            self.transform(context)?
                .inverse()
                .ok_or_else(|| invalid("Camera inverse is not representable"))?,
            point,
        )
    }
}

/// Applies a view, retaining invalid camera diagnostics until compilation.
pub fn camera(
    context: &crate::SceneContext<'_>,
    content: impl Into<crate::Node>,
    view: Camera,
) -> crate::Node {
    try_camera(context, content, view).unwrap_or_else(super::scene::failure)
}

/// Applies the inverse view to a subtree, not to subsequent HUD nodes.
/// No font discovery, old layers, new node kind or implicit viewport clip is added.
///
/// ```
/// use motion_core::prelude::*;
/// use motion_core::{
///     DisplayList, RasterLimits, ResourcePool, ValidatedDisplayList, MAX_PATH_VERBS,
///     MAX_STROKE_VERBS, MAX_SVG_PATH_BYTES, sample_offsets,
/// };
/// let cache = SceneCache::new(0);
/// let context = SceneContext::new(Time::ZERO, FrameRate::from(30), Size::new(8.0, 8.0), &cache);
/// let view = Camera {
///     center: Point::new(10.0, 5.0),
///     zoom: 2.0,
///     rotation: 0.0,
/// };
/// let node = camera(
///     &context,
///     Rect::new(9.0, 4.0, 11.0, 6.0).fill(Color::WHITE),
///     view,
/// );
/// assert_eq!(
///     view.world_to_screen(&context, view.center)?,
///     context.center()
/// );
/// node.compile(
///     context.size(),
///     PixelSize::new(8, 8),
///     ValidationLimits::default(),
/// )?;
/// # Ok::<(), motion_core::ValidationError>(())
/// ```
pub fn try_camera(
    context: &crate::SceneContext<'_>,
    content: impl Into<crate::Node>,
    view: Camera,
) -> Result<crate::Node, crate::ValidationError> {
    use crate::NodeModifiers;
    view.transform(context)?;
    Ok(
        crate::Node::group([content.into().translate(-view.center.x, -view.center.y)])
            .pivot(crate::Pivot::Point(crate::Point::ORIGIN))
            .scale(view.zoom)
            .rotate(-view.rotation)
            .translate(context.center().x, context.center().y),
    )
}

fn invalid(message: &str) -> crate::ValidationError {
    crate::ValidationError::invalid("camera", "view", message)
}

/// A 2.5D source plane; larger depth is farther from the camera.
#[derive(Clone, Debug)]
pub struct DepthPlane {
    /// Immutable plane artwork.
    pub node: crate::Node,
    /// Positive camera distance in the same units as focal length.
    pub depth: f64,
}

/// Thin-lens circle of confusion and optional depth-dependent perspective/parallax.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthOfField {
    /// Camera-to-focus distance, greater than focal length.
    pub focus_distance: f64,
    /// Positive aperture f-number.
    pub f_stop: f64,
    /// Positive focal length in scene units.
    pub focal_length: f64,
    /// Finite nonnegative maximum blur radius in design units.
    pub max_radius: f64,
    /// Apply pinhole perspective and depth-scaled camera translation.
    pub perspective: bool,
}

impl DepthOfField {
    /// Validates physical lens parameters.
    pub fn validate(self) -> Result<(), crate::ValidationError> {
        if ![
            self.focus_distance,
            self.f_stop,
            self.focal_length,
            self.max_radius,
        ]
        .iter()
        .all(|value| value.is_finite())
            || self.f_stop <= 0.0
            || self.focal_length <= 0.0
            || self.focus_distance <= self.focal_length
            || self.max_radius < 0.0
        {
            return Err(invalid(
                "Require a positive aperture/focal length, focus beyond the lens and a finite blur cap",
            ));
        }
        Ok(())
    }

    /// Radius is exactly zero at focus; extreme finite depths remain capped.
    pub fn radius(self, depth: f64) -> Result<f64, crate::ValidationError> {
        self.validate()?;
        if !depth.is_finite() || depth <= self.focal_length {
            return Err(invalid(
                "Plane depth must be finite and beyond the focal length",
            ));
        }
        let radius = (self.focal_length / self.f_stop)
            * (self.focal_length / (self.focus_distance - self.focal_length))
            * ((depth - self.focus_distance).abs() / depth)
            * 0.5;
        Ok(radius.min(self.max_radius))
    }

    /// Orders far planes before near planes, then applies the camera to each plane.
    pub fn capture(
        self,
        context: &crate::SceneContext<'_>,
        view: Camera,
        planes: impl IntoIterator<Item = DepthPlane>,
    ) -> Result<crate::Node, crate::ValidationError> {
        use crate::NodeModifiers;
        self.validate()?;
        view.transform(context)?;
        let mut planes = planes.into_iter().take(257).collect::<Vec<_>>();
        if planes.len() > 256 {
            return Err(invalid("Depth capture supports at most 256 planes"));
        }
        for plane in &planes {
            self.radius(plane.depth)?;
        }
        planes.sort_by(|first, last| last.depth.total_cmp(&first.depth));
        let mut nodes = Vec::with_capacity(planes.len());
        for plane in planes {
            let radius = self.radius(plane.depth)?;
            let factor = if self.perspective {
                (self.focus_distance - self.focal_length) / (plane.depth - self.focal_length)
            } else {
                1.0
            };
            let transform = view.transform(context)?;
            let node = plane
                .node
                .transformed(crate::Transform::scale(factor).then(transform));
            nodes.push(if radius == 0.0 {
                node
            } else {
                node.filter(crate::Filter::Optics(crate::Optics::Bokeh {
                    radius,
                    shape: crate::ApertureShape::Disc,
                    highlight_boost: 0.0,
                }))
            });
        }
        Ok(crate::Node::group(nodes))
    }
}

fn map(
    transform: crate::Transform,
    point: crate::Point,
) -> Result<crate::Point, crate::ValidationError> {
    let result = transform.apply(point);
    if [point.x, point.y, result.x, result.y]
        .iter()
        .all(|value| value.is_finite())
    {
        Ok(result)
    } else {
        Err(invalid("Camera points must map to finite coordinates"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Shape;

    #[test]
    fn thin_lens_focus_is_exact_and_out_of_focus_radius_is_capped() {
        let lens = super::DepthOfField {
            focus_distance: 1000.0,
            f_stop: 2.0,
            focal_length: 50.0,
            max_radius: 10.0,
            perspective: true,
        };
        assert_eq!(lens.radius(1000.0).unwrap(), 0.0);
        assert!(lens.radius(100.0).unwrap() > 0.0);
        assert!(lens.radius(50.0).is_err());
        assert!(
            super::DepthOfField {
                f_stop: 0.0,
                ..lens
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn camera_invalid_parameters_fail_without_rendering_and_do_not_mutate_views() {
        let cache = crate::SceneCache::new(0);
        let context = crate::SceneContext::new(
            crate::Time::ZERO,
            crate::FrameRate::from(30),
            crate::Size::new(8.0, 8.0),
            &cache,
        );
        for view in [
            Camera {
                zoom: 0.0,
                ..Camera::default()
            },
            Camera {
                zoom: -1.0,
                ..Camera::default()
            },
            Camera {
                rotation: f64::NAN,
                ..Camera::default()
            },
            Camera {
                center: crate::Point::new(f64::INFINITY, 0.0),
                ..Camera::default()
            },
        ] {
            assert!(try_camera(&context, crate::Node::empty(), view).is_err());
            assert!(
                view.screen_to_world(&context, crate::Point::ORIGIN)
                    .is_err()
            );
        }
        let view = Camera::default();
        assert!(
            view.world_to_screen(&context, crate::Point::new(f64::NAN, 0.0))
                .is_err()
        );
        assert_eq!(view, Camera::default());
    }

    #[test]
    fn projection_keeps_fixed_clip_parent_mapping_and_overflow_guards() -> crate::Result {
        use crate::{DrawCommand, Node, NodeModifiers, Pivot, Point, Rect, Size, Transform};
        let cache = crate::SceneCache::new(0);
        let context = crate::SceneContext::new(
            crate::Time::ZERO,
            crate::FrameRate::NTSC,
            Size::new(120.0, 80.0),
            &cache,
        );
        let view = Camera {
            center: Point::new(30.0, -20.0),
            zoom: 2.5,
            rotation: 0.7,
        };
        let viewport = Rect::new(20.0, 40.0, 140.0, 120.0);
        for parent in [
            Transform::IDENTITY,
            Transform::scale_non_uniform(-1.3, 1.1)
                .then(Transform::rotate(0.3))
                .then(Transform::translate(80.0, 100.0)),
        ] {
            let artwork = try_camera(
                &context,
                Rect::new(0.0, 0.0, 12.0, 8.0).fill(crate::Color::WHITE),
                view,
            )?;
            let world = Node::group([artwork.translate(20.0, 40.0)])
                .clip_with(viewport, crate::ClipMode::Geometric);
            let components = parent.decompose().unwrap();
            let node = world
                .pivot(Pivot::Point(Point::ORIGIN))
                .scale_xy(components.scale.x, components.scale.y)
                .rotate(components.rotation)
                .translate(components.translation.x, components.translation.y);
            let list = node.compile(
                Size::new(320.0, 180.0),
                crate::PixelSize::new(320, 180),
                crate::ValidationLimits::default(),
            )?;
            let transform = list
                .commands
                .iter()
                .find_map(|command| {
                    if let DrawCommand::FillRect { transform, .. } = command {
                        Some(*transform)
                    } else {
                        None
                    }
                })
                .unwrap();
            let clip = list
                .commands
                .iter()
                .find_map(|command| {
                    if let DrawCommand::PushRectLayer { rect_clip, .. } = command {
                        Some(*rect_clip)
                    } else {
                        None
                    }
                })
                .unwrap();
            assert_eq!(clip.bounds, viewport);
            for point in [
                Point::new(30.0, -20.0),
                Point::ORIGIN,
                Point::new(-10.0, 25.0),
            ] {
                let expected = parent.apply(
                    view.world_to_screen(&context, point)? + crate::Vector2::new(20.0, 40.0),
                );
                let actual = transform.apply(point);
                assert!(
                    (actual.x - expected.x).abs() < 1e-10 && (actual.y - expected.y).abs() < 1e-10
                );
                let recovered =
                    view.screen_to_world(&context, view.world_to_screen(&context, point)?)?;
                assert!(
                    (recovered.x - point.x).abs() < 1e-12 && (recovered.y - point.y).abs() < 1e-12
                );
            }
            assert!(
                clip.transform
                    .coefficients()
                    .iter()
                    .zip(parent.coefficients())
                    .all(|(actual, expected)| (actual - expected).abs() < 1e-12)
            );
        }
        for zoom in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::from_bits(1)] {
            let invalid = Camera { zoom, ..view };
            assert!(invalid.world_to_screen(&context, Point::ORIGIN).is_err());
            assert!(invalid.screen_to_world(&context, Point::ORIGIN).is_err());
            assert!(try_camera(&context, Node::empty(), invalid).is_err());
        }
        let zoomed = Camera {
            zoom: 2.0,
            ..Camera::default()
        };
        assert!(
            zoomed
                .world_to_screen(&context, Point::new(f64::MAX, 0.0))
                .is_err()
        );
        let zoomed = Camera {
            zoom: 0.5,
            ..Camera::default()
        };
        assert!(
            zoomed
                .screen_to_world(&context, Point::new(f64::MAX, 0.0))
                .is_err()
        );
        Ok(())
    }
}
