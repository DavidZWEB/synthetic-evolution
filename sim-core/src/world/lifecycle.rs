//! Agent lifecycle transitions within a world.
//!
//! Owns spawn-time allocation/compilation, founder seeding, and removal accounting.
//! Reproduction eligibility, mutation, and tick ordering remain in their systems.
//!
//! Lifecycle entry-point regressions deliberately live in the parent `world.rs`
//! `tests` module. Add new entry-point cases there until that group is moved together,
//! rather than creating a second unit-test home here.

use glam::Vec3;

use super::{PARTS_PER_AGENT, World};
use crate::agents::{Handles, SpawnSpec};
use crate::arena::{AllocationFailure, Block, VariableArena};
use crate::brain;
use crate::effectors;
use crate::genome::{self, BodyTrait, Gene};
use crate::history::{Event as HistoryEvent, EventKind as HistoryKind, Parent as HistoryParent};
use crate::ids::{AgentId, BirthId, SpeciesId, issue_birth};
use crate::math;
use crate::perceive;
use crate::spawn::{self, ArenaKind, SpawnError};
use crate::species::{Departure, SpeciesEvent};

impl World {
    /// Claims a slot and its arena blocks, and compiles the genome into a runnable
    /// brain. Refusals identify invalid genomes, per-genome limits, or storage pressure.
    ///
    /// **The caller owns the energy accounting.** This hands the new agent
    /// `spec.energy` and tells the ledger nothing, because the two ways an agent comes
    /// into existence account for it differently: a founder's tank is energy entering
    /// the world and is recorded as input by [`Self::spawn_founder`], while an
    /// offspring's is taken from its parent and is a transfer that must *not* be
    /// recorded at all. Getting this wrong is invisible until the conservation test
    /// runs, which is exactly why that test exists (spec §5.1).
    pub fn spawn(&mut self, spec: &SpawnSpec, genes: &[Gene]) -> Result<AgentId, SpawnError> {
        self.spawn_with_species_observer(spec, genes, |_| {})
    }

    pub fn spawn_with_species_observer(
        &mut self,
        spec: &SpawnSpec,
        genes: &[Gene],
        on_species: impl FnMut(SpeciesEvent),
    ) -> Result<AgentId, SpawnError> {
        self.spawn_with_history_observer(spec, genes, on_species, |_, _| {})
    }

    pub fn spawn_with_history_observer(
        &mut self,
        spec: &SpawnSpec,
        genes: &[Gene],
        on_species: impl FnMut(SpeciesEvent),
        on_history: impl FnMut(HistoryEvent, Option<&[Gene]>),
    ) -> Result<AgentId, SpawnError> {
        spawn::validate_limits(genes, &self.params.storage)?;
        spawn::validate_sensor_parameters(genes, self.hash.cell_size(), self.field.channels())?;
        spawn::validate_body(genes, spec.size, &self.params.body)?;
        let id = self.spawn_validated(spec, genes, on_species, on_history)?;
        // Imported genomes may carry fresh IDs beyond this world's template. Keep
        // subsequent structural edits from reusing them (spec section 3.1).
        if let Some(last) = genes.iter().filter_map(Gene::innovation).max() {
            self.next_innovation = self.next_innovation.max(last.raw() + 1);
        }
        Ok(id)
    }

