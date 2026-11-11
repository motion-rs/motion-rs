//! Instanced rendering of particle states and scene integration for simulations and trajectories.

#[cfg(test)]
use super::batch::SpatialNode;
use super::*;
use motion_core::{Node, Rect, SceneContext};

#[cfg(test)]
mod instance_protocol_tests;

fn render(
    style: &ParticleStyle,
    context: &SceneContext<'_>,
    states: Arc<[Particle]>,
    mut sample: impl FnMut(Time) -> Result<Arc<[Particle]>, ValidationError>,
) -> Result<Node, ValidationError> {
    match style {
        ParticleStyle::Configured { style, options } => {
            if matches!(style.as_ref(), ParticleStyle::Trails { .. })
                && options.blend.is_none()
                && options.sheet.is_none()
                && options.stretch.is_none()
                && options.parallax == [0.0; 2]
                && !options.sort_depth
                && !options.flip
                && options.trails.segments == 8
                && options.trails.taper.is_none()
                && options.trails.fade.is_none()
                && !options.trails.smooth
                && !options.trails.ribbon
            {
                return render(style, context, states, sample);
            }
            return super::extended::render(style, options, context, states, sample);
        }
        ParticleStyle::Shape { .. } | ParticleStyle::Bokeh { .. } => {
            return super::extended::render(
                style,
                &StyleOptions::default(),
                context,
                states,
                sample,
            );
        }
        _ => {}
    }
    if let ParticleStyle::Custom(callback) = style {
        return callback(&states, context);
    }
    let extent = match style {
        ParticleStyle::Dots { radius }
        | ParticleStyle::GlowDots { radius }
        | ParticleStyle::Plexus { radius, .. } => *radius,
        ParticleStyle::Sprites { size, .. } => *size,
        ParticleStyle::Trails { width, .. } => *width,
        _ => unreachable!(),
    };
    if !extent.is_finite() || !(0.0..=4096.0).contains(&extent) {
        return Err(invalid(
            "style",
            "Particle style size must be finite in 0..=4096",
        ));
    }
    if let ParticleStyle::Plexus { max_distance, .. } = style
        && (!max_distance.is_finite() || *max_distance <= 0.0)
    {
        return Err(invalid(
            "distance",
            "Plexus distance must be finite and positive",
        ));
    }
    if states.is_empty() && !matches!(style, ParticleStyle::Trails { .. }) {
        return Ok(Node::empty());
    }
    let (kind, radius, width, align, sprite) = match style {
        ParticleStyle::Dots { radius } => (0, *radius, 1.0, false, None),
        ParticleStyle::GlowDots { radius } => (1, *radius, 1.0, false, None),
        ParticleStyle::Sprites {
            image,
            size,
            align_to_velocity,
        } => match image.get() {
            Ok(image) => (2, *size * 0.5, 1.0, *align_to_velocity, Some(image)),
            Err(_) => {
                return Ok(Node::from(
                    crate::Image::new(image.clone()).size(*size, *size),
                ));
            }
        },
        ParticleStyle::Trails { width, .. } => (3, 1.0, *width, false, None),
        ParticleStyle::Plexus { radius, .. } => (4, *radius, 1.0, false, None),
        _ => unreachable!(),
    };
    if !radius.is_finite() || radius < 0.0 || !width.is_finite() || width < 0.0 {
        return Err(invalid("style", "Invalid particle style geometry"));
    }
    let mut instances = Vec::new();
    let mut append = |particle: Particle, previous: [f32; 2], line: bool| {
        if instances.len() >= MAX_INSTANCE_COUNT {
            return Err(limit("Particle instance allowance exhausted"));
        }
        instances
            .try_reserve(1)
            .map_err(|_| limit("Particle instance allocation failed"))?;
        instances.push(Instance::new(particle, previous, line));
        Ok(())
    };
    if let ParticleStyle::Trails { length, .. } = style {
        if *length <= Duration::ZERO || length.as_seconds() > 60.0 {
            return Err(invalid("trail", "Trail duration must be in (0,60] seconds"));
        }
        let mut previous = sample(context.time() - *length)?;
        for segment in 1..=8 {
            let time =
                context.time() - *length + Duration::from_ticks(length.ticks() * segment / 8);
            let current = sample(time)?;
            let lookup = previous
                .iter()
                .map(|particle| (particle.index, particle))
                .collect::<std::collections::BTreeMap<_, _>>();
            for particle in current.iter() {
                if let Some(before) = lookup.get(&particle.index) {
                    append(*particle, before.position, true)?;
                }
            }
            previous = current;
        }
    } else {
        for particle in states.iter() {
            append(*particle, particle.position, false)?;
        }
    }
    if let ParticleStyle::Plexus { max_distance, .. } = style {
        for (first, second) in super::neighbors::pairs(&states, *max_distance, 100_000_000)? {
            append(states[first], states[second].position, true)?;
        }
    }
    let mut bounds = Rect::default();
    for (index, instance) in instances.iter().enumerate() {
        let particle = instance.particle;
        let reach =
            (radius * f64::from(particle.size) * if kind == 1 { 3.0 } else { 2.0 }).max(width);
        let left = f64::from(particle.position[0].min(instance.previous[0])) - reach;
        let top = f64::from(particle.position[1].min(instance.previous[1])) - reach;
        let right = f64::from(particle.position[0].max(instance.previous[0])) + reach;
        let bottom = f64::from(particle.position[1].max(instance.previous[1])) + reach;
        let extent = Rect::new(left, top, right, bottom);
        bounds = if index == 0 {
            extent
        } else {
            bounds.union(extent)
        };
    }
    let mut batch = ParticleBatch {
        instances: instances.into(),
        kind,
        radius: radius as f32,
        width: width as f32,
        align,
        sprite,
        bounds,
        index: Arc::from([]),
    };
    batch.validate()?;
    batch.index = batch.build_spatial_index();
    Ok(Node::from(batch))
}

