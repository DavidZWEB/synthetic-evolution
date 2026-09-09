//! Bounded classification against immutable active-species representatives.
//!
//! Owns representative copies, monotonic IDs, and membership totals (spec §3.4).
//! It does not spawn agents, choose a threshold default, or retain ancestry/history.

use std::collections::TryReserveError;

use crate::arena::{AllocationFailure, ArenaBuildError, Block, VariableArena};
use crate::distance;
use crate::genome::{self, Gene};
use crate::ids::{NULL_ID, SpeciesId};
use crate::params::{DistanceParams, ParamError};

mod events;
mod hash;
pub use events::{SpeciesEvent, SpeciesEventCounts};

#[derive(Debug)]
pub enum BuildError {
    Parameters(ParamError),
    Arena(ArenaBuildError),
    Reservation(TryReserveError),
}

impl core::fmt::Display for BuildError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Parameters(error) => error.fmt(f),
            Self::Arena(error) => error.fmt(f),
            Self::Reservation(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for BuildError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(match self {
            Self::Parameters(error) => error,
            Self::Arena(error) => error,
            Self::Reservation(error) => error,
        })
    }
}

/// Classification failures are not permission to refuse an ecological birth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unclassified {
    Capacity,
    IdExhausted,
    GenomeTooLarge,
    MemberCountExhausted,
    Storage(AllocationFailure),
}

