use super::*;
use crate::TimeLiteral;

fn parameters(kind: AtmosphereKind) -> AtmosphereParameters {
    AtmosphereParameters {
        kind,
        seed: 42,
        color: Color::from_rgb_float(0.7, 0.3, 0.2).with_alpha(0.4),
        opacity: 0.65,
        intensity: 3.0,
        scale: 4.0,
        drift: [0.0; 2],
        fractal: Fractal::default(),
    }
}

#[test]
fn analytic_envelopes_match_exact_integer_lattice_cpu_reference() {
    let canvas = PixelSize::new(64, 48);
    let local = Point::new(0.25, 0.25);
    for (kind, density) in [
        (
            AtmosphereKind::Fog {
                gradient: [0.25, 1.25],
            },
            1.0 - libm::exp(-0.65 * 0.5 * 0.85),
        ),
        (
            AtmosphereKind::LightLeak {
                center: [0.25, 0.25],
                extent: [0.3, 0.7],
            },
            0.65 * 0.8,
        ),
    ] {
        let parameters = parameters(kind);
        // All octaves sample integer lattice coordinates [4,3], hence cloud is exactly 0.5.
        let straight = parameters.sample([16.0, 12.0], canvas, [0.0; 2]).unwrap();
        let tint = parameters.color.to_linear_premultiplied();
        assert_eq!(straight[3], density * tint[3]);
        let paint = AtmospherePaint::new(parameters, canvas, 1.seconds().after_origin()).unwrap();
        let expected = [
            tint[0] / tint[3] * 3.0 * straight[3],
            tint[1] / tint[3] * 3.0 * straight[3],
            tint[2] / tint[3] * 3.0 * straight[3],
            straight[3],
        ]
        .map(|value| value as f32);
        assert_eq!(paint.sample(local).unwrap(), expected);
        assert_eq!(
            paint.work_per_pixel(),
            u64::from(parameters.fractal.octaves) * 192 + 256
        );
    }
}

#[test]
fn analytic_paint_validates_full_lattice_and_tiny_shader_envelopes_before_sampling() {
    let canvas = PixelSize::new(64, 48);
    let time = 2.seconds().after_origin();
    for kind in [
        AtmosphereKind::Dust {
            radius: 1e-100,
            coverage: 0.5,
        },
        AtmosphereKind::LightLeak {
            center: [0.0, 0.0],
            extent: [1e-100, 1.0],
        },
    ] {
        assert!(AtmospherePaint::new(parameters(kind), canvas, time).is_err());
    }
    let mut recipe = parameters(AtmosphereKind::Fog {
        gradient: [0.0, 1.0],
    });
    recipe.drift = [f64::MAX, 0.0];
    assert!(AtmospherePaint::new(recipe, canvas, time).is_err());
    recipe.drift = [0.0; 2];
    recipe.color = Color::from_rgb_float(1e30, 0.0, 0.0);
    assert!(AtmospherePaint::new(recipe, canvas, time).is_err());
    assert!(
        AtmospherePaint::new(
            parameters(AtmosphereKind::Fog {
                gradient: [0.0, 1.0]
            }),
            PixelSize::new(0, 1),
            time
        )
        .is_err()
    );
}
