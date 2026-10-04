//! Explicit spawn refusals and opt-in storage observations.
//!
//! Describes boundary/capacity outcomes without deciding reproduction or logging.
//! Counters belong to observing shells; the ordinary tick uses a no-op observer.

use serde::{Deserialize, Serialize};

use crate::arena::AllocationFailure;
use crate::genome::{self, BodyTrait, Gene, GenomeError, Modality};
use crate::params::{BodyParams, StorageParams};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArenaKind {
    Genes,
    Neurons,
    Synapses,
    Sensors,
    Effectors,
    Parts,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpawnError {
    PoolFull,
    InvalidGenome(GenomeError),
    SensorParameters(&'static str),
    BodyTraits(&'static str),
    GenomeLimit {
        kind: &'static str,
        count: usize,
        limit: u32,
    },
    Arena {
        arena: ArenaKind,
        reason: AllocationFailure,
    },
}

impl core::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::PoolFull => f.write_str("agent pool is full"),
            Self::InvalidGenome(reason) => write!(f, "invalid genome: {reason:?}"),
            Self::SensorParameters(reason) => write!(f, "invalid sensor parameters: {reason}"),
            Self::BodyTraits(reason) => write!(f, "invalid body: {reason}"),
            Self::GenomeLimit { kind, count, limit } => {
                write!(f, "genome has {count} {kind}, exceeding limit {limit}")
            }
            Self::Arena { arena, reason } => write!(f, "{arena:?} storage: {reason}"),
        }
    }
}

impl core::error::Error for SpawnError {}

/// Sampled on demand; these are elements, not byte or resident-memory estimates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArenaUsage {
    pub arena: ArenaKind,
    pub capacity: u32,
    pub free_elements: u32,
    pub largest_free_block: u32,
    pub live_blocks: u32,
}

/// Optional shell-owned cumulative counts. Not authoritative simulation state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnFailureCounts {
    pub pool_full: u64,
    pub genome_limit: u64,
    pub arena_capacity: u64,
    pub arena_fragmentation: u64,
    pub arena_block_limit: u64,
    pub invalid_genome: u64,
}

impl SpawnFailureCounts {
    pub fn record(&mut self, error: SpawnError) {
        let counter = match error {
            SpawnError::PoolFull => &mut self.pool_full,
            SpawnError::InvalidGenome(_)
            | SpawnError::SensorParameters(_)
            | SpawnError::BodyTraits(_) => &mut self.invalid_genome,
            SpawnError::GenomeLimit { .. } => &mut self.genome_limit,
            SpawnError::Arena { reason, .. } => match reason {
                AllocationFailure::BlockLimit => &mut self.arena_block_limit,
                AllocationFailure::InsufficientSpace => &mut self.arena_capacity,
                AllocationFailure::Fragmented => &mut self.arena_fragmentation,
            },
        };
        *counter = counter.saturating_add(1);
    }
}

/// An imported body must lie inside the ranges every birth keeps bodies within, or one
/// agent could carry a size whose feeding reach validation never covered, or a muscle
/// whose force overflows (spec §3.5). `size` is the radius the spawn requests.
pub(crate) fn validate_body(
    genes: &[Gene],
    size: f32,
    body: &BodyParams,
) -> Result<(), SpawnError> {
    let inside = |value: f32, [low, high]: [f32; 2]| (low..=high).contains(&value);
    if !inside(size, body.size_range) {
        return Err(SpawnError::BodyTraits("size is outside body.size_range"));
    }
    for gene in genes {
        let Gene::Body(gene) = gene else { continue };
        let range = match gene.trait_ {
            BodyTrait::Size => body.size_range,
            BodyTrait::Muscle => body.muscle_range,
            BodyTrait::Mouth => body.mouth_range,
            BodyTrait::SignatureR | BodyTrait::SignatureG | BodyTrait::SignatureB => [0.0, 1.0],
        };
        if !inside(gene.value, range) {
            return Err(SpawnError::BodyTraits("a body trait is outside its range"));
        }
    }
    Ok(())
}

