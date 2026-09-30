//! The tick: spec §2.4's eleven steps, in the one order that makes them reproducible.
//!
//! The order here is **normative**. It is not an implementation detail and not a
//! performance choice — changing it changes both behaviour and every seeded run that
//! ever ran, so the golden hash moves with it. Four properties it exists to guarantee,
//! each of which fails silently rather than loudly:
//!
//! - **Brains read a consistent world.** Perception is a whole pass before any brain
//!   steps, so no agent's decision depends on where it sits in the pool.
//! - **Intents are buffered, not applied.** Effectors write requests; movement and
//!   feeding drain them afterwards. Otherwise an agent acting early sees a different
//!   world than one acting late.
//! - **Births and deaths are deferred to step 10** and resolved in agent-index order.
//!   Mutating the pool mid-tick makes free-list allocation depend on iteration order.
//! - **Nothing iterates a hash map to drive simulation.** Every loop here walks
//!   `iter_live`, which is ascending agent index.
//!
//! The steps are `pub` as well as [`World::step`], because the test suite drives phases
//! in isolation on purpose: `steering.rs` runs perception through movement without the
//! economy, so what it measures is the sensorimotor chain and not a population's luck
//! with food. A shell should call `step`.
//!
//! Deliberately not here: what any step *does*. Each one gathers the slices belonging
//! to one agent and hands them to a system that takes plain slices and knows nothing
//! about a `World`.

use glam::Vec3;

use crate::agents::SpawnSpec;
use crate::brain;
use crate::effectors::{self, AgentIntents};
use crate::energy;
use crate::feeding;
use crate::genome::{self, BodyTrait, Gene};
use crate::history::Event as HistoryEvent;
use crate::metabolism;
use crate::movement;
use crate::mutate::{MutationState, structural::StructuralMutationEvent};
use crate::perceive::{self, SelfView, WorldView};
use crate::reproduction;
use crate::spawn::SpawnError;
use crate::species::SpeciesEvent;
use crate::world::World;

impl World {
    /// Advances the world one tick, in spec §2.4's order.
    ///
    /// Steps 6 (collision) and the rest of 7 (bite, grab) have no Phase 1 content and
    /// are absent rather than stubbed — an empty function in the sequence reads as a
    /// step that runs and does nothing, which is a different claim.
    ///
    /// Plant growth sits between feeding and the field update: it is where energy
    /// enters the world (spec §5.1), and its scent deposit is the "deposit" half of
    /// step 8, so it has to land before the diffuse and decay that follow it.
    pub fn step(&mut self) {
        self.step_with_spawn_observer(|_| {});
    }

    /// Opt-in observation; the ordinary step monomorphizes away the no-op callback.
    pub fn step_with_spawn_observer(&mut self, mut on_refusal: impl FnMut(SpawnError)) {
        self.step_with_observers(&mut on_refusal, |_| {});
    }

    pub fn step_with_observers(
        &mut self,
        mut on_refusal: impl FnMut(SpawnError),
        mut on_mutation: impl FnMut(StructuralMutationEvent),
    ) {
        self.step_with_all_observers(&mut on_refusal, &mut on_mutation, |_| {});
    }

    pub fn step_with_all_observers(
        &mut self,
        mut on_refusal: impl FnMut(SpawnError),
        mut on_mutation: impl FnMut(StructuralMutationEvent),
        mut on_species: impl FnMut(SpeciesEvent),
    ) {
        self.step_with_history_observer(
            &mut on_refusal,
            &mut on_mutation,
            &mut on_species,
            |_, _| {},
        );
    }

    pub fn step_with_history_observer(
        &mut self,
        mut on_refusal: impl FnMut(SpawnError),
        mut on_mutation: impl FnMut(StructuralMutationEvent),
        mut on_species: impl FnMut(SpeciesEvent),
        mut on_history: impl FnMut(HistoryEvent, Option<&[Gene]>),
    ) {
        // Before step 1, so an agent placed this tick gets a whole one (spec §2.2b).
        self.apply_commands_with_observers(&mut on_refusal, &mut on_species, &mut on_history);
        self.rebuild_spatial_hash(); // 1
        self.perceive_all(); // 2
        self.step_brains(); // 3
        self.drive_effectors(); // 4
        self.integrate_movement(); // 5
        self.resolve_feeding(); // 7
        self.grow_plants(); // 8, deposit
        self.update_chemo(); // 8, diffuse and decay
        self.charge_metabolism(); // 9
        self.resolve_deaths_with_history_observer(&mut on_species, &mut on_history); // 10
        self.resolve_births_with_history_observer(
            &mut on_refusal,
            &mut on_mutation,
            &mut on_species,
            &mut on_history,
        ); // 10
        self.advance_tick(); // 11
    }