impl core::fmt::Display for Unclassified {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Capacity => f.write_str("species representative capacity exhausted"),
            Self::IdExhausted => f.write_str("species ID space exhausted"),
            Self::GenomeTooLarge => f.write_str("genome exceeds representative storage limit"),
            Self::MemberCountExhausted => f.write_str("species member count exhausted"),
            Self::Storage(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for Unclassified {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Assignment {
    pub species: SpeciesId,
    pub created: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Departure {
    MemberRemoved,
    /// Returned exactly once when the last counted member leaves.
    Extinct,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnknownSpecies(pub SpeciesId);

impl core::fmt::Display for UnknownSpecies {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "species {} is not active", self.0.raw())
    }
}

impl core::error::Error for UnknownSpecies {}

#[derive(Clone, Copy, Debug)]
struct Entry {
    id: SpeciesId,
    block: Block,
    genome_len: u32,
    members: u32,
}

/// A caller owns admission/death order and records each successful assignment.
///
/// Call `classify` once per admitted member and `remove_member` once on its death.
/// Failed classification creates no membership; an unclassified agent is never
/// relabeled by this component. Its descendants may be classified independently.
#[derive(Debug)]
pub struct Classifier {
    representatives: VariableArena<Gene>,
    entries: Vec<Entry>,
    capacity: u32,
    max_genes: u32,
    next_id: u32,
    coefficients: DistanceParams,
    threshold: f64,
}

impl Classifier {
    /// Requested heap bytes, shared with future world construction accounting.
    ///
    /// Reserves `max_genes` for every representative, even a short or empty genome.
    /// Includes the arena's span metadata and the active-entry buffer.
    pub fn estimated_construction_bytes(capacity: u32, max_genes: u32) -> Result<u64, ParamError> {
        Self::construction_layout(capacity, max_genes).map(|(_, bytes)| bytes)
    }

    fn construction_layout(capacity: u32, max_genes: u32) -> Result<(u32, u64), ParamError> {
        if max_genes == 0 {
            return Err(ParamError("species max_genes must be positive"));
        }
        let genes = capacity.checked_mul(max_genes).ok_or(ParamError(
            "species capacity times max_genes exceeds u32 capacity",
        ))?;
        let spans = if genes == 0 {
            0
        } else {
            u64::from(capacity) + 1
        };
        let mut total = 0;
        for (count, width) in [
            (u64::from(genes), size_of::<Gene>() as u64),
            (spans, size_of::<Block>() as u64),
            (u64::from(capacity), size_of::<Entry>() as u64),
        ] {
            let bytes = count
                .checked_mul(width)
                .ok_or(ParamError("species buffer byte count overflows"))?;
            if bytes > i32::MAX as u64 {
                return Err(ParamError(
                    "species buffer exceeds the portable byte ceiling",
                ));
            }
            total += bytes;
        }
        Ok((genes, total))
    }

    /// Copies and freezes the coefficients and explicit threshold at construction.
    ///
    /// A zero capacity permits no classifications and allocates no backing storage.
    /// The caller supplies a memory ceiling; host reservation failures remain errors.
    pub fn try_new(
        capacity: u32,
        max_genes: u32,
        threshold: f64,
        coefficients: DistanceParams,
        max_memory_bytes: u64,
    ) -> Result<Self, BuildError> {
        coefficients.validate().map_err(BuildError::Parameters)?;
        if !threshold.is_finite() || threshold <= 0.0 {
            return Err(BuildError::Parameters(ParamError(
                "species threshold must be finite and positive",
            )));
        }
        let (gene_capacity, bytes) =
            Self::construction_layout(capacity, max_genes).map_err(BuildError::Parameters)?;
        if bytes > max_memory_bytes {
            return Err(BuildError::Parameters(ParamError(
                "species construction exceeds the supplied memory budget",
            )));
        }
        let representatives =
            VariableArena::try_with_capacity(gene_capacity, capacity).map_err(BuildError::Arena)?;
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(capacity as usize)
            .map_err(BuildError::Reservation)?;
        Ok(Self {
            representatives,
            entries,
            capacity,
            max_genes,
            next_id: 0,
            coefficients,
            threshold,
        })
    }

    /// Assign one coherent genome accepted by `genome::validate`.
    ///
    /// Scan active representatives only; use strict `< threshold`, nearest distance,
    /// then lowest historical ID. Refusals change no state or counters. Existing
    /// compatible species remain usable when new-species storage/IDs are exhausted.
    pub fn classify(&mut self, genes: &[Gene]) -> Result<Assignment, Unclassified> {
        debug_assert!(genome::validate(genes).is_ok(), "genome must be coherent");
        if genes.len() > self.max_genes as usize {
            return Err(Unclassified::GenomeTooLarge);
        }
        let mut nearest: Option<(usize, f64)> = None;
        for (index, entry) in self.entries.iter().enumerate() {
            let representative =
                &self.representatives.get(entry.block)[..entry.genome_len as usize];
            let value = distance::between(genes, representative, &self.coefficients).value;
            if value < self.threshold
                && nearest.is_none_or(|(best, best_value)| {
                    value < best_value || (value == best_value && entry.id < self.entries[best].id)
                })
            {
                nearest = Some((index, value));
            }
        }
        if let Some((index, _)) = nearest {
            let entry = &mut self.entries[index];
            entry.members = entry
                .members
                .checked_add(1)
                .ok_or(Unclassified::MemberCountExhausted)?;
            return Ok(Assignment {
                species: entry.id,
                created: false,
            });
        }
        if self.entries.len() == self.capacity as usize {
            return Err(Unclassified::Capacity);
        }
        if self.next_id == NULL_ID {
            return Err(Unclassified::IdExhausted);
        }
        // Equal-size full reservations make fragmentation impossible and guarantee
        // each admitted representative its maximum size (spec §3.4).
        let block = self
            .representatives
            .alloc(self.max_genes)
            .map_err(Unclassified::Storage)?;
        self.representatives.get_mut(block)[..genes.len()].copy_from_slice(genes);
        let id = SpeciesId::new(self.next_id);
        self.next_id += 1;
        self.entries.push(Entry {
            id,
            block,
            genome_len: genes.len() as u32,
            members: 1,
        });
        Ok(Assignment {
            species: id,
            created: true,
        })
    }

    /// Remove one previously counted member; the representative survives until zero.
    ///
    /// Unknown/null/retired IDs return an explicit error without changing counts.
    /// Extinction frees storage, never identity; a later arrival gets a fresh ID.
    pub fn remove_member(&mut self, id: SpeciesId) -> Result<Departure, UnknownSpecies> {
        let index = self.index(id).ok_or(UnknownSpecies(id))?;
        let entry = &mut self.entries[index];
        if entry.members > 1 {
            entry.members -= 1;
            return Ok(Departure::MemberRemoved);
        }
        let removed = self.entries.remove(index);
        self.representatives.free(removed.block);
        Ok(Departure::Extinct)
    }

    pub fn representative(&self, id: SpeciesId) -> Option<&[Gene]> {
        let entry = &self.entries[self.index(id)?];
        Some(&self.representatives.get(entry.block)[..entry.genome_len as usize])
    }

    /// Active IDs and member counts in ascending historical-ID order.
    pub fn active(&self) -> impl Iterator<Item = (SpeciesId, u32)> + '_ {
        self.entries.iter().map(|entry| (entry.id, entry.members))
    }

    fn index(&self, id: SpeciesId) -> Option<usize> {
        self.entries
            .binary_search_by_key(&id, |entry| entry.id)
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::NeuronGene;
    use crate::ids::InnovationId;

    fn gene(id: u32) -> [Gene; 1] {
        [Gene::Neuron(NeuronGene {
            id: InnovationId::new(id),
            tau: 1.0,
            ..Default::default()
        })]
    }

    #[test]
    fn exhausted_ids_never_wrap_and_do_not_prevent_existing_membership() {
        let mut classifier =
            Classifier::try_new(2, 1, 0.5, DistanceParams::default(), 1024).unwrap();
        classifier.next_id = NULL_ID - 1;
        let last = classifier.classify(&gene(0)).unwrap();
        assert_eq!(last.species.raw(), NULL_ID - 1);
        assert_eq!(
            classifier.classify(&gene(1)),
            Err(Unclassified::IdExhausted)
        );
        assert_eq!(classifier.classify(&gene(0)).unwrap().species, last.species);
        classifier.remove_member(last.species).unwrap();
        assert_eq!(
            classifier.remove_member(last.species),
            Ok(Departure::Extinct)
        );
        assert_eq!(
            classifier.classify(&gene(0)),
            Err(Unclassified::IdExhausted)
        );
        assert_eq!(classifier.next_id, NULL_ID);
    }

    #[test]
    fn member_overflow_is_explicit_and_atomic() {
        let mut classifier =
            Classifier::try_new(1, 1, 0.5, DistanceParams::default(), 1024).unwrap();
        let id = classifier.classify(&gene(0)).unwrap().species;
        classifier.entries[0].members = u32::MAX;
        assert_eq!(
            classifier.classify(&gene(0)),
            Err(Unclassified::MemberCountExhausted)
        );
        assert_eq!(classifier.active().next(), Some((id, u32::MAX)));
        classifier.remove_member(id).unwrap();
        assert_eq!(classifier.classify(&gene(0)).unwrap().species, id);
        assert_eq!(classifier.active().next(), Some((id, u32::MAX)));
    }

    #[test]
    fn world_id_exhaustion_keeps_admissions_and_existing_membership_available() {
        use crate::{AgentId, SimParams, SpawnSpec, World};
        use glam::Vec3;
        let mut params = SimParams::default();
        params.world.max_agents = 3;
        params.plants.max_plants = 0;
        params.species.capacity = 2;
        params.species.threshold = f64::MIN_POSITIVE;
        let mut world = World::new(42, params).unwrap();
        let first = world.spawn_founder(Vec3::ZERO).unwrap();
        let genes = world.genome(first).to_vec();
        world.classifier.next_id = NULL_ID;
        let mut events = SpeciesEventCounts::default();
        let unclassified = world
            .spawn_founder_with_species_observer(Vec3::ZERO, |event| events.record(event))
            .unwrap();
        assert_eq!(world.agents().species_id[unclassified.index()], NULL_ID);
        assert_eq!(events.unclassified_id_exhausted, 1);
        let assigned = world
            .spawn_with_species_observer(
                &SpawnSpec {
                    position: Vec3::ZERO,
                    yaw: 0.0,
                    energy: 0.0,
                    size: 3.0,
                    signature: Vec3::ONE,
                    parent_a: AgentId::NULL,
                },
                &genes,
                |event| events.record(event),
            )
            .unwrap();
        assert_eq!(world.agents().species_id[assigned.index()], 0);
        assert_eq!(world.population(), 3);
        assert_eq!(world.unclassified_population(), 1);
        assert_eq!(
            world.species().active().collect::<Vec<_>>(),
            vec![(SpeciesId::new(0), 2)]
        );
        assert_eq!(events.created, 0);
    }
}