    /// Caller must establish genome coherence, sensor bounds, and every storage limit.
    ///
    /// Public spawns validate explicitly; founders are covered by construction
    /// validation. The birth mutation pipeline must preserve architecture and enforce
    /// limits atomically before using this path (spec sections 2.2a and 3.3).
    pub(crate) fn spawn_validated(
        &mut self,
        spec: &SpawnSpec,
        genes: &[Gene],
        mut on_species: impl FnMut(SpeciesEvent),
        mut on_history: impl FnMut(HistoryEvent, Option<&[Gene]>),
    ) -> Result<AgentId, SpawnError> {
        if self.pool.live_count() == self.pool.capacity() {
            return Err(SpawnError::PoolFull);
        }
        // Capture before allocation: a dead parent slot may be reused by this child.
        // Resolving it afterwards could create self-parentage (spec §3.4).
        let parent_is_alive = self.pool.is_alive(spec.parent_a);
        let parent_birth_a = if parent_is_alive {
            self.agents.birth_id[spec.parent_a.index()]
        } else {
            BirthId::NULL
        };
        let history_parent_a = if spec.parent_a.is_null() {
            HistoryParent::Absent
        } else if parent_is_alive {
            let species = SpeciesId::new(self.agents.species_id[spec.parent_a.index()]);
            HistoryParent::Observed {
                birth_id: parent_birth_a,
                species_id: (!species.is_null()).then_some(species),
            }
        } else {
            HistoryParent::Unavailable
        };
        let handles = self.claim_blocks(genes)?;
        // Reserve every arena before claiming an identity: a failed birth must not
        // advance a slot incarnation or alter future allocation order (spec section 2.2a).
        let Some(id) = self.pool.alloc() else {
            self.release_blocks(handles);
            return Err(SpawnError::PoolFull);
        };
        self.genes.get_mut(handles.genome).copy_from_slice(genes);
        // Compiled once, here, and never read from the genome again during a tick
        // — resolving an innovation id costs a binary search (`brain`, `perceive`).
        brain::compile(
            genes,
            self.brains.get_mut(handles.brain),
            self.synapses.get_mut(handles.synapses),
        );
        perceive::compile(genes, self.sensors.get_mut(handles.sensors));
        effectors::compile(genes, self.effectors.get_mut(handles.effectors));
        self.agents.init(id, spec, &handles);
        self.agents.birth_id[id.index()] = match issue_birth(&mut self.next_birth) {
            Some(birth) => birth,
            None => BirthId::NULL,
        };
        self.agents.parent_birth_a[id.index()] = parent_birth_a;
        // The intent buffer lives outside `Agents`, so it misses `init`'s guarantee that
        // nothing survives from the slot's previous tenant. Without this a newborn
        // claiming a recycled slot acts on a corpse's last request on its first tick —
        // harmless only while no step between births and the next step 4 reads an
        // intent, which is an ordering M8 is free to change.
        self.intents.clear_slot(id.index());
        // Derived from the genome rather than the caller, and cached because metabolism
        // charges for them every tick and they cannot change while the agent lives.
        self.agents.brain_units[id.index()] = genome::brain_complexity(genes);
        self.agents.sensor_load[id.index()] = genome::sensor_load(genes);
        self.agents.muscle[id.index()] =
            genome::body_trait(genes, BodyTrait::Muscle).unwrap_or(1.0);
        self.agents.mouth[id.index()] = genome::body_trait(genes, BodyTrait::Mouth).unwrap_or(1.0);
        // Classification follows ecological admission and cannot refuse it (spec §3.4).
        match self.classifier.classify(genes) {
            Ok(assignment) => {
                self.agents.species_id[id.index()] = assignment.species.raw();
                if assignment.created {
                    on_species(SpeciesEvent::Created(assignment.species));
                    // The classifier's own immutable copy, borrowed only for this call:
                    // a later lookup can miss species that die before a drain (§3.4).
                    let representative = self.classifier.representative(assignment.species);
                    debug_assert_eq!(representative, Some(genes));
                    on_history(
                        HistoryEvent {
                            tick: self.tick,
                            kind: HistoryKind::SpeciesOrigin {
                                species_id: assignment.species,
                                founder_birth_id: self.agents.birth_id[id.index()],
                                parent_a: history_parent_a,
                                parent_b: HistoryParent::Absent,
                            },
                        },
                        representative,
                    );
                }
            }
            Err(reason) => {
                self.unclassified += 1;
                on_species(SpeciesEvent::Unclassified(reason));
            }
        }
        Ok(id)
    }

    /// Claims one block from every per-agent arena, or none of them.
    ///
    /// Capacity or fragmentation can fail any claim, so unwind every preceding claim.
    fn claim_blocks(&mut self, genes: &[Gene]) -> Result<Handles, SpawnError> {
        fn claim<T: Copy + Default>(
            arena: &mut VariableArena<T>,
            count: u32,
            kind: ArenaKind,
        ) -> Result<Block, SpawnError> {
            arena.alloc(count).map_err(|reason| SpawnError::Arena {
                arena: kind,
                reason,
            })
        }
        let mut handles = Handles::default();
        let claimed = (|| {
            handles.brain = claim(
                &mut self.brains,
                genome::neuron_count(genes) as u32,
                ArenaKind::Neurons,
            )?;
            handles.synapses = claim(
                &mut self.synapses,
                brain::synapse_count(genes) as u32,
                ArenaKind::Synapses,
            )?;
            handles.sensors = claim(
                &mut self.sensors,
                perceive::sensor_count(genes) as u32,
                ArenaKind::Sensors,
            )?;
            handles.effectors = claim(
                &mut self.effectors,
                effectors::effector_count(genes) as u32,
                ArenaKind::Effectors,
            )?;
            handles.genome = claim(&mut self.genes, genes.len() as u32, ArenaKind::Genes)?;
            handles.parts = self.parts.alloc(PARTS_PER_AGENT).ok_or(SpawnError::Arena {
                arena: ArenaKind::Parts,
                reason: AllocationFailure::BlockLimit,
            })?;
            Ok(())
        })();
        if let Err(error) = claimed {
            self.release_blocks(handles);
            return Err(error);
        }
        Ok(handles)
    }

