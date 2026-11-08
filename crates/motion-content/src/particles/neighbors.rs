//! Bounded spatial hashing with fixed cell and stable pair traversal.
//!
//! [`NeighborIndex`] is the reusable building block for modifiers that need
//! radius queries: it is built once per step, never mutates particles, and every
//! traversal order is a pure function of the particle order, so results do not
//! depend on how many threads evaluate them.

use super::*;
use std::collections::BTreeMap;

/// Populations at or above this size evaluate pair chunks on worker threads.
const PARALLEL_THRESHOLD: usize = 2048;

const MAX_NEIGHBOR_THREADS: usize = 8;

/// Uniform grid with cell size equal to the query radius.
pub(crate) struct NeighborIndex<'a> {
    particles: &'a [Particle],
    radius: f64,
    cells: BTreeMap<(i64, i64), Vec<usize>>,
    coordinates: Vec<(i64, i64)>,
}

impl<'a> NeighborIndex<'a> {
    /// Validates the population and radius and hashes particles into cells.
    pub(crate) fn new(particles: &'a [Particle], radius: f64) -> Result<Self, ValidationError> {
        if !radius.is_finite() || radius <= 0.0 || particles.len() > 65536 {
            return Err(invalid(
                "neighbors",
                "Require a finite positive radius and at most 65536 particles",
            ));
        }
        let mut cells = BTreeMap::<(i64, i64), Vec<usize>>::new();
        let mut coordinates = Vec::new();
        coordinates
            .try_reserve_exact(particles.len())
            .map_err(|_| limit("Neighbor allocation failed"))?;
        for (index, particle) in particles.iter().enumerate() {
            particle.validate()?;
            let cell = particle
                .position
                .map(|value| libm::floor(f64::from(value) / radius));
            if cell
                .iter()
                .any(|value| !value.is_finite() || value.abs() > i64::MAX as f64 * 0.5)
            {
                return Err(invalid(
                    "neighbors",
                    "Spatial cell coordinates exceed integer range",
                ));
            }
            let cell = (cell[0] as i64, cell[1] as i64);
            coordinates.push(cell);
            cells.entry(cell).or_default().push(index);
        }
        Ok(Self {
            particles,
            radius,
            cells,
            coordinates,
        })
    }

    /// Number of indexed particles.
    pub(crate) fn len(&self) -> usize {
        self.particles.len()
    }

    /// Indices of other particles in the 3x3 cell block around `index`, in cell
    /// order; callers apply their own exact-distance test.
    pub(crate) fn candidates(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        let (horizontal, vertical) = self.coordinates[index];
        (vertical - 1..=vertical + 1)
            .flat_map(move |row| (horizontal - 1..=horizontal + 1).map(move |column| (column, row)))
            .filter_map(|cell| self.cells.get(&cell))
            .flatten()
            .copied()
            .filter(move |other| *other != index)
    }

    /// Sorted indices within the radius; `later_only` keeps just `other > index`
    /// so each unordered pair is reported once. Every inspected candidate adds
    /// one unit to `work`, which fails at `max_work`.
    pub(crate) fn within(
        &self,
        index: usize,
        later_only: bool,
        work: &mut u64,
        max_work: u64,
    ) -> Result<Vec<usize>, ValidationError> {
        let particle = &self.particles[index];
        let mut neighbors = Vec::new();
        for other in self.candidates(index) {
            if later_only && other < index {
                continue;
            }
            *work = work
                .checked_add(1)
                .ok_or_else(|| limit("Neighbor work overflow"))?;
            if *work > max_work {
                return Err(limit("Neighbor work allowance exhausted"));
            }
            if libm::hypot(
                f64::from(particle.position[0] - self.particles[other].position[0]),
                f64::from(particle.position[1] - self.particles[other].position[1]),
            ) <= self.radius
            {
                neighbors.push(other);
            }
        }
        neighbors.sort_unstable();
        Ok(neighbors)
    }

    fn chunk_pairs(
        &self,
        range: std::ops::Range<usize>,
        max_work: u64,
    ) -> Result<(Vec<(usize, usize)>, u64), ValidationError> {
        let mut output = Vec::new();
        let mut work = 0;
        for index in range {
            let neighbors = self.within(index, true, &mut work, max_work)?;
            if output.len() + neighbors.len() > MAX_INSTANCE_COUNT {
                return Err(limit("Neighbor pair allowance exhausted"));
            }
            output
                .try_reserve(neighbors.len())
                .map_err(|_| limit("Neighbor allocation failed"))?;
            output.extend(neighbors.into_iter().map(|other| (index, other)));
        }
        Ok((output, work))
    }

    /// Unordered pairs `(i, j)` with `i < j`, ordered by `i` then `j`. Large
    /// populations are split into contiguous index chunks evaluated in parallel
    /// and concatenated in order, so the result and the charged work are
    /// independent of the thread count.
    pub(crate) fn pairs(&self, max_work: u64) -> Result<Vec<(usize, usize)>, ValidationError> {
        let count = self.len();
        let base = count as u64;
        if base > max_work {
            return Err(limit("Neighbor preparation work allowance exhausted"));
        }
        let remaining = max_work - base;
        let threads = if count < PARALLEL_THRESHOLD {
            1
        } else {
            std::thread::available_parallelism()
                .map_or(1, usize::from)
                .min(MAX_NEIGHBOR_THREADS)
        };
        let chunks = if threads == 1 {
            vec![self.chunk_pairs(0..count, remaining)?]
        } else {
            let size = count.div_ceil(threads);
            std::thread::scope(|scope| {
                let handles = (0..count)
                    .step_by(size)
                    .map(|start| {
                        scope.spawn(move || {
                            self.chunk_pairs(start..(start + size).min(count), remaining)
                        })
                    })
                    .collect::<Vec<_>>();
                handles
                    .into_iter()
                    .map(|handle| {
                        handle
                            .join()
                            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
                    })
                    .collect::<Result<Vec<_>, _>>()
            })?
        };
        let work = chunks
            .iter()
            .try_fold(0u64, |total, (_, work)| total.checked_add(*work))
            .ok_or_else(|| limit("Neighbor work overflow"))?;
        if work > remaining {
            return Err(limit("Neighbor work allowance exhausted"));
        }
        let total = chunks.iter().map(|(pairs, _)| pairs.len()).sum::<usize>();
        if total > MAX_INSTANCE_COUNT {
            return Err(limit("Neighbor pair allowance exhausted"));
        }
        let mut output = Vec::new();
        output
            .try_reserve_exact(total)
            .map_err(|_| limit("Neighbor allocation failed"))?;
        for (pairs, _) in chunks {
            output.extend(pairs);
        }
        Ok(output)
    }
}

/// Convenience wrapper: builds an index and returns all pairs within `radius`.
pub(crate) fn pairs(
    particles: &[Particle],
    radius: f64,
    max_work: u64,
) -> Result<Vec<(usize, usize)>, ValidationError> {
    NeighborIndex::new(particles, radius)?.pairs(max_work)
}