    /// Runs every live agent's sensors and writes what they return into its brain.
    /// Step 2 of the tick (spec §2.4).
    ///
    /// A whole pass before [`Self::step_brains`], not fused with it: every agent must
    /// perceive the same world, and a fused loop would let agent 0's decision reach
    /// agent 1's eye within the same tick.
    pub fn perceive_all(&mut self) {
        for id in self.pool.iter_live() {
            let i = id.index();
            let agent = SelfView {
                index: id.raw(),
                position: self.agents.position[i],
                orientation: self.agents.orientation[i],
                energy_tanks: (energy::total(self.agents.energy[i], self.agents.energy_reserve[i])
                    / self.params.reproduction.start_energy as f64)
                    as f32,
            };
            let world = WorldView {
                positions: &self.agents.position,
                signatures: &self.agents.signature,
                sizes: &self.agents.size,
                hash: &self.hash,
                field: &self.field,
                plants: &self.plants,
                plant_radius: self.params.plants.radius,
                plant_signature: Vec3::from(self.params.plants.signature),
            };
            perceive::perceive(
                self.sensors.get(self.agents.sensors[i]),
                &agent,
                &world,
                self.brains.get_mut(self.agents.brain[i]),
            );
        }
    }

    /// Advances every live brain one Euler step. Step 3 of the tick (spec §2.4).
    ///
    /// Lives here rather than in `brain` for the reason `rebuild_spatial_hash` does:
    /// this is the only place that knows which blocks belong to which agent. The step
    /// itself takes plain slices and is testable without a world.
    ///
    /// Agent-index order, so that nothing about the result depends on pool layout —
    /// though with every brain reading only the previous step's outputs, the order is
    /// belt as well as braces here.
    pub fn step_brains(&mut self) {
        let dt = self.params.world.dt;
        for id in self.pool.iter_live() {
            let i = id.index();
            let (brain, synapses) = (self.agents.brain[i], self.agents.synapses[i]);
            brain::step(self.brains.get_mut(brain), self.synapses.get(synapses), dt);
        }
    }

    /// Reads every live agent's effectors into the intent buffer. Step 4 of the tick
    /// (spec §2.4).
    ///
    /// Nothing here changes the world. The buffer is cleared first, so an agent that
    /// lost an effector coasts rather than repeating its last request forever.
    pub fn drive_effectors(&mut self) {
        self.intents.clear();
        let movement = &self.params.movement;
        for id in self.pool.iter_live() {
            let i = id.index();
            effectors::drive(
                self.effectors.get(self.agents.effectors[i]),
                self.brains.get(self.agents.brain[i]),
                movement,
                &mut AgentIntents {
                    thrust: &mut self.intents.thrust[i],
                    turn: &mut self.intents.turn[i],
                    ingest: &mut self.intents.ingest[i],
                    reproduce: &mut self.intents.reproduce[i],
                },
            );
        }
    }

    /// Applies the movement intents. Step 5 of the tick (spec §2.4).
    ///
    /// Split from [`Self::drive_effectors`] rather than fused with it because every
    /// agent must decide against the same world: an agent early in the pool moving
    /// before a later one has chosen is exactly what the intent buffer exists to
    /// prevent.
    pub fn integrate_movement(&mut self) {
        for id in self.pool.iter_live() {
            let i = id.index();
            movement::integrate(
                &mut self.agents.position[i],
                &mut self.agents.velocity[i],
                &mut self.agents.orientation[i],
                self.intents.thrust[i],
                self.intents.turn[i],
                &self.params.movement,
                &self.params.world,
            );
        }
    }

    /// Diffuses and decays the pheromone field by one tick. Step 8 of the tick
    /// (spec §2.4), after whatever deposited into it.
    pub fn update_chemo(&mut self) {
        self.field.update(&self.params.chemo);
    }

