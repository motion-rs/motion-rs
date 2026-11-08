//! Conversion of supported SVG filter primitives into vector filter groups.

use super::*;

pub(super) fn filter_group(
    source: &usvg::filter::Filter,
    content: VectorGroup,
) -> motion_core::Result<VectorGroup> {
    use motion_core::{Filter, FilterOperation, Vector2};
    use usvg::filter::{ColorInterpolation, ColorMatrixKind, Input, Kind};
    let mut result = content;
    let mut previous = None;
    for primitive in source.primitives() {
        if primitive.color_interpolation() != ColorInterpolation::LinearRGB {
            return Err(unsupported(
                "SVG sRGB filter interpolation cannot use the linear filter kernel",
            ));
        }
        let (input, filter) = match primitive.kind() {
            Kind::GaussianBlur(value) => {
                if value.std_dev_x() != value.std_dev_y() {
                    return Err(unsupported("Anisotropic SVG blur is unsupported"));
                }
                (
                    value.input(),
                    Filter::Blur {
                        sigma: f64::from(value.std_dev_x().get()),
                    },
                )
            }
            Kind::DropShadow(value) => {
                if value.std_dev_x() != value.std_dev_y() {
                    return Err(unsupported("Anisotropic SVG shadow is unsupported"));
                }
                let color = value.color();
                (
                    value.input(),
                    Filter::DropShadow {
                        sigma: f64::from(value.std_dev_x().get()),
                        offset: Vector2::new(f64::from(value.dx()), f64::from(value.dy())),
                        color: Color::rgb(color.red, color.green, color.blue)
                            .with_alpha(f64::from(value.opacity().get())),
                    },
                )
            }
            Kind::ColorMatrix(value) => {
                let matrix = match value.kind() {
                    ColorMatrixKind::Matrix(values) => values
                        .iter()
                        .copied()
                        .map(f64::from)
                        .collect::<Vec<_>>()
                        .try_into()
                        .map_err(|_| unsupported("Invalid SVG color matrix"))?,
                    ColorMatrixKind::Saturate(amount) => {
                        let amount = f64::from(amount.get());
                        let weights = [0.213, 0.715, 0.072];
                        let mut matrix = [0.0; 20];
                        for row in 0..3 {
                            for column in 0..3 {
                                matrix[row * 5 + column] = weights[column] * (1.0 - amount)
                                    + if row == column { amount } else { 0.0 };
                            }
                        }
                        matrix[18] = 1.0;
                        matrix
                    }
                    ColorMatrixKind::HueRotate(angle) => {
                        let cosine = libm::cos(f64::from(*angle).to_radians());
                        let sine = libm::sin(f64::from(*angle).to_radians());
                        let weights = [0.213, 0.715, 0.072];
                        let rotations = [
                            [-0.213, -0.715, 0.928],
                            [0.143, 0.140, -0.283],
                            [-0.787, 0.715, 0.072],
                        ];
                        let mut matrix = [0.0; 20];
                        for row in 0..3 {
                            for column in 0..3 {
                                matrix[row * 5 + column] = weights[column]
                                    + (if row == column { 1.0 } else { 0.0 } - weights[column])
                                        * cosine
                                    + rotations[row][column] * sine;
                            }
                        }
                        matrix[18] = 1.0;
                        matrix
                    }
                    ColorMatrixKind::LuminanceToAlpha => {
                        let mut matrix = [0.0; 20];
                        matrix[15..18].copy_from_slice(&[0.2125, 0.7154, 0.0721]);
                        matrix
                    }
                };
                (
                    value.input(),
                    Filter::Custom(vec![FilterOperation::ColorMatrix(matrix)]),
                )
            }
            other => {
                let name = match other {
                    Kind::Blend(_) => "feBlend",
                    Kind::ComponentTransfer(_) => "feComponentTransfer",
                    Kind::Composite(_) => "feComposite",
                    Kind::ConvolveMatrix(_) => "feConvolveMatrix",
                    Kind::DiffuseLighting(_) => "feDiffuseLighting",
                    Kind::DisplacementMap(_) => "feDisplacementMap",
                    Kind::Flood(_) => "feFlood",
                    Kind::Image(_) => "feImage",
                    Kind::Merge(_) => "feMerge",
                    Kind::Morphology(_) => "feMorphology",
                    Kind::Offset(_) => "feOffset",
                    Kind::SpecularLighting(_) => "feSpecularLighting",
                    Kind::Tile(_) => "feTile",
                    Kind::Turbulence(_) => "feTurbulence",
                    _ => "unknown primitive",
                };
                return Err(unsupported(format!(
                    "SVG filter primitive is unsupported: {name}"
                )));
            }
        };
        if matches!((input, &previous), (Input::SourceAlpha, None)) {
            let mut alpha = [0.0; 20];
            alpha[18] = 1.0;
            result = VectorGroup {
                filters: vec![Filter::Custom(vec![FilterOperation::ColorMatrix(alpha)])],
                children: vec![VectorItem::Group(result)],
                ..Default::default()
            };
        } else if !matches!((input, &previous), (Input::SourceGraphic, None))
            && !matches!((input,&previous),(Input::Reference(name),Some(previous)) if name==previous)
        {
            return Err(unsupported(
                "SVG filter input graph is not a sequential source chain",
            ));
        }
        result = VectorGroup {
            filters: vec![filter],
            clip: Some(Path2D::from_rect(rect(primitive.rect()))?.into()),
            children: vec![VectorItem::Group(result)],
            ..Default::default()
        };
        previous = Some(primitive.result().to_owned());
    }
    Ok(VectorGroup {
        clip: Some(Path2D::from_rect(rect(source.rect()))?.into()),
        children: vec![VectorItem::Group(result)],
        ..Default::default()
    })
}
