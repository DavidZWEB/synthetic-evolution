//! `state_hash`: one number standing for everything a run has done to a world.
//!
//! The golden-hash test is the highest-value test in the project (spec §7.8), and this
//! is what it compares. Its value comes entirely from being **complete**: anything the
//! tick can change and this does not fold is a divergence the test cannot see, and a
//! golden hash with a hole in it is worse than none — it reports agreement that was
//! never checked.
//!
//! So the rule for this module is the opposite of the usual one. Elsewhere a field
//! nothing reads is dead weight; here a field nothing reads is folded anyway, because
//! the question is not "does this matter today" but "could a change to it go unnoticed".
//! `health` and `species_id` have no Phase 1 behaviour and are folded.
//!
//! Every variable-length run folds its **length first**. Without that, a genome of one
//! gene followed by a plant could produce the same bytes as no genes followed by a
//! differently-placed plant, and the hash would call two different worlds equal — the
//! one failure mode a golden test cannot survive.
//!
//! Variable-arena placement is future-relevant: freeing a particular agent's block
//! changes fragmentation and later birth success. Fold free spans and live handles,
//! but not stale contents of free space or optional shell-owned observations.

use crate::arena::VariableArena;
use crate::command::Kind;
use crate::genome::Gene;
use crate::world::World;

/// FNV-1a over 64 bits.
///
/// Not chosen for hash quality. `std::collections::hash_map::DefaultHasher` is
/// explicitly not stable across Rust releases, so a golden value built on it would move
/// when the toolchain moved — and every such move would look exactly like a behaviour
/// change, which is the one thing the golden test exists to distinguish. FNV-1a is a
/// published constant and a published loop: same number on every target, every
/// compiler, every version. That is the only property that matters here (spec §7.4).
#[derive(Clone, Debug)]
pub(crate) struct Fnv1a(u64);

impl Default for Fnv1a {
    fn default() -> Self {
        Self::new()
    }
}

impl Fnv1a {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    pub(crate) fn new() -> Self {
        Self(Self::OFFSET)
    }

    pub(crate) fn byte(&mut self, b: u8) {
        self.0 ^= b as u64;
        self.0 = self.0.wrapping_mul(Self::PRIME);
    }

    pub(crate) fn u32(&mut self, v: u32) {
        // Little-endian explicitly, not `to_ne_bytes`. Native order is a property of
        // the host, and native and wasm agreeing is the point (spec §7.4).
        for b in v.to_le_bytes() {
            self.byte(b);
        }
    }

    pub(crate) fn u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.byte(b);
        }
    }

    /// Floats fold by their **bits**, never by value.
    ///
    /// `-0.0 == 0.0` and `NaN != NaN`, so comparing values would call two different
    /// states equal and one identical state different. Bits do neither, and a NaN
    /// reaching the hash is a divergence that should be reported rather than smoothed
    /// over.
    pub(crate) fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }

    pub(crate) fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }

    pub(crate) fn bool(&mut self, v: bool) {
        self.byte(v as u8);
    }

    pub(crate) fn finish(&self) -> u64 {
        self.0
    }
}

fn fold_gene(h: &mut Fnv1a, gene: &Gene) {
    // The discriminant first, so a neuron and a connection holding the same numbers
    // cannot collide.
    match gene {
        Gene::Neuron(g) => {
            h.byte(0);
            h.u32(g.id.raw());
            h.f32(g.bias);
            h.f32(g.tau);
            h.byte(g.activation as u8);
            h.f32(g.period);
        }
        Gene::Sensor(g) => {
            h.byte(1);
            h.u32(g.id.raw());
            h.byte(g.modality as u8);
            for p in g.params {
                h.f32(p);
            }
            for t in g.targets {
                h.u32(t.raw());
            }
        }
        Gene::Effector(g) => {
            h.byte(2);
            h.u32(g.id.raw());
            h.byte(g.action as u8);
            for p in g.params {
                h.f32(p);
            }
            h.u32(g.source.raw());
        }
        Gene::Connection(g) => {
            h.byte(3);
            h.u32(g.id.raw());
            h.u32(g.from.raw());
            h.u32(g.to.raw());
            h.f32(g.weight);
            h.bool(g.enabled);
        }
        Gene::Body(g) => {
            h.byte(4);
            h.byte(g.trait_ as u8);
            h.f32(g.value);
        }
        Gene::Meta(g) => {
            h.byte(5);
            h.byte(g.trait_ as u8);
            h.f32(g.value);
        }
    }
}