    /// Rebuilds the neighbour grid from current positions. Step 1 of the tick.
    ///
    /// Lives here rather than in `spatial` because it is the only place that knows
    /// which slices belong together; the hash itself takes plain slices so it can be
    /// tested without a world.
    pub fn rebuild_spatial_hash(&mut self) {
        self.hash.rebuild(
            &self.agents.position,
            self.pool.alive_flags(),
            &mut self.agents.grid_cell,
        );
    }

    /// Grows the plants by one tick and scents the field. Part of step 8, and the only
    /// place energy enters the world (spec §5.1).
    ///
    /// What the plants *actually* absorbed goes into the ledger, not the nominal input
    /// rate: at carrying capacity the surplus never enters, and conservation has to be
    /// measured rather than inferred.
    pub fn grow_plants(&mut self) -> f64 {
        let dt = self.params.world.dt;
        let absorbed = self.plants.grow(&self.params.plants, dt);
        self.ledger.record_input(absorbed);
        self.plants.scent(&mut self.field, &self.params.plants, dt);
        absorbed
    }

    /// Charges every live agent its upkeep and notes who ran out. Step 9 of the tick
    /// (spec §2.4).
    ///
    /// An agent is charged only what it has left, so energy never goes negative and the
    /// amount dissipated is exactly the amount that existed. The death itself is
    /// deferred to step 10: mutating the pool here would make free-list allocation
    /// depend on iteration order, which is the fastest way to lose determinism.
    pub fn charge_metabolism(&mut self) {
        self.dying.clear();
        for id in self.pool.iter_live() {
            let i = id.index();
            let cost = metabolism::cost_per_tick(
                self.agents.size[i],
                self.agents.brain_units[i],
                self.agents.sensor_load[i],
                self.intents.thrust[i],
                &self.params.metabolism,
            );
            // Only what is there. Charging past zero would dissipate energy the world
            // never held, and the ledger would report a leak that is really an
            // overdraft.
            let charged = energy::take_amount(
                &mut self.agents.energy[i],
                &mut self.agents.energy_reserve[i],
                cost as f64,
            );
            self.ledger.record_dissipated_amount(charged);
            if energy::total(self.agents.energy[i], self.agents.energy_reserve[i]) <= 0.0 {
                self.dying.push(id);
            }
        }
    }

    /// Removes the agents that ran out of energy. Part of step 10 (spec §2.4).
    ///
    /// Agent-index order, because `iter_live` is ascending and that is what fills
    /// `dying`. Deaths resolved in any other order would hand the free list back in a
    /// different sequence and the next births would land in different slots.
    ///
    /// A starving agent holds no energy by the time it gets here — `charge_metabolism`
    /// took exactly what was left — so `despawn` finds nothing to dissipate. It would
    /// dissipate a remainder if there were one, which is what makes any *other* route
    /// to removal safe too. Corpses that return part of an agent to the world arrive
    /// with predation in Phase 3.
    pub fn resolve_deaths(&mut self) -> usize {
        self.resolve_deaths_with_observer(|_| {})
    }

    pub fn resolve_deaths_with_observer(
        &mut self,
        mut on_species: impl FnMut(SpeciesEvent),
    ) -> usize {
        self.resolve_deaths_with_history_observer(&mut on_species, |_, _| {})
    }

    pub fn resolve_deaths_with_history_observer(
        &mut self,
        mut on_species: impl FnMut(SpeciesEvent),
        mut on_history: impl FnMut(HistoryEvent, Option<&[Gene]>),
    ) -> usize {
        let dying = core::mem::take(&mut self.dying);
        let mut removed = 0;
        for &id in &dying {
            debug_assert!(
                self.agents.energy[id.index()] <= 0.0,
                "starvation should have drained this agent before step 10"
            );
            if self.despawn_with_history_observer(id, &mut on_species, &mut on_history) {
                removed += 1;
            }
        }
        self.dying = dying;
        self.dying.clear();
        removed
    }