    fn release_blocks(&mut self, handles: Handles) {
        self.brains.free(handles.brain);
        self.synapses.free(handles.synapses);
        self.sensors.free(handles.sensors);
        self.effectors.free(handles.effectors);
        self.genes.free(handles.genome);
        self.parts.free(handles.parts);
    }

    /// Spawns a founder: the world's fixed topology with fresh random scalars.
    ///
    /// Body traits come out of the genome rather than the caller, because that is the
    /// point of carrying them genetically — an offspring inherits its parent's size
    /// and colour without anything else having to remember to copy them.
    pub fn spawn_founder(&mut self, position: Vec3) -> Result<AgentId, SpawnError> {
        self.spawn_founder_with_species_observer(position, |_| {})
    }

    pub fn spawn_founder_with_species_observer(
        &mut self,
        position: Vec3,
        on_species: impl FnMut(SpeciesEvent),
    ) -> Result<AgentId, SpawnError> {
        self.spawn_founder_with_history_observer(position, on_species, |_, _| {})
    }

    pub fn spawn_founder_with_history_observer(
        &mut self,
        position: Vec3,
        on_species: impl FnMut(SpeciesEvent),
        on_history: impl FnMut(HistoryEvent, Option<&[Gene]>),
    ) -> Result<AgentId, SpawnError> {
        // Taken out of `self` so the borrow checker sees the buffer and the world as
        // separate; put back before returning.
        let mut scratch = core::mem::take(&mut self.genome_scratch);
        scratch.resize(self.plan.len(), Gene::default());
        self.plan
            .instantiate(&mut self.rng, &self.params, &mut scratch);

        let yaw = self
            .rng
            .range(-core::f32::consts::PI, core::f32::consts::PI);
        let spec = SpawnSpec {
            position,
            yaw,
            energy: self.params.reproduction.start_energy,
            size: genome::body_trait(&scratch, BodyTrait::Size).unwrap_or(self.params.body.size),
            signature: Vec3::new(
                genome::body_trait(&scratch, BodyTrait::SignatureR).unwrap_or(0.5),
                genome::body_trait(&scratch, BodyTrait::SignatureG).unwrap_or(0.5),
                genome::body_trait(&scratch, BodyTrait::SignatureB).unwrap_or(0.5),
            ),
            parent_a: AgentId::NULL,
        };
        let spawned = self.spawn_validated(&spec, &scratch, on_species, on_history);
        self.genome_scratch = scratch;
        if spawned.is_ok() {
            // A founder's tank is the one energy source that is not a plant. It is a
            // boundary condition — the experimenter seeding a world — not an ongoing
            // leak, and recording it as input is what keeps §5.1's books balanced
            // without pretending the agent arrived empty. An *offspring* is different:
            // its energy comes out of its parent, so reproduction records nothing.
            self.ledger.record_input(spec.energy as f64);
        }
        spawned
    }

    /// Returns a slot and its arena blocks. Despawning a dead agent is a no-op, so a
    /// double death cannot free the same block twice.
    ///
    /// **Death is the only thing that calls this in the simulation today** — one call
    /// site, in [`Self::resolve_deaths`], for an agent that starved. It is public
    /// because removal has to be reachable from outside the tick: `no_alloc` exercises
    /// it directly, and the `Command` enum at M8 is the route a client will use to take
    /// an agent out of a running world.
    ///
    /// **Whatever the agent still holds is dissipated here**, so removing one can never
    /// delete energy the world was accounting for (spec §5.1). A starved agent was
    /// already drained to zero and dissipates nothing, so this costs the normal path
    /// nothing; it is what makes every *other* route to removal safe, including the
    /// tests that despawn a living agent today and the Phase 3 corpse that will
    /// transfer part of one before removing it.
    ///
    /// Unlike [`Self::spawn`], this can own its accounting: creation genuinely differs
    /// between a founder and an offspring, while removal has one correct rule.
    pub fn despawn(&mut self, id: AgentId) -> bool {
        self.despawn_with_species_observer(id, |_| {})
    }

    pub fn despawn_with_species_observer(
        &mut self,
        id: AgentId,
        on_species: impl FnMut(SpeciesEvent),
    ) -> bool {
        self.despawn_with_history_observer(id, on_species, |_, _| {})
    }