fn fold_arena<T: Copy + Default>(h: &mut Fnv1a, arena: &VariableArena<T>) {
    h.u32(arena.capacity());
    h.u32(arena.live_blocks());
    h.u32(arena.free_elements());
    h.u32(arena.free_spans().len() as u32);
    for block in arena.free_spans() {
        h.u32(block.offset());
        h.u32(block.len());
    }
}

/// Stable fingerprint of one genome for telemetry and lineage counting.
///
/// This is not an identity assigned by the simulation and collisions remain possible;
/// it is a compact observation of every serialized gene field in deterministic order.
pub fn genome_fingerprint(genes: &[Gene]) -> u64 {
    let mut hash = Fnv1a::new();
    hash.u32(genes.len() as u32);
    for gene in genes {
        fold_gene(&mut hash, gene);
    }
    hash.finish()
}

impl World {
    /// Folds the whole world into one number.
    ///
    /// Walks live agents in ascending index and nothing else — never a `HashMap`, whose
    /// iteration order Rust deliberately varies (spec §2.4). Two runs of the same seed
    /// and params must agree on this exactly; if they do not, something in the tick is
    /// reading an order it should not be.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv1a::new();

        h.u64(self.tick_count());
        h.u32(self.population());

        // Slot reuse affects future births and observer identity (spec §2.2b, §2.4).
        let pool = self.pool();
        h.u32(pool.free_indices().len() as u32);
        for &index in pool.free_indices() {
            h.u32(index);
        }
        h.u32(pool.incarnations().len() as u32);
        for &incarnation in pool.incarnations() {
            h.u32(incarnation);
        }

        // The stream's position, not just its output so far. Two runs agreeing on every
        // agent but sitting at different points in the RNG diverge on the very next
        // draw, and a hash that missed that would certify a run that was already wrong.
        h.u64(self.rng.state_fingerprint());
        h.u32(self.next_innovation);

        let ledger = self.ledger();
        h.f64(ledger.input());
        h.f64(ledger.dissipated());
        h.f64(ledger.expected_stock());

        let agents = self.agents();
        for id in self.pool().iter_live() {
            let i = id.index();
            // The slot itself. Two worlds holding the same agents in different slots
            // will allocate differently on the next birth, so they are not the same
            // world even before anything visible differs.
            h.u32(id.raw());

            for v in [agents.position[i], agents.velocity[i], agents.signature[i]] {
                h.f32(v.x);
                h.f32(v.y);
                h.f32(v.z);
            }
            let q = agents.orientation[i];
            h.f32(q.x);
            h.f32(q.y);
            h.f32(q.z);
            h.f32(q.w);

            h.f32(agents.energy[i]);
            h.f64(agents.energy_reserve[i]);
            h.f32(agents.health[i]);
            h.u32(agents.age[i]);
            h.u32(agents.species_id[i]);
            h.f32(agents.size[i]);
            h.u32(agents.parent_a[i]);
            h.u32(agents.parent_b[i]);
            h.u32(agents.grid_cell[i]);
            h.u32(agents.brain_units[i]);
            h.f32(agents.sensor_load[i]);

            let genome = self.genome(id);
            h.u32(genome.len() as u32);
            for gene in genome {
                fold_gene(&mut h, gene);
            }

            // The genome cannot reconstruct recurrent memory or a newborn's silent
            // outputs. Inputs survive between perception and integration (spec §3.2).
            let brain = self.brain(id);
            h.u32(brain.len() as u32);
            for neuron in brain {
                h.f32(neuron.state);
                h.f32(neuron.output);
                h.f32(neuron.input);
            }

            let parts = self.parts_of(id);
            h.u32(parts.len() as u32);
            for &offset in parts {
                h.f32(offset);
            }
        }

        // Plants in seeding order, which never changes: positions are fixed at
        // construction and folding them catches a world seeded from a different stream
        // even when every agent still agrees.
        let plants = self.plants();
        h.u32(plants.len() as u32);
        for ((&p, &e), &reserve) in plants
            .position()
            .iter()
            .zip(plants.energy().iter())
            .zip(plants.energy_reserve().iter())
        {
            h.f32(p.x);
            h.f32(p.y);
            h.f32(e);
            h.f64(reserve);
        }