    /// Moves energy from plants into the agents eating them. Step 7 of the tick
    /// (spec §2.4).
    ///
    /// A transfer, not a flow: nothing is recorded in the ledger, because the same
    /// joules are still in the world afterwards. If this is ever written wrongly the
    /// conservation test says so without knowing that eating exists.
    ///
    /// Agent-index order, because two agents can reach the same plant in one tick and
    /// the plant may not hold enough for both. Whoever is asked first gets what is
    /// there; resolving in any other order would make the outcome depend on pool
    /// layout.
    pub fn resolve_feeding(&mut self) {
        let feeding = self.params.feeding.clone();
        let plant_radius = self.params.plants.radius;
        for id in self.pool.iter_live() {
            let i = id.index();
            if self.intents.ingest[i] <= feeding.gate {
                continue;
            }
            let reach = self.agents.size[i] + plant_radius + feeding.reach;
            feeding::ingest(
                self.agents.position[i],
                reach,
                feeding.rate,
                &mut self.agents.energy[i],
                &mut self.agents.energy_reserve[i],
                &mut self.plants,
            );
        }
    }

    /// Notes which agents asked to reproduce and can afford to. Part of step 4's
    /// reading of the intent buffer, deferred like a death so the pool is not mutated
    /// mid-tick (spec §2.4).
    fn note_breeders(&mut self) {
        self.breeding.clear();
        for id in self.pool.iter_live() {
            let i = id.index();
            if reproduction::ready(
                self.agents.energy[i],
                self.agents.energy_reserve[i],
                self.agents.age[i],
                self.intents.reproduce[i],
                &self.params.reproduction,
            ) {
                self.breeding.push(id);
            }
        }
    }

    /// Creates the offspring of every agent that asked. The other half of step 10.
    ///
    /// The parent's energy is **split, not granted**: the offspring's tank comes out of
    /// the parent and nothing is recorded in the ledger, unlike a founder's. And the
    /// deduction happens only after the spawn succeeds — at the population ceiling a
    /// birth is refused, and charging a parent for a child that never existed would
    /// destroy energy on the busiest tick of a run.
    ///
    /// Agent-index order. Births hand out pool slots, so any other order would put the
    /// same population in different slots and every later tick would diverge.
    pub fn resolve_births(&mut self) -> usize {
        self.resolve_births_with_observer(|_| {})
    }

    pub fn resolve_births_with_observer(
        &mut self,
        mut on_refusal: impl FnMut(SpawnError),
    ) -> usize {
        self.resolve_births_with_observers(&mut on_refusal, |_| {})
    }

    pub fn resolve_births_with_observers(
        &mut self,
        mut on_refusal: impl FnMut(SpawnError),
        mut on_mutation: impl FnMut(StructuralMutationEvent),
    ) -> usize {
        self.resolve_births_with_all_observers(&mut on_refusal, &mut on_mutation, |_| {})
    }

    pub fn resolve_births_with_all_observers(
        &mut self,
        mut on_refusal: impl FnMut(SpawnError),
        mut on_mutation: impl FnMut(StructuralMutationEvent),
        mut on_species: impl FnMut(SpeciesEvent),
    ) -> usize {
        self.resolve_births_with_history_observer(
            &mut on_refusal,
            &mut on_mutation,
            &mut on_species,
            |_, _| {},
        )
    }