pub(crate) fn validate_limits(genes: &[Gene], limits: &StorageParams) -> Result<(), SpawnError> {
    let mut neurons = 0;
    let mut connections = 0;
    let mut sensors = 0;
    let mut rays = 0;
    let mut effectors = 0;
    for gene in genes {
        match gene {
            Gene::Neuron(_) => neurons += 1,
            Gene::Connection(_) => connections += 1,
            Gene::Sensor(sensor) => {
                sensors += 1;
                rays += usize::from(sensor.modality == Modality::VisionRay);
            }
            Gene::Effector(_) => effectors += 1,
            _ => {}
        }
    }
    for (kind, count, limit) in [
        ("genes", genes.len(), limits.max_genes),
        ("neurons", neurons, limits.max_neurons),
        ("connections", connections, limits.max_connections),
        ("sensors", sensors, limits.max_sensors),
        ("vision rays", rays, limits.max_vision_rays),
        ("effectors", effectors, limits.max_effectors),
    ] {
        if count > limit as usize {
            return Err(SpawnError::GenomeLimit { kind, count, limit });
        }
    }
    genome::validate_architecture(genes).map_err(SpawnError::InvalidGenome)
}

/// Check actual allocated sensing bounds, not retuned founder initialization values.
pub(crate) fn validate_sensor_parameters(
    genes: &[Gene],
    grid_cell: f32,
    channels: usize,
) -> Result<(), SpawnError> {
    for gene in genes {
        let Gene::Sensor(sensor) = gene else { continue };
        match sensor.modality {
            Modality::VisionRay => {
                if sensor.params[1] != 0.0 {
                    return Err(SpawnError::SensorParameters(
                        "vision elevation must be zero",
                    ));
                }
                if sensor.params[2] < 0.0 || sensor.params[2] > grid_cell {
                    return Err(SpawnError::SensorParameters(
                        "vision range exceeds the allocated envelope",
                    ));
                }
                if sensor.params[3] < 0.0 {
                    return Err(SpawnError::SensorParameters(
                        "vision field of view must be non-negative",
                    ));
                }
            }
            Modality::Chemo => {
                let channel = sensor.params[0];
                if channel < 0.0 || channel.fract() != 0.0 || channel as f64 >= channels as f64 {
                    return Err(SpawnError::SensorParameters(
                        "chemo channel must name an allocated channel",
                    ));
                }
                if sensor.params[1] < 0.0 || sensor.params[1] > grid_cell {
                    return Err(SpawnError::SensorParameters(
                        "chemo radius exceeds the allocated envelope",
                    ));
                }
            }
            Modality::Interoception => {
                if sensor.params[0] != 0.0 {
                    return Err(SpawnError::SensorParameters(
                        "only energy interoception is supported",
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_genome_limit_is_checked_at_the_boundary() {
        for kind in [
            "genes",
            "neurons",
            "connections",
            "sensors",
            "vision rays",
            "effectors",
        ] {
            let mut genes = genome::fixtures::tiny();
            let mut limits = StorageParams::default();
            match kind {
                "genes" => limits.max_genes = 0,
                "neurons" => limits.max_neurons = 0,
                "connections" => limits.max_connections = 0,
                "sensors" => limits.max_sensors = 0,
                "vision rays" => {
                    limits.max_vision_rays = 0;
                    for gene in &mut genes {
                        if let Gene::Sensor(sensor) = gene {
                            sensor.modality = Modality::VisionRay;
                            sensor.targets.fill(crate::InnovationId::new(0));
                        }
                    }
                }
                "effectors" => limits.max_effectors = 0,
                _ => unreachable!(),
            }
            assert!(matches!(validate_limits(&genes, &limits),
                Err(SpawnError::GenomeLimit { kind: found, .. }) if found == kind));
        }
    }

    #[test]
    fn observing_pressure_saturates_without_wrapping() {
        let mut counts = SpawnFailureCounts {
            arena_fragmentation: u64::MAX,
            ..Default::default()
        };
        counts.record(SpawnError::Arena {
            arena: ArenaKind::Genes,
            reason: AllocationFailure::Fragmented,
        });
        assert_eq!(counts.arena_fragmentation, u64::MAX);
        counts.record(SpawnError::PoolFull);
        assert_eq!(counts.pool_full, 1);
    }
}
