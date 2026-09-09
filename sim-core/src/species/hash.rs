//! Canonical hashing of authoritative classifier state.
//!
//! Includes future assignment policy, identity, storage placement, and owned genomes.
//! Unused gene payload and optional observer counts are deliberately excluded (§7.8).

use super::Classifier;
use crate::state_hash::{Fnv1a, fold_arena, fold_gene};

impl Classifier {
    pub(crate) fn fold_state(&self, hash: &mut Fnv1a) {
        hash.u32(self.capacity);
        hash.u32(self.max_genes);
        hash.u32(self.next_id);
        hash.f64(self.threshold);
        hash.f32(self.coefficients.disjoint_coefficient);
        hash.f32(self.coefficients.excess_coefficient);
        hash.f32(self.coefficients.weight_coefficient);
        fold_arena(hash, &self.representatives);
        hash.u32(self.entries.len() as u32);
        for entry in &self.entries {
            hash.u32(entry.id.raw());
            hash.u32(entry.members);
            hash.u32(entry.block.offset());
            hash.u32(entry.block.len());
            hash.u32(entry.genome_len);
            for gene in &self.representatives.get(entry.block)[..entry.genome_len as usize] {
                fold_gene(hash, gene);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::{Gene, NeuronGene};
    use crate::ids::InnovationId;
    use crate::params::DistanceParams;
    use crate::{SimParams, World};

    fn hash(classifier: &Classifier) -> u64 {
        let mut hash = Fnv1a::new();
        classifier.fold_state(&mut hash);
        hash.finish()
    }

    fn genes() -> [Gene; 1] {
        [Gene::Neuron(NeuronGene {
            id: InnovationId::new(0),
            tau: 1.0,
            ..Default::default()
        })]
    }

    fn classifier() -> Classifier {
        Classifier::try_new(2, 4, 0.5, DistanceParams::default(), 1024).unwrap()
    }

    #[test]
    fn frozen_policy_and_next_identity_are_hashed_even_without_members() {
        let original = hash(&classifier());
        let mut changed = classifier();
        changed.next_id += 1;
        assert_ne!(hash(&changed), original);
        let mut changed = classifier();
        changed.threshold = 0.75;
        assert_ne!(hash(&changed), original);
        let mut changed = classifier();
        changed.coefficients.weight_coefficient = 0.75;
        assert_ne!(hash(&changed), original);
        assert_ne!(
            hash(&Classifier::try_new(1, 4, 0.5, DistanceParams::default(), 1024).unwrap()),
            original
        );
        assert_ne!(
            hash(&Classifier::try_new(2, 3, 0.5, DistanceParams::default(), 1024).unwrap()),
            original
        );
    }

    #[test]
    fn owned_genomes_members_and_historical_ids_are_authoritative() {
        let mut changed = classifier();
        changed.classify(&genes()).unwrap();
        let original = hash(&changed);
        changed.classify(&genes()).unwrap();
        assert_ne!(hash(&changed), original);
        changed.entries[0].members = 1;
        assert_eq!(hash(&changed), original);
        changed.entries[0].id = crate::ids::SpeciesId::new(1);
        assert_ne!(hash(&changed), original);
        changed.entries[0].id = crate::ids::SpeciesId::new(0);
        let block = changed.entries[0].block;
        let Gene::Neuron(neuron) = &mut changed.representatives.get_mut(block)[0] else {
            unreachable!()
        };
        neuron.bias = 0.25;
        assert_ne!(
            hash(&changed),
            original,
            "even distance-ignored representative fields are owned state"
        );
    }

    #[test]
    fn unused_reserved_payload_does_not_change_identity() {
        let mut classifier = classifier();
        let first = classifier.classify(&genes()).unwrap().species;
        let block = classifier.entries[0].block;
        let original = hash(&classifier);
        classifier.representatives.get_mut(block)[1] = genes()[0];
        assert_eq!(
            hash(&classifier),
            original,
            "tail beyond genome_len is not owned genome data"
        );
        classifier.remove_member(first).unwrap();
        let retired = hash(&classifier);
        let temporary = classifier.representatives.alloc(4).unwrap();
        classifier.representatives.get_mut(temporary)[0] = genes()[0];
        classifier.representatives.free(temporary);
        assert_eq!(
            hash(&classifier),
            retired,
            "freed payload must remain irrelevant"
        );
    }

    #[test]
    fn world_hash_includes_classifier_and_unclassified_count_but_ecology_hash_does_not() {
        let mut params = SimParams::default();
        params.world.max_agents = 2;
        params.plants.max_plants = 0;
        params.species.capacity = 1;
        let mut world = World::new(42, params).unwrap();
        let full = world.state_hash();
        let ecology = world.ecology_hash();
        world.classifier.next_id += 1;
        assert_ne!(world.state_hash(), full);
        assert_eq!(world.ecology_hash(), ecology);
        world.classifier.next_id -= 1;
        assert_eq!(world.state_hash(), full);
        world.unclassified += 1;
        assert_ne!(world.state_hash(), full);
        assert_eq!(world.ecology_hash(), ecology);
    }
}