    pub fn resolve_births_with_history_observer(
        &mut self,
        mut on_refusal: impl FnMut(SpawnError),
        mut on_mutation: impl FnMut(StructuralMutationEvent),
        mut on_species: impl FnMut(SpeciesEvent),
        mut on_history: impl FnMut(HistoryEvent, Option<&[Gene]>),
    ) -> usize {
        self.note_breeders();
        let breeding = core::mem::take(&mut self.breeding);
        let mut born = 0;

        for &parent in &breeding {
            let p = parent.index();
            let split = self.params.reproduction.energy_split as f64;
            let visible_share = self.agents.energy[p] as f64 * split;
            let residual_share = self.agents.energy_reserve[p] * split;

            // Built before the spawn so the parent's genome can be read while the world
            // is otherwise untouched; mutation is what makes the child a variation
            // rather than a clone (spec §3.3).
            let mut scratch = core::mem::take(&mut self.genome_scratch);
            let genome = self.agents.genome[p];
            scratch.clear();
            scratch.extend_from_slice(self.genes.get(genome));
            self.brain_inheritance.prepare_offspring(
                &self.plan,
                &mut scratch,
                &self.params,
                &mut MutationState {
                    rng: &mut self.rng,
                    next_innovation: &mut self.next_innovation,
                    neuron_scratch: &mut self.brain_fan_in_scratch,
                },
                &mut on_mutation,
            );

            let position = reproduction::offspring_position(
                self.agents.position[p],
                &self.params.reproduction,
                self.params.world.size,
                &mut self.rng,
            );
            let yaw = self
                .rng
                .range(-core::f32::consts::PI, core::f32::consts::PI);
            let spec = SpawnSpec {
                position,
                yaw,
                energy: 0.0,
                size: genome::body_trait(&scratch, BodyTrait::Size)
                    .unwrap_or(self.params.body.size),
                signature: Vec3::new(
                    genome::body_trait(&scratch, BodyTrait::SignatureR).unwrap_or(0.5),
                    genome::body_trait(&scratch, BodyTrait::SignatureG).unwrap_or(0.5),
                    genome::body_trait(&scratch, BodyTrait::SignatureB).unwrap_or(0.5),
                ),
                parent_a: parent,
            };
            // Structural edits enforce coherence and per-genome bounds atomically
            // before the validated spawn fast path (spec section 3.3).
            let spawned = self.spawn_validated(&spec, &scratch, &mut on_species, &mut on_history);
            self.genome_scratch = scratch;

            if let Ok(child) = spawned {
                // Only now. A refused birth leaves the parent whole. The shared
                // rounding reserve keeps both f32 endpoints conservative (spec §5.1).
                let mut parent_energy = self.agents.energy[p];
                let c = child.index();
                let mut child_energy = self.agents.energy[c];
                let mut parent_reserve = self.agents.energy_reserve[p];
                let mut child_reserve = self.agents.energy_reserve[c];
                energy::transfer(
                    &mut parent_energy,
                    &mut parent_reserve,
                    &mut child_energy,
                    &mut child_reserve,
                    visible_share,
                );
                energy::transfer(
                    &mut parent_energy,
                    &mut parent_reserve,
                    &mut child_energy,
                    &mut child_reserve,
                    residual_share,
                );
                self.agents.energy[p] = parent_energy;
                self.agents.energy_reserve[p] = parent_reserve;
                self.agents.energy[c] = child_energy;
                self.agents.energy_reserve[c] = child_reserve;
                born += 1;
            } else if let Err(error) = spawned {
                on_refusal(error);
            }
        }

        self.breeding = breeding;
        self.breeding.clear();
        born
    }

