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
use crate::combat::{self, Swing};
use crate::control::{self, BrainInheritance};
use crate::effectors::{self, AgentIntents};
use crate::energy;
use crate::feeding;
use crate::genome::{self, BodyTrait, Gene};
use crate::history::Event as HistoryEvent;
use crate::ids::AgentId;
use crate::math;
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
    /// Step 6 (collision) and the rest of 7 (grab) have no content yet and are absent
    /// rather than stubbed — an empty function in the sequence reads as a step that
    /// runs and does nothing, which is a different claim.
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
        self.resolve_bites(); // 7, before anyone eats
        self.resolve_feeding(); // 7
        self.grow_plants(); // 8, deposit
        self.update_chemo(); // 8, diffuse and decay
        self.charge_metabolism(); // 9
        self.recover_from_bites(); // 9
        self.decay_corpses(); // 9
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
                corpses: &self.corpses,
                corpse_radius: self.params.corpses.radius,
                corpse_signature: Vec3::from(self.params.corpses.signature),
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
                    bite: &mut self.intents.bite[i],
                    bite_azimuth: &mut self.intents.bite_azimuth[i],
                    bite_reach: &mut self.intents.bite_reach[i],
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
            // Muscle scales the force a drive produces and mass divides it, so a large
            // body is sluggish unless it pays for muscle (spec §3.5). Both factors are
            // exactly 1 at the reference body, so Phase 2 movement is unchanged.
            let relative = self.agents.size[i] / self.params.body.size;
            let force = self.intents.thrust[i] * self.agents.muscle[i];
            movement::integrate(
                &mut self.agents.position[i],
                &mut self.agents.velocity[i],
                &mut self.agents.orientation[i],
                force / (relative * relative),
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

    /// Rebuilds the neighbour grid from current positions, and the corpse grid if the
    /// last tick's deaths or decomposition changed it. Step 1 of the tick.
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
        self.corpses.settle();
    }

    /// Grows the plants by one tick, resolves plant deaths and reseeds, and scents the
    /// field. Part of step 8, and the only place energy enters the world (spec §5.1).
    ///
    /// What the plants *actually* absorbed goes into the ledger, not the nominal input
    /// rate: at carrying capacity the surplus never enters, and conservation has to be
    /// measured rather than inferred. Deaths come after growth, so a plant that just
    /// regrew past starving survives, and before scent, so the field follows the plants
    /// to their new sites.
    pub fn grow_plants(&mut self) -> f64 {
        let dt = self.params.world.dt;
        let absorbed = self.plants.grow(&self.params.plants, dt);
        self.ledger.record_input(absorbed);
        self.plants.turn_over(&self.params, &mut self.rng);
        self.plants.scent(&mut self.field, &self.params.plants, dt);
        absorbed
    }

    /// Charges every live agent its upkeep and notes who ran out, of energy or of the
    /// health bites took. Step 9 of the tick (spec §2.4, §4.2).
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
                self.agents.muscle[i],
                self.agents.mouth[i],
                self.agents.brain_units[i],
                self.agents.sensor_load[i],
                self.intents.thrust[i] * self.agents.muscle[i],
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
            if energy::total(self.agents.energy[i], self.agents.energy_reserve[i]) <= 0.0
                || self.agents.health[i] <= 0.0
            {
                self.dying.push(id);
            }
        }
    }

    /// Regenerates living health and counts down bite cooldowns. Step 9 of the tick,
    /// after the charge that notes who a bite killed (spec §2.4, §4.2).
    pub fn recover_from_bites(&mut self) {
        let combat = &self.params.combat;
        let dt = self.params.world.dt;
        for id in self.pool.iter_live() {
            let i = id.index();
            combat::recover(
                &mut self.agents.health[i],
                &mut self.agents.cooldown[i],
                combat,
                dt,
            );
        }
    }

    /// Removes the agents that ran out of energy, or of the health bites took. Part of
    /// step 10 (spec §2.4).
    ///
    /// Agent-index order, because `iter_live` is ascending and that is what fills
    /// `dying`. Deaths resolved in any other order would hand the free list back in a
    /// different sequence and the next births would land in different slots.
    ///
    /// A starving agent holds no energy by the time it gets here — `charge_metabolism`
    /// took exactly what was left — so it leaves nothing. One a bite killed still holds
    /// what it had: `despawn` leaves its corpse share and dissipates the rest, which is
    /// what makes every route to removal safe (spec §4.2, §5.1).
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
                self.agents.energy[id.index()] <= 0.0 || self.agents.health[id.index()] <= 0.0,
                "a dying agent ran out of neither energy nor health"
            );
            if self.despawn_with_history_observer(id, &mut on_species, &mut on_history) {
                removed += 1;
            }
        }
        self.dying = dying;
        self.dying.clear();
        removed
    }

    /// Resolves this tick's swings before anyone eats. The first half of step 7 (spec
    /// §2.4, §4.2).
    ///
    /// Two passes. The first fixes who swings, from drives, cooldowns, and energy at the
    /// start of the step, charges every swing its cost, and finds each target, so an
    /// earlier biter's mouthful cannot drain a later one below the cost and cancel its
    /// swing. The second applies hits in agent-index order. Damage accumulates, so order
    /// cannot decide who dies; a victim holding less than every biter asks serves
    /// earlier slots first, as food does.
    pub fn resolve_bites(&mut self) {
        let combat = &self.params.combat;
        let cooldown = combat::cooldown_ticks(combat, self.params.world.dt);
        self.swings.clear();
        for id in self.pool.iter_live() {
            let i = id.index();
            if !combat::swings(
                self.intents.bite[i],
                self.agents.cooldown[i],
                self.agents.energy[i],
                self.agents.energy_reserve[i],
                combat,
            ) {
                continue;
            }
            // Paid whether or not the swing connects.
            let paid = energy::take_amount(
                &mut self.agents.energy[i],
                &mut self.agents.energy_reserve[i],
                combat.attack_cost as f64,
            );
            self.ledger.record_dissipated_amount(paid);
            self.agents.cooldown[i] = cooldown;
            self.swings.push(Swing {
                biter: id,
                target: AgentId::NULL,
                damage: 0.0,
            });
        }
        if self.swings.is_empty() {
            return;
        }

        // Step 1 placed everyone before step 5 moved them; a swing aims at where they
        // are. Only on ticks with a swing, so a world without bites pays nothing.
        self.hash.rebuild(
            &self.agents.position,
            self.pool.alive_flags(),
            &mut self.agents.grid_cell,
        );
        let targets = combat::Targets {
            positions: &self.agents.position,
            sizes: &self.agents.size,
            hash: &self.hash,
            largest: self.params.body.size_range[1],
            world_size: self.params.world.size,
            arc: combat.arc,
        };
        for swing in &mut self.swings {
            let i = swing.biter.index();
            let aim = combat::Aim {
                position: self.agents.position[i],
                heading: math::yaw_of(self.agents.orientation[i])
                    + math::reduce_angle(self.intents.bite_azimuth[i]),
                reach: self.intents.bite_reach[i],
                radius: self.agents.size[i],
            };
            if let Some(victim) = combat::target(i, &aim, &targets) {
                swing.target = AgentId::from(victim);
            }
        }

        let reference = self.params.body.size;
        let agents = &mut self.agents;
        // Mouthfuls in agent-index order: a victim holding less than every biter asks
        // serves earlier slots first, as food does (spec §4.2).
        for swing in &mut self.swings {
            if swing.target.is_null() {
                continue;
            }
            let (i, j) = (swing.biter.index(), swing.target.index());
            let gape = agents.mouth[i] * (agents.size[i] / reference);
            swing.damage = combat::damage(gape, agents.size[j] / reference, combat);
            let (mut biter_energy, mut biter_reserve) =
                (agents.energy[i], agents.energy_reserve[i]);
            let (mut victim_energy, mut victim_reserve) =
                (agents.energy[j], agents.energy_reserve[j]);
            let dissipated = combat::take_mouthful(
                combat::Hit {
                    biter_energy: &mut biter_energy,
                    biter_reserve: &mut biter_reserve,
                    victim_energy: &mut victim_energy,
                    victim_reserve: &mut victim_reserve,
                    gape,
                },
                combat,
            );
            agents.energy[i] = biter_energy;
            agents.energy_reserve[i] = biter_reserve;
            agents.energy[j] = victim_energy;
            agents.energy_reserve[j] = victim_reserve;
            self.ledger.record_dissipated_amount(dissipated);
        }
        // Wounds once per victim, in an order no slot chooses.
        combat::wound(&mut self.swings, &mut agents.health);
    }

    /// Moves energy from plants and corpses into the agents eating them. Step 7 of the
    /// tick (spec §2.4).
    ///
    /// A transfer, not a flow: nothing is recorded in the ledger, because the same
    /// joules are still in the world afterwards. If this is ever written wrongly the
    /// conservation test says so without knowing that eating exists.
    ///
    /// Agent-index order, because two agents can reach the same food in one tick and
    /// it may not hold enough for both. Whoever is asked first gets what is there;
    /// resolving in any other order would make the outcome depend on pool layout.
    pub fn resolve_feeding(&mut self) {
        let feeding = self.params.feeding.clone();
        let plant_radius = self.params.plants.radius;
        let corpse_radius = self.params.corpses.radius;
        for id in self.pool.iter_live() {
            let i = id.index();
            if self.intents.ingest[i] <= feeding.gate {
                continue;
            }
            let body = self.agents.size[i] + feeding.reach;
            // Intake grows with bite area, the square of gape: mouth relative to the
            // body, times the body relative to the reference (spec §3.5).
            let gape = self.agents.mouth[i] * (self.agents.size[i] / self.params.body.size);
            feeding::ingest(
                self.agents.position[i],
                feeding.rate * gape * gape,
                &mut self.agents.energy[i],
                &mut self.agents.energy_reserve[i],
                &mut feeding::Larder {
                    plants: &mut self.plants,
                    plant_reach: body + plant_radius,
                    corpses: &mut self.corpses,
                    corpse_reach: body + corpse_radius,
                },
            );
        }
    }

    /// Decomposes every corpse into dissipation. Step 9 of the tick (spec §2.4, §5.1).
    pub fn decay_corpses(&mut self) {
        let dissipated = self
            .corpses
            .decay(&self.params.corpses, self.params.world.dt);
        self.ledger.record_dissipated(dissipated);
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
            if self.brain_inheritance == BrainInheritance::StructuralNull {
                // The donor replaces only the inherited topology; the body and the
                // shared genes' scalars stay the parent's (spec §7.8).
                let donor = control::pick_donor(&self.pool, parent, &mut self.rng);
                let parent_genes = self.genes.get(genome);
                if let Err(error) = control::donor_topology(
                    parent_genes,
                    self.genes.get(self.agents.genome[donor.index()]),
                    self.params.storage.max_genes,
                    &mut scratch,
                ) {
                    // Refused like any birth: the parent stays whole.
                    self.genome_scratch = scratch;
                    on_refusal(error);
                    continue;
                }
                control::parent_scalars(parent_genes, &mut scratch);
            } else {
                scratch.extend_from_slice(self.genes.get(genome));
            }
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
        // Fixed topology, so the genomes line up gene for gene: this is about the
        // scalar redraw, and a structural edit would misalign the comparison.
        let mut params = SimParams::default().without_structural_mutation();
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

    /// Founders on the given traits, at rest and facing one way, so after a tick they
    /// differ only by what each body did with the same request. Powers of two keep
    /// every expected ratio exact in `f32`.
    fn bodies(params: SimParams, traits: &[(f32, f32, f32)]) -> World {
        let mut world = World::new(17, params).expect("valid params");
        for i in 0..traits.len() {
            world
                .spawn_founder(Vec3::new(100.0 + 60.0 * i as f32, 100.0, 0.0))
                .expect("room");
        }
        let facing = world.agents().orientation[0];
        let reference = world.params().body.size;
        for (i, &(size, muscle, mouth)) in traits.iter().enumerate() {
            let agents = world.agents_mut();
            agents.size[i] = reference * size;
            agents.muscle[i] = muscle;
            agents.mouth[i] = mouth;
            agents.orientation[i] = facing;
            agents.velocity[i] = Vec3::ZERO;
        }
        world
    }

    #[test]
    fn mass_slows_a_large_body_and_muscle_drives_it() {
        // Acceleration is thrust × muscle / s² (spec §3.5). (relative size, muscle,
        // velocity as a multiple of the reference body's)
        let cases = [
            (1.0, 1.0, 1.0),
            (2.0, 1.0, 0.25),
            (1.0, 2.0, 2.0),
            (2.0, 4.0, 1.0),
        ];
        let mut params = SimParams::default();
        params.world.max_agents = cases.len() as u32;
        params.plants.max_plants = 0;
        let traits: Vec<_> = cases.iter().map(|&(s, m, _)| (s, m, 1.0)).collect();
        let mut world = bodies(params, &traits);
        for i in 0..cases.len() {
            world.intents_mut().thrust[i] = 3.0;
        }
        world.integrate_movement();
        let reference = world.agents().velocity[0];
        assert_ne!(reference, Vec3::ZERO, "nothing moved");
        for (i, &(_, _, factor)) in cases.iter().enumerate() {
            assert_eq!(world.agents().velocity[i], reference * factor, "body {i}");
        }
    }

    #[test]
    fn upkeep_charges_the_body_and_the_force_muscle_adds() {
        // `k_move` charges the force muscle produced, not the drive that asked for it,
        // and each trait pays `k · (trait² − 1)` at rest (spec §3.5, §5.2). Every value
        // is exact in f32, so the charges are too.
        let mut params = SimParams::default();
        params.world.max_agents = 3;
        params.plants.max_plants = 0;
        params.metabolism = crate::params::MetabolismParams {
            base: 0.0,
            k_size: 0.0,
            k_brain: 0.0,
            k_sensor: 0.0,
            k_move: 0.5,
            k_muscle: 0.25,
            k_mouth: 0.125,
        };
        let mut world = bodies(params, &[(1.0, 1.0, 1.0), (1.0, 2.0, 1.0), (1.0, 1.0, 2.0)]);
        for i in 0..3 {
            world.intents_mut().thrust[i] = 1.0;
        }
        let held = |world: &World, i: usize| {
            energy::total(world.agents().energy[i], world.agents().energy_reserve[i])
        };
        let before: Vec<f64> = (0..3).map(|i| held(&world, i)).collect();
        world.charge_metabolism();
        let paid: Vec<f64> = (0..3).map(|i| before[i] - held(&world, i)).collect();
        // 0.5 · 1²; 0.25 · 3 + 0.5 · 2²; 0.125 · 3 + 0.5 · 1².
        assert_eq!(paid, [0.5, 2.75, 0.875]);
    }

    #[test]
    fn intake_grows_with_the_square_of_gape() {
        // Gape is mouth × relative size and intake is `feeding.rate · gape²` (spec
        // §3.5): doubling either quadruples the bite, and halving the body undoes a
        // doubled mouth. (relative size, mouth, intake per tick)
        let cases = [
            (1.0, 1.0, 1.0),
            (1.0, 2.0, 4.0),
            (2.0, 1.0, 4.0),
            (0.5, 2.0, 1.0),
        ];
        let mut params = SimParams::default();
        params.world.max_agents = cases.len() as u32;
        params.plants.max_plants = 64;
        params.feeding.rate = 1.0;
        let traits: Vec<_> = cases.iter().map(|&(s, mouth, _)| (s, 1.0, mouth)).collect();
        let mut world = bodies(params, &traits);
        // Each agent sits on a different stocked plant, which is then nearer to it
        // than any other food.
        let stocked: Vec<usize> = (0..world.plants().position().len())
            .filter(|&p| world.plants().alive()[p] != 0 && world.plants().energy()[p] > 8.0)
            .take(cases.len())
            .collect();
        assert_eq!(stocked.len(), cases.len(), "too few stocked plants");
        for (i, &plant) in stocked.iter().enumerate() {
            world.agents_mut().position[i] = world.plants().position()[plant];
            world.intents_mut().ingest[i] = 1.0;
        }
        world.rebuild_spatial_hash();
        let held = |world: &World, i: usize| {
            energy::total(world.agents().energy[i], world.agents().energy_reserve[i])
        };
        let before: Vec<f64> = (0..cases.len()).map(|i| held(&world, i)).collect();
        world.resolve_feeding();
        for (i, &(_, _, intake)) in cases.iter().enumerate() {
            assert_eq!(held(&world, i) - before[i], intake, "body {i}");
        }
    }

    /// Founders placed at `at`, each facing `yaw` and holding `energy`.
    fn duel(params: SimParams, agents: &[(Vec3, f32, f32)]) -> World {
        let mut world = World::new(23, params).expect("valid params");
        for &(at, yaw, energy) in agents {
            let i = world.spawn_founder(at).expect("room").index();
            let a = world.agents_mut();
            a.orientation[i] = crate::math::yaw_quat(yaw);
            a.energy[i] = energy;
            a.energy_reserve[i] = 0.0;
        }
        world
    }

    /// Every living agent asks to bite straight ahead, as a bite effector would.
    fn ask_to_bite(world: &mut World, reach: f32) {
        let live: Vec<usize> = world.pool().iter_live().map(|id| id.index()).collect();
        let intents = world.intents_mut();
        for i in live {
            intents.bite[i] = 1.0;
            intents.bite_azimuth[i] = 0.0;
            intents.bite_reach[i] = reach;
        }
    }

    fn duel_params(agents: u32) -> SimParams {
        let mut params = SimParams::default();
        params.world.max_agents = agents;
        params.plants.max_plants = 0;
        params
    }

    #[test]
    fn an_imported_bite_turns_with_its_body_whatever_its_azimuth() {
        // An import may carry any finite azimuth. At 2^26 a quarter turn of yaw added to
        // it rounded away, so the bite kept aiming one way however its biter turned.
        // Targets are placed from the azimuth's own sine and cosine, turned by the yaw,
        // so the aim must keep the direction the gene encodes as well as turn.
        use core::f32::consts::FRAC_PI_2;
        let azimuth = 67_108_864.0f32;
        let (sin, cos) = (crate::math::sin(azimuth), crate::math::cos(azimuth));
        let toward = |yaw: f32| {
            let (sy, cy) = (crate::math::sin(yaw), crate::math::cos(yaw));
            Vec3::new(
                100.0 + 5.0 * (cos * cy - sin * sy),
                100.0 + 5.0 * (sin * cy + cos * sy),
                0.0,
            )
        };
        for (yaw, target, hit) in [
            (0.0, 0.0, true),
            (FRAC_PI_2, 0.0, false),
            (FRAC_PI_2, FRAC_PI_2, true),
        ] {
            let mut world = duel(
                duel_params(2),
                &[
                    (Vec3::new(100.0, 100.0, 0.0), yaw, 100.0),
                    (toward(target), 0.0, 100.0),
                ],
            );
            ask_to_bite(&mut world, 4.0);
            world.intents_mut().bite[1] = 0.0;
            world.intents_mut().bite_azimuth[0] = azimuth;
            world.resolve_bites();
            assert_eq!(
                world.agents().health[1] < 1.0,
                hit,
                "biter facing {yaw}, target placed for {target}"
            );
        }
    }

    #[test]
    fn index_order_decides_neither_who_swings_nor_who_dies() {
        // Two founders face each other, each able to kill the other in one hit. The
        // first's mouthful leaves the second short of a swing's cost, which must not
        // cancel the second's swing: both are fixed before either lands (spec §4.2).
        let mut params = duel_params(2);
        params.combat.attack_damage = 1.0;
        for (first, second) in [(50.0, 10.0), (10.0, 50.0)] {
            let mut world = duel(
                params.clone(),
                &[
                    (Vec3::new(100.0, 100.0, 0.0), 0.0, first),
                    (Vec3::new(104.0, 100.0, 0.0), core::f32::consts::PI, second),
                ],
            );
            ask_to_bite(&mut world, 4.0);
            world.resolve_bites();
            assert_eq!(&world.agents().cooldown[..2], &[30, 30], "both swung");
            assert_eq!(&world.agents().health[..2], &[0.0, 0.0], "both hits landed");
            world.charge_metabolism();
            assert_eq!(world.dying.len(), 2, "both die in step 10");
        }
    }

    #[test]
    fn a_held_drive_swings_once_per_cooldown_and_every_swing_pays() {
        // At 0.5 s and 60 ticks a second, a held drive swings every 30 ticks and pays
        // the cost to the ledger whether or not it hits (spec §4.2).
        let mut world = duel(
            duel_params(1),
            &[(Vec3::new(100.0, 100.0, 0.0), 0.0, 100.0)],
        );
        let mut swings = Vec::new();
        for tick in 0..90 {
            ask_to_bite(&mut world, 4.0);
            let before = world.ledger().dissipated();
            world.resolve_bites();
            let paid = world.ledger().dissipated() - before;
            if paid != 0.0 {
                assert_eq!(paid, 8.0);
                swings.push(tick);
            }
            world.recover_from_bites();
        }
        assert_eq!(swings, [0, 30, 60]);
        assert_eq!(world.agents().energy[0], 76.0);
        // Short of the cost, a ready drive does not swing.
        world.agents_mut().energy[0] = 7.5;
        world.agents_mut().cooldown[0] = 0;
        ask_to_bite(&mut world, 4.0);
        world.resolve_bites();
        assert_eq!(world.agents().energy[0], 7.5);
        assert_eq!(world.agents().cooldown[0], 0);
    }

    #[test]
    fn a_bitten_agent_regenerates_and_a_killed_one_dies_leaving_a_corpse() {
        // Health regenerates only above 0, so lethal damage holds until step 10; and the
        // victim still holds energy, so unlike the starved it leaves a corpse (§4.2).
        let mut world = duel(
            duel_params(2),
            &[
                (Vec3::new(100.0, 100.0, 0.0), 0.0, 100.0),
                (Vec3::new(200.0, 100.0, 0.0), 0.0, 100.0),
            ],
        );
        world.agents_mut().health[0] = 0.5;
        world.agents_mut().health[1] = 0.0;
        world.charge_metabolism();
        world.recover_from_bites();
        let dt = world.params().world.dt;
        assert_eq!(world.agents().health[0], 0.5 + 0.02 * dt);
        assert_eq!(world.agents().health[1], 0.0);
        assert_eq!(world.resolve_deaths(), 1);
        assert_eq!(world.population(), 1);
        assert_eq!(
            world.corpses().count(),
            1,
            "a killed agent leaves its share"
        );
    }

    #[test]
    fn dormant_founders_never_swing_until_their_bite_wakes() {
        // A founder's bite reads a neuron biased below the gate, with nothing wired to
        // it, so no founder swings (spec §4.2).
        let mut params = SimParams::default();
        params.world.max_agents = 32;
        params.plants.max_plants = 64;
        params.founder.bite = true;
        let mut world = World::new(5, params.clone()).unwrap();
        world.seed_founders(32);
        for _ in 0..300 {
            world.step();
            assert!(
                world.agents().cooldown.iter().all(|&ticks| ticks == 0),
                "a dormant founder swung"
            );
        }
        // Biased above the gate, the same bite swings at once.
        params.combat.dormant_bias = 3.0;
        let mut world = World::new(5, params).unwrap();
        world.seed_founders(32);
        world.step();
        assert!(world.agents().cooldown.iter().any(|&ticks| ticks > 0));
    }

    #[test]
    fn a_retuned_timestep_keeps_a_running_cooldown_s_time() {
        // A swing at 60 ticks a second leaves half a second of cooldown; halving the
        // timestep mid-cooldown must leave the same time, not the same tick count.
        let mut world = duel(
            duel_params(1),
            &[(Vec3::new(100.0, 100.0, 0.0), 0.0, 100.0)],
        );
        ask_to_bite(&mut world, 4.0);
        world.resolve_bites();
        world.recover_from_bites();
        assert_eq!(world.agents().cooldown[0], 29);
        let mut finer = world.params().clone();
        finer.world.dt = 1.0 / 120.0;
        world.set_params(finer).unwrap();
        assert_eq!(
            world.agents().cooldown[0],
            58,
            "29 sixtieths are 58 hundred-twentieths"
        );
        let mut ticks = 0;
        loop {
            ask_to_bite(&mut world, 4.0);
            let before = world.ledger().dissipated();
            world.resolve_bites();
            if world.ledger().dissipated() > before {
                break;
            }
            world.recover_from_bites();
            ticks += 1;
        }
        assert_eq!(
            ticks, 58,
            "the next swing came after the time that was left"
        );
    }

    #[test]
    fn a_timestep_too_short_to_count_a_running_cooldown_is_refused_whole() {
        // A half-second swing leaves 29 ticks running after the cooldown is retuned to a
        // tenth of a second. At 1e-10 s those 29 sixtieths need about 4.8 billion ticks
        // and a new tenth only a billion, so only the running cooldown refuses the
        // timestep, and the preflight and the retune agree.
        let mut world = duel(
            duel_params(1),
            &[(Vec3::new(100.0, 100.0, 0.0), 0.0, 100.0)],
        );
        ask_to_bite(&mut world, 4.0);
        world.resolve_bites();
        world.recover_from_bites();
        let mut quick = world.params().clone();
        quick.combat.cooldown_seconds = 0.1;
        // No timestep this fine can count regeneration either, so none.
        quick.combat.health_regen = 0.0;
        world.set_params(quick).unwrap();
        assert_eq!(
            world.agents().cooldown[0],
            29,
            "a running cooldown keeps its ticks"
        );
        let mut tiny = world.params().clone();
        tiny.world.dt = 1e-10;
        assert_eq!(tiny.validate(), Ok(()));
        let refusal = Err(crate::params::ParamError(
            "a timestep this short cannot count a bite cooldown in whole ticks",
        ));
        assert_eq!(world.check_retune(&tiny), refusal);
        assert_eq!(world.set_params(tiny), refusal);
        assert_eq!(world.params().world.dt, 1.0 / 60.0, "the params moved");
        assert_eq!(world.agents().cooldown[0], 29, "the cooldown moved");
    }

    #[test]
    fn a_scheduled_retune_is_refused_before_a_swing_could_make_it_fail() {
        // A shell checks a scheduled retune before its run, when no cooldown is running.
        // Any swing before the retune's tick starts a full half second, which needs 5
        // billion ticks at 1e-10 s, so the check must refuse what the retune would.
        let mut world = duel(
            duel_params(1),
            &[(Vec3::new(100.0, 100.0, 0.0), 0.0, 100.0)],
        );
        let mut tiny = world.params().clone();
        tiny.world.dt = 1e-10;
        tiny.combat.cooldown_seconds = 0.4;
        // No timestep this fine can count regeneration either, so none.
        tiny.combat.health_regen = 0.0;
        let mut fine = tiny.clone();
        fine.world.dt = 1e-9;
        assert_eq!((tiny.validate(), fine.validate()), (Ok(()), Ok(())));
        assert_eq!(world.agents().cooldown[0], 0, "nothing is running yet");
        let refusal = Err(crate::params::ParamError(
            "a timestep this short cannot count a bite cooldown in whole ticks",
        ));
        assert_eq!(world.check_retune(&tiny), refusal);
        assert_eq!(world.check_retune(&fine), Ok(()));
        ask_to_bite(&mut world, 4.0);
        world.resolve_bites();
        world.recover_from_bites();
        assert_eq!(world.agents().cooldown[0], 29);
        assert_eq!(
            world.set_params(tiny),
            refusal,
            "the check and the retune agree"
        );
        assert_eq!(
            world.set_params(fine),
            Ok(()),
            "a timestep that counts the longest cooldown still applies"
        );
    }

    #[test]
    fn the_attackers_slots_cannot_decide_whether_their_victim_dies() {
        // Three biters close on one victim from east, west, and north. Their mouths deal
        // 0.1, 0.2, and 0.7 of full health, which subtracted hit by hit in some slot
        // orders would leave the victim a sliver of health (spec §4.2).
        use core::f32::consts::{FRAC_PI_2, PI};
        let around = [
            (Vec3::new(104.0, 100.0, 0.0), PI),
            (Vec3::new(96.0, 100.0, 0.0), 0.0),
            (Vec3::new(100.0, 104.0, 0.0), -FRAC_PI_2),
        ];
        let orders = [
            [0.4, 0.8, 2.8],
            [0.4, 2.8, 0.8],
            [0.8, 0.4, 2.8],
            [0.8, 2.8, 0.4],
            [2.8, 0.4, 0.8],
            [2.8, 0.8, 0.4],
        ];
        for mouths in orders {
            let mut agents = vec![(Vec3::new(100.0, 100.0, 0.0), 0.0, 1_000.0)];
            agents.extend(around.map(|(at, yaw)| (at, yaw, 100.0)));
            let mut world = duel(duel_params(4), &agents);
            for (slot, mouth) in (1..).zip(mouths) {
                world.agents_mut().mouth[slot] = mouth;
            }
            ask_to_bite(&mut world, 4.0);
            world.intents_mut().bite[0] = 0.0;
            world.resolve_bites();
            assert_eq!(
                world.agents().health[0].to_bits(),
                0.0f32.to_bits(),
                "{mouths:?}"
            );
        }
    }
}
