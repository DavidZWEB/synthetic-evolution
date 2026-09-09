//! Bounded gene-buffer growth shared by neural and organ edits.
//!
//! Checks representation/storage capacity before callers mutate their candidate.
//! It does not select operators, issue innovations, or own genome storage.

use crate::genome::{self, Gene, Modality};
use crate::params::StorageParams;
use crate::rng::Rng;

use super::StructuralMutationResult;

#[derive(Clone, Copy, Default)]
pub(crate) struct Growth {
    pub neurons: usize,
    pub connections: usize,
    pub sensors: usize,
    pub vision_rays: usize,
}

fn fits(current: usize, extra: usize, limit: usize) -> bool {
    current
        .checked_add(extra)
        .is_some_and(|total| total <= limit)
}

pub(crate) fn select_index<T>(
    items: &[T],
    rng: &mut Rng,
    eligible: impl Fn(&T) -> bool,
) -> Option<usize> {
    let count = items.iter().filter(|item| eligible(item)).count();
    if count == 0 {
        return None;
    }
    let rank = rng.below(count as u32) as usize;
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| eligible(item))
        .nth(rank)
        .map(|(index, _)| index)
}

pub(crate) fn preflight_growth(
    genes: &Vec<Gene>,
    limits: &StorageParams,
    neuron_scratch: &[u32],
    growth: Growth,
) -> Result<(), StructuralMutationResult> {
    let neurons = genome::neuron_count(genes);
    let mut connections = 0;
    let mut sensors = 0;
    let mut rays = 0;
    for gene in genes {
        match gene {
            Gene::Connection(_) => connections += 1,
            Gene::Sensor(sensor) => {
                sensors += 1;
                rays += usize::from(sensor.modality == Modality::VisionRay);
            }
            _ => {}
        }
    }
    let extra_genes = growth
        .neurons
        .checked_add(growth.connections)
        .and_then(|count| count.checked_add(growth.sensors))
        .ok_or(StructuralMutationResult::GenomeLimit)?;
    if !fits(genes.len(), extra_genes, limits.max_genes as usize)
        || !fits(neurons, growth.neurons, limits.max_neurons as usize)
        || !fits(
            connections,
            growth.connections,
            limits.max_connections as usize,
        )
        || !fits(sensors, growth.sensors, limits.max_sensors as usize)
        || !fits(rays, growth.vision_rays, limits.max_vision_rays as usize)
    {
        return Err(StructuralMutationResult::GenomeLimit);
    }
    if !fits(genes.len(), extra_genes, genes.capacity())
        || !fits(neurons, growth.neurons, neuron_scratch.len())
    {
        return Err(StructuralMutationResult::ScratchLimit);
    }
    Ok(())
}

pub(crate) fn insert_gene(genes: &mut Vec<Gene>, gene: Gene) {
    debug_assert!(genes.len() < genes.capacity());
    let index = genes.partition_point(|existing| existing.sort_key() < gene.sort_key());
    genes.insert(index, gene);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn growth_arithmetic_cannot_wrap() {
        assert!(!fits(usize::MAX, 1, usize::MAX));
        assert!(fits(usize::MAX - 1, 1, usize::MAX));
    }
}