    /// Ages every live agent and advances the clock. Step 11 of the tick (spec §2.4).
    pub fn advance_tick(&mut self) {
        for id in self.pool.iter_live() {
            // Saturating, so a world left running for a year does not wrap an agent's
            // age back to zero and make it eligible to breed again from nothing.
            let age = &mut self.agents.age[id.index()];
            *age = age.saturating_add(1);
        }
        self.tick += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::BrainInheritance;
    use crate::genome::Gene;
    use crate::params::SimParams;

    fn world_of(agents: u32, seed: u64) -> World {
        let mut params = SimParams::default();
        params.world.max_agents = agents.max(1);
        params.plants.max_plants = 64;
        let mut world = World::new(seed, params).expect("defaults are valid");
        let size = world.params().world.size;
        for i in 0..agents {
            let a = i as f32 * 2.399_963_2;
            world
                .spawn_founder(Vec3::new(
                    size * 0.5 + 40.0 * crate::math::cos(a),
                    size * 0.5 + 40.0 * crate::math::sin(a),
                    0.0,
                ))
                .expect("pool has room");
        }
        world
    }

    fn positions(world: &World) -> Vec<Vec3> {
        world
            .pool()
            .iter_live()
            .map(|id| world.agents().position[id.index()])
            .collect()
    }

    #[test]
    fn effectors_ask_and_movement_answers() {
        // Step 4 writes intents and changes nothing. If an effector moved its own agent,
        // one early in the pool would move before a later one had chosen, and every
        // decision after it would be against a world that had already shifted.
        let mut world = world_of(8, 3);
        world.rebuild_spatial_hash();
        world.perceive_all();
        world.step_brains();

        let before = positions(&world);
        world.drive_effectors();
        assert_eq!(before, positions(&world), "step 4 moved something");

        assert!(
            world.intents().thrust.iter().any(|&t| t != 0.0),
            "nothing asked to move, so the next assertion would pass vacuously"
        );
        world.integrate_movement();
        assert_ne!(before, positions(&world), "step 5 applied nothing");
    }

    #[test]
    fn a_starving_agent_lives_until_step_ten() {
        // Metabolism notes who ran out; only step 10 removes them. Despawning inside
        // step 9 would hand slots back to the free list part-way through a pass over
        // the pool, and the next birth would land somewhere that depended on how far
        // that pass had got.
        let mut params = SimParams::default();
        params.world.max_agents = 4;
        params.plants.max_plants = 8;
        // Fatal in a single tick, so the death is certain rather than eventual.
        params.metabolism.base = params.reproduction.start_energy * 2.0;
        let mut world = World::new(5, params).expect("valid params");
        world
            .spawn_founder(Vec3::new(100.0, 100.0, 0.0))
            .expect("room");

        world.charge_metabolism();
        assert_eq!(world.population(), 1, "step 9 removed an agent itself");
        assert_eq!(
            world.agents().energy[0],
            0.0,
            "the agent should be drained, just not yet gone"
        );

        assert_eq!(world.resolve_deaths(), 1);
        assert_eq!(world.population(), 0);
    }

    #[test]
    fn a_tick_is_deterministic() {
        // The property the golden hash will pin at the end of M8. Two worlds from one
        // seed must not merely agree on totals: every agent has to be in the same place
        // holding the same energy, or something in the tick is reading iteration order.
        let run = || {
            let mut world = world_of(24, 99);
            // Inside the idle lifetime on purpose. At 300 the founders have all starved
            // and this compares two empty worlds, which is what the assertion below
            // caught when it was written that way.
            for _ in 0..150 {
                world.step();
            }
            let state: Vec<(Vec3, f32, u32)> = world
                .pool()
                .iter_live()
                .map(|id| {
                    let i = id.index();
                    (
                        world.agents().position[i],
                        world.agents().energy[i],
                        world.agents().age[i],
                    )
                })
                .collect();
            (state, world.tick_count(), world.total_energy())
        };
        let (a, ta, ea) = run();
        let (b, tb, eb) = run();
        assert!(
            !a.is_empty(),
            "everything died, so this compares two empties"
        );
        assert_eq!(a, b);
        assert_eq!(ta, tb);
        assert_eq!(ea, eb);
    }

    #[test]
    fn the_clock_advances_once_per_tick() {
        let mut world = world_of(4, 7);
        assert_eq!(world.tick_count(), 0);
        for expected in 1..=10 {
            world.step();
            assert_eq!(world.tick_count(), expected);
        }
    }

    #[test]
    fn ages_track_the_clock() {
        // Maturity is measured in ticks (spec §5.4), so an age that advanced twice per
        // tick would let an agent breed at half the intended age and nothing else would
        // report it.
        let mut world = world_of(4, 11);
        for _ in 0..25 {
            world.step();
        }
        for id in world.pool().iter_live() {
            assert_eq!(world.agents().age[id.index()], 25);
        }
    }

    #[test]
    fn random_control_breaks_neural_heredity_at_birth() {
        let mut params = SimParams::default();
        params.world.max_agents = 4;
        params.plants.max_plants = 8;
        params.reproduction.maturity_ticks = 0;
        let mut world =
            World::new_with_brain_inheritance(17, params, BrainInheritance::RandomizedAtBirth)
                .expect("valid params");
        let parent = world
            .spawn_founder(Vec3::new(500.0, 500.0, 0.0))
            .expect("pool has room");
        let parent_genome = world.genome(parent).to_vec();
        let rich = world.params().reproduction.threshold + 100.0;
        world.agents_mut().energy[parent.index()] = rich;
        world.intents_mut().reproduce[parent.index()] = 1.0;

        assert_eq!(world.resolve_births(), 1);
        let child = world
            .pool()
            .iter_live()
            .find(|&id| id != parent)
            .expect("child was born");
        let child_genome = world.genome(child);

        let mut neural_change = false;
        for (parent_gene, child_gene) in parent_genome.iter().zip(child_genome) {
            match (parent_gene, child_gene) {
                (Gene::Neuron(_), Gene::Neuron(_)) | (Gene::Connection(_), Gene::Connection(_)) => {
                    neural_change |= parent_gene != child_gene;
                }
                _ => assert_eq!(
                    parent_gene, child_gene,
                    "control changed inherited non-neural genes"
                ),
            }
        }
        assert!(neural_change, "control child inherited its parent's brain");
    }
}