impl ParticleSimulationBuilder {
    /// Sets presentation; it does not affect the physics snapshots.
    pub fn style(mut self, style: ParticleStyle) -> Self {
        self.style = Arc::new(style);
        self
    }
}

impl ParticleSimulation {
    /// Renders through the instance protocol using explicit clip metadata.
    pub fn try_render(&self, context: &SceneContext<'_>) -> Result<Node, ValidationError> {
        let duration = context.clip_duration();
        // Trail history is queried in ascending time, so one cursor steps through it once
        // instead of replaying from a retained snapshot for every historical sample.
        let mut cursor = super::simulation::SampleCursor::default();
        let states = self.sample_with(&mut cursor, context.time(), duration)?;
        render(&self.style, context, states, |time| {
            self.sample_with(&mut cursor, time, duration)
        })
    }
}

impl ParticleTrajectory {
    /// Replaces presentation without adding simulation state.
    pub fn style(mut self, style: ParticleStyle) -> Self {
        self.style = Arc::new(style);
        self
    }

    /// Evaluates trajectories in Rust before uploading renderable instance states.
    pub fn try_render(&self, context: &SceneContext<'_>) -> Result<Node, ValidationError> {
        render(&self.style, context, self.sample(context.time())?, |time| {
            self.sample(time)
        })
    }
}

impl motion_core::authoring::Scene for ParticleSimulation {
    fn render(&self, context: &SceneContext<'_>) -> Node {
        self.try_render(context)
            .unwrap_or_else(motion_core::authoring::transition::diagnostic)
    }

    fn duration(&self) -> Option<Duration> {
        self.natural_duration()
    }
}

impl motion_core::authoring::Scene for ParticleTrajectory {
    fn render(&self, context: &SceneContext<'_>) -> Node {
        self.try_render(context)
            .unwrap_or_else(motion_core::authoring::transition::diagnostic)
    }

    fn duration(&self) -> Option<Duration> {
        self.model.duration()
    }
}

#[cfg(test)]
mod tests;
