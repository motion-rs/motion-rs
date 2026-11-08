//! Asset-free audio/brush study sharing a reveal clock with a path emitter.

use crate::{DemoMetadata, DemoParameters};
use motion::audio::{AudioAnalysis, synth::Synth};
use motion::ink::{BrushStroke, Ink, NibMode, StrokeFont};
use motion::particles::{ParticleSimulation, PathEmitter};
use motion::prelude::*;
use std::sync::Arc;

pub(super) const METADATA: DemoMetadata = DemoMetadata::new(
    "audio-brush",
    "Audio Brush",
    "Motion",
    &[
        "audio-bloom",
        "brush-tip-particles",
        "synthesis",
        "extended-ink",
        "single-line-lettering",
    ],
);

pub(super) fn timeline(parameters: DemoParameters) -> motion::Result<Timeline> {
    let duration = parameters.seconds.seconds();
    let drums = Synth::drum_machine(
        120.0,
        [5, 0, 4, 0, 2, 0, 4, 0, 1, 0, 4, 0, 6, 0, 4, 0],
        duration,
        parameters.seed as u32,
    )?;
    let pulse = AudioAnalysis::from_source(&drums, duration)?
        .loudness()?
        .normalize(0.95)?
        .attack_release(20.milliseconds(), 150.milliseconds())?;
    let mut path = Path2D::new();
    path.try_move_to(Point::new(64.0, 220.0))?;
    path.try_cubic_to(
        Point::new(140.0, 40.0),
        Point::new(330.0, 300.0),
        Point::new(420.0, 110.0),
    )?;
    let brush = BrushStroke::new(path.clone(), 12.0, Color::rgb(24, 178, 146))
        .pressure_curve(vec![(0.0, 0.2), (0.5, 1.5), (1.0, 0.2)])
        .nib_mode(NibMode::FollowPath(0.0))
        .bristles(5, 0.4, 0.15, parameters.seed as u32)
        .sample_limit(64)
        .ink(Ink::new(parameters.seed as u32).dryness(0.2).bleed(1.0))
        .parallel_material(true);
    let progress = Tween::new(0.0, 1.0, duration);
    let particles = ParticleSimulation::new(parameters.seed)
        .duration(duration)
        .emitter(
            PathEmitter::new(Arc::new(path))
                .along_path(progress.clone())
                .rate(60.0)
                .lifetime(400.milliseconds())
                .speed(20.0)
                .size(2.0),
        );
    let lettering = Node::group(
        StrokeFont::new(16.0)?
            .strokes("AUDIO + INK", 0.8, Color::rgb(214, 220, 217))?
            .into_iter()
            .map(Node::from)
            .collect::<Vec<_>>(),
    )
    .translate(64.0, 36.0);
    Ok(Timeline::new(Size::new(960.0, 540.0), 30)
        .audio(AudioClip::source(drums).volume(0.3))
        .master(Master::safe())
        .track(|track| {
            track.scene(duration, move |context| {
                let revealed = brush.clone().write(context.sample(&progress));
                let cursor = revealed.clone().tip_node(
                    Circle::new(Point::new(0.0, 0.0), 3.0).fill(Color::rgb(243, 201, 70)),
                )?;
                Ok::<Node, motion::Error>(
                    Node::group([
                        context.bounds().fill(Color::rgb(16, 20, 24)).into(),
                        Node::group([
                            lettering.clone(),
                            Node::from(revealed),
                            cursor,
                            particles.try_render(context)?,
                        ])
                        .scale(1.8)
                        .translate(48.0, 0.0),
                    ])
                    .filter(Filter::Bloom {
                        threshold: 0.4,
                        knee: 0.2,
                        radius: 5.0,
                        intensity: context.sample(&pulse) * 0.8,
                        levels: 3,
                        falloff: 0.6,
                        tint: Color::WHITE,
                    }),
                )
            })
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion::audio::AudioSource;

    #[test]
    fn asset_free_audio_brush_replays_pixels_and_pcm() -> motion::Result {
        let timeline = timeline(DemoParameters {
            seconds: 2,
            ..Default::default()
        })?;
        let options = RenderOptions::default().size(PixelSize::new(160, 100));
        let last = render_still(&timeline, 1500.milliseconds(), &options)?;
        let early = render_still(&timeline, 500.milliseconds(), &options)?;
        let again = render_still(&timeline, 1500.milliseconds(), &options)?;
        assert_eq!(last.bytes(), again.bytes());
        assert_ne!(last.bytes(), early.bytes());
        assert!(
            last.bytes()
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[1] > 80)
        );
        let mixed = motion::audio::MixedAudio::from_timeline(&timeline, 48000)?;
        let mut first = [0.0; 1024];
        let mut repeat = [0.0; 1024];
        mixed.read(1000, &mut first);
        mixed.read(0, &mut repeat);
        mixed.read(1000, &mut repeat);
        assert_eq!(first, repeat);
        assert!(first.iter().any(|value| value.abs() > 0.0001));
        Ok(())
    }
}