        // Commands not yet due are state too: two worlds identical in every other way
        // but holding different queues diverge on the tick those queues come due.
        h.u32(self.commands.len() as u32);
        for command in &self.commands {
            h.u64(command.apply_at_tick);
            match &command.kind {
                Kind::SpawnFounder { position } => {
                    h.byte(0);
                    h.f32(position.x);
                    h.f32(position.y);
                    h.f32(position.z);
                }
            }
        }

        // The field carries into the next tick, so it is state and not a view of state.
        let field = self.chemo();
        h.u32(field.channels() as u32);
        for c in 0..field.channels() {
            h.u32(field.channel(c).len() as u32);
            for &v in field.channel(c) {
                h.f32(v);
            }
        }

        // First-fit placement affects which later births fit, even when every live
        // genome and every free-element count is identical (spec section 2.2a).
        fold_arena(&mut h, &self.genes);
        fold_arena(&mut h, &self.brains);
        fold_arena(&mut h, &self.synapses);
        fold_arena(&mut h, &self.sensors);
        fold_arena(&mut h, &self.effectors);
        for id in self.pool.iter_live() {
            let i = id.index();
            for block in [
                self.agents.genome[i],
                self.agents.brain[i],
                self.agents.synapses[i],
                self.agents.sensors[i],
                self.agents.effectors[i],
            ] {
                h.u32(block.offset());
                h.u32(block.len());
            }
        }

        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::SimParams;
    use glam::Vec3;