    pub fn despawn_with_history_observer(
        &mut self,
        id: AgentId,
        mut on_species: impl FnMut(SpeciesEvent),
        mut on_history: impl FnMut(HistoryEvent, Option<&[Gene]>),
    ) -> bool {
        if !self.pool.is_alive(id) {
            return false;
        }
        let i = id.index();
        let species = SpeciesId::new(self.agents.species_id[i]);
        let extinct = if species.is_null() {
            debug_assert!(
                self.unclassified > 0,
                "unclassified membership must be counted"
            );
            self.unclassified -= 1;
            false
        } else {
            let departure = self.classifier.remove_member(species);
            debug_assert!(
                departure.is_ok(),
                "live species must have counted membership"
            );
            departure == Ok(Departure::Extinct)
        };
        // A share of what the agent held lies where it fell; the rest leaves the world
        // (spec §5.1). A starved agent holds nothing, so it leaves nothing.
        self.corpses.leave(
            self.agents.position[i],
            &mut self.agents.energy[i],
            &mut self.agents.energy_reserve[i],
            &self.params.corpses,
        );
        let remaining = crate::energy::take_amount(
            &mut self.agents.energy[i],
            &mut self.agents.energy_reserve[i],
            f64::MAX,
        );
        self.ledger.record_dissipated_amount(remaining);
        self.brains.free(self.agents.brain[i]);
        self.synapses.free(self.agents.synapses[i]);
        self.sensors.free(self.agents.sensors[i]);
        self.effectors.free(self.agents.effectors[i]);
        self.genes.free(self.agents.genome[i]);
        self.parts.free(self.agents.parts[i]);
        self.agents.clear(id);
        let removed = self.pool.free(id);
        if extinct {
            on_species(SpeciesEvent::Extinct(species));
            on_history(
                HistoryEvent {
                    tick: self.tick,
                    kind: HistoryKind::SpeciesExtinct {
                        species_id: species,
                    },
                },
                None,
            );
        }
        removed
    }

    /// Places `count` founders on a golden-angle spiral around the centre, and reports
    /// how many the pool had room for.
    ///
    /// Where generation 0 starts is a property of the simulation, not of whichever shell
    /// asked for it: the layout is folded into every seeded run, so two arrangements of
    /// the same count are different experiments from the same seed. One definition here
    /// means a headless sweep and the browser are running the same one.
    ///
    /// The **golden angle** avoids rings and spokes, while square-root radial spacing
    /// gives equal-area density across the disc. Linear radial spacing would pack half
    /// the founders into the inner quarter of its area, handing generation 0 a dense
    /// central niche that nothing in the ecology created. Offspring still inherit only
    /// their local neighbourhood through spatial viscosity (spec §5.4).
    pub fn seed_founders(&mut self, count: u32) -> u32 {
        self.seed_founders_with_observer(count, |_| {})
    }

    pub fn seed_founders_with_observer(
        &mut self,
        count: u32,
        mut on_refusal: impl FnMut(SpawnError),
    ) -> u32 {
        self.seed_founders_with_observers(count, &mut on_refusal, |_| {})
    }

    pub fn seed_founders_with_observers(
        &mut self,
        count: u32,
        mut on_refusal: impl FnMut(SpawnError),
        mut on_species: impl FnMut(SpeciesEvent),
    ) -> u32 {
        self.seed_founders_with_history_observer(count, &mut on_refusal, &mut on_species, |_, _| {})
    }

    pub fn seed_founders_with_history_observer(
        &mut self,
        count: u32,
        mut on_refusal: impl FnMut(SpawnError),
        mut on_species: impl FnMut(SpeciesEvent),
        mut on_history: impl FnMut(HistoryEvent, Option<&[Gene]>),
    ) -> u32 {
        /// Radians. The irrational turn that makes a phyllotactic spiral, and the reason
        /// sunflower seeds pack without lining up.
        const GOLDEN_ANGLE: f32 = 2.399_963_2;

        let size = self.params.world.size;
        let spread = self.params.world.founder_spread;
        // A shell can request more founders than the fixed pool holds. Stop at the
        // boundary rather than spending billions of iterations constructing genomes
        // that allocation is guaranteed to reject.
        let available = self.pool.capacity().saturating_sub(self.population());
        let count = count.min(available);
        let mut placed = 0;
        for i in 0..count {
            let angle = i as f32 * GOLDEN_ANGLE;
            let area_fraction = (i as f32 + 0.5) / count as f32;
            let r = size * spread * math::sqrt(area_fraction);
            let position = Vec3::new(
                size * 0.5 + r * math::cos(angle),
                size * 0.5 + r * math::sin(angle),
                0.0,
            );
            match self.spawn_founder_with_history_observer(
                position,
                &mut on_species,
                &mut on_history,
            ) {
                Ok(_) => placed += 1,
                Err(error) => {
                    on_refusal(error);
                    break;
                }
            }
        }
        placed
    }
}