    #[test]
    fn variable_capacity_is_future_relevant_even_in_an_empty_world() {
        let mut params = SimParams::default();
        params.world.max_agents = 4;
        let a = World::new(42, params.clone()).unwrap();
        params.storage.genes_per_slot += 1;
        let b = World::new(42, params).unwrap();
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn live_genome_placement_is_hashed_even_when_contents_and_free_spans_match() {
        let make = || {
            let mut params = SimParams::default();
            params.world.max_agents = 4;
            let mut world = World::new(42, params).unwrap();
            let spec = crate::SpawnSpec {
                position: Vec3::ZERO,
                yaw: 0.0,
                energy: 0.0,
                size: 1.0,
                signature: Vec3::ONE,
                parent_a: crate::AgentId::NULL,
            };
            for _ in 0..2 {
                world
                    .spawn(&spec, &crate::genome::fixtures::tiny())
                    .unwrap();
            }
            world
        };
        let a = make();
        let mut b = make();
        assert_eq!(a.state_hash(), b.state_hash());
        b.agents.genome.swap(0, 1);
        assert_eq!(a.storage_usage(), b.storage_usage());
        for id in a.pool.iter_live() {
            assert_eq!(a.genome(id), b.genome(id));
        }
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn free_space_payload_is_not_authoritative() {
        let mut world = populated(42);
        let before = world.state_hash();
        let block = world.genes.alloc(1).unwrap();
        assert_ne!(
            world.state_hash(),
            before,
            "unreturned claims must be visible"
        );
        world.genes.get_mut(block)[0] = crate::genome::fixtures::tiny()[0];
        world.genes.free(block);
        assert_eq!(
            world.state_hash(),
            before,
            "unused payload is reset before reuse"
        );
    }

    fn populated(seed: u64) -> World {
        let mut params = SimParams::default();
        params.world.max_agents = 16;
        params.plants.max_plants = 32;
        let mut world = World::new(seed, params).expect("defaults are valid");
        for i in 0..8u32 {
            let a = i as f32 * 2.399_963_2;
            world
                .spawn_founder(Vec3::new(
                    500.0 + 30.0 * crate::math::cos(a),
                    500.0 + 30.0 * crate::math::sin(a),
                    0.0,
                ))
                .expect("pool has room");
        }
        world
    }

    #[test]
    fn fnv_matches_the_published_vectors() {
        // The point of FNV over a std hasher is that it is specified, so it is worth
        // checking against the specification rather than against itself.
        let hash = |s: &str| {
            let mut h = Fnv1a::new();
            for b in s.bytes() {
                h.byte(b);
            }
            h.finish()
        };
        assert_eq!(Fnv1a::new().finish(), 0xcbf2_9ce4_8422_2325);
        assert_eq!(hash("a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(hash("foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn the_same_seed_hashes_the_same() {
        let (mut a, mut b) = (populated(4), populated(4));
        assert_eq!(a.state_hash(), b.state_hash(), "differed before any tick");
        for _ in 0..120 {
            a.step();
            b.step();
        }
        assert_eq!(a.state_hash(), b.state_hash());
        assert!(a.population() > 0, "everything died; the tick did nothing");
    }

    #[test]
    fn a_different_seed_hashes_differently() {
        assert_ne!(populated(4).state_hash(), populated(5).state_hash());
    }

    #[test]
    fn every_tick_moves_the_hash() {
        // A hash that only folded, say, population would sit still for most of a run
        // and pass this file's other tests while catching nothing.
        let mut world = populated(9);
        let mut seen = vec![world.state_hash()];
        for _ in 0..40 {
            world.step();
            seen.push(world.state_hash());
        }
        let mut unique = seen.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), seen.len(), "the hash repeated across ticks");
    }

    #[test]
    fn moving_one_agent_moves_the_hash() {
        let mut world = populated(2);
        let before = world.state_hash();
        world.agents_mut().position[0].x += 0.5;
        assert_ne!(before, world.state_hash());
    }

    #[test]
    fn advancing_only_recurrent_brain_state_moves_the_hash() {
        let mut world = populated(2);
        let before = world.state_hash();
        world.step_brains();
        assert_eq!(world.tick_count(), 0);
        assert_ne!(
            before,
            world.state_hash(),
            "neural memory changed without moving the world hash"
        );
    }

    #[test]
    fn every_recurrent_neuron_value_is_folded() {
        type Change = fn(&mut crate::brain::Neuron);
        let changes: [(&str, Change); 3] = [
            ("state", |neuron| neuron.state += 0.25),
            ("output", |neuron| neuron.output += 0.25),
            ("input", |neuron| neuron.input += 0.25),
        ];
        for (name, change) in changes {
            let mut world = populated(2);
            let before = world.state_hash();
            let block = world.agents.brain[0];
            change(&mut world.brains.get_mut(block)[0]);
            assert_ne!(before, world.state_hash(), "{name} was not folded");
        }
    }

    #[test]
    fn part_values_are_folded_without_their_arena_addresses() {
        let mut world = populated(2);
        let before = world.state_hash();
        let block = world.agents.parts[0];
        world.parts.get_mut(block)[0] += 0.25;
        assert_ne!(before, world.state_hash());
    }

    #[test]
    fn dead_slot_incarnations_are_folded() {
        let mut world = populated(2);
        let before = world.state_hash();
        let id = world.pool.alloc().expect("a free slot");
        assert!(world.pool.free(id));
        assert_ne!(before, world.state_hash());
    }

    #[test]
    fn free_slot_order_is_part_of_the_world_state() {
        let (mut a, mut b) = (populated(2), populated(2));
        let ids: Vec<_> = a.pool().iter_live().take(2).collect();
        for &id in &ids {
            assert!(a.despawn(id));
        }
        for &id in ids.iter().rev() {
            assert!(b.despawn(id));
        }
        let hashes = (a.state_hash(), b.state_hash());
        assert_ne!(
            a.spawn_founder(Vec3::ZERO),
            b.spawn_founder(Vec3::ZERO),
            "the different free lists must change the next allocated agent index"
        );
        assert_ne!(
            hashes.0, hashes.1,
            "different future allocation order was invisible to the hash"
        );
    }

    #[test]
    fn a_float_folds_by_bits_not_by_value() {
        // -0.0 == 0.0, so a hash comparing values would call two genuinely different
        // states identical. They divide differently, so they are not the same state.
        let bits = |v: f32| {
            let mut h = Fnv1a::new();
            h.f32(v);
            h.finish()
        };
        assert_ne!(bits(0.0), bits(-0.0));
    }

    #[test]
    fn genome_fingerprint_tracks_neural_scalars() {
        let world = populated(1);
        let id = world.pool().iter_live().next().expect("one agent");
        let a = world.genome(id).to_vec();
        let mut b = a.clone();
        let connection = b
            .iter_mut()
            .find_map(|gene| match gene {
                Gene::Connection(connection) => Some(connection),
                _ => None,
            })
            .expect("founder has connections");
        connection.weight += 0.25;
        assert_eq!(genome_fingerprint(&a), genome_fingerprint(&a));
        assert_ne!(genome_fingerprint(&a), genome_fingerprint(&b));
    }

    #[test]
    fn the_rng_position_is_folded() {
        // Two worlds identical in every visible way but at different points in the
        // stream diverge on the next draw. Without this the hash would certify them
        // equal right up until they were not.
        let mut world = populated(6);
        let before = world.state_hash();
        let _ = world.rng_mut().next_u64();
        assert_ne!(before, world.state_hash());
    }
}
