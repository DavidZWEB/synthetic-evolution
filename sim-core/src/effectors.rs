//! Effectors: turning brain outputs into requests, and nothing else.
//!
//! **Nothing here changes the world.** An effector reads one neuron's activation and
//! writes a number into the intent buffer; the systems that come after — movement,
//! interaction, births — are what act on it. That separation is what makes step 4 of
//! the tick meaningful: every agent decides against the same world, and an agent that
//! happens to sit early in the pool cannot move before a later one has chosen
//! (spec §2.4, CLAUDE.md).
//!
//! Effectors bind to a neuron **by innovation id**, resolved to a slot once at birth,
//! exactly as sensors and connections are. [`Effector`] is the compiled form.
//!
//! Deliberately not here: what any intent means. Movement drains `thrust` and `turn`;
//! `ingest` and `reproduce` wait for the plants and the economy at M7. Writing them
//! now is not pre-building — the brain already has the output neurons, and an effector
//! whose request nothing reads is how the wiring gets tested before the consequence
//! exists.

use serde::{Deserialize, Serialize};

use crate::brain::Neuron;
use crate::genome::{Action, GENE_PARAMS, Gene};
use crate::ids::NeuronId;
use crate::params::MovementParams;

/// One effector of a compiled agent: what it does, its parameters, and the brain slot
/// that drives it.
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Effector {
    pub action: Action,
    /// `[axis x, axis y, axis z, _]` for a turn, unused by the rest.
    ///
    /// The axis is pinned to Z and nothing varies it, the same treatment sensor
    /// elevation gets. V1's integrator uses the yaw-only specialization in
    /// `math::rotate_yaw` and never reads these, but they occupy their slot in the
    /// serialized genome so a body that can roll and pitch is an unclamping rather than
    /// a format break (build-plan checklist, spec §9.1).
    pub params: [f32; GENE_PARAMS],
    pub source: NeuronId,
}

/// What every agent's effectors asked for this tick.
///
/// Struct-of-arrays indexed by pool slot, not a queue of events. Phase 1's four
/// effectors are all self-directed — they move, feed, or reproduce *this* agent — so
/// one slot each is the whole story and agent-index order is free rather than something
/// a sort has to restore. An interaction that names another agent (`bite`, `grab` in
/// Phase 3 and 6) is the case that will need a real queue, and it can have one then.
#[derive(Clone, Debug)]
pub struct Intents {
    /// Forward force, already scaled by [`MovementParams::max_thrust`].
    pub thrust: Vec<f32>,
    /// Signed angular velocity in radians per second. Positive turns left.
    pub turn: Vec<f32>,
    /// Raw drive, not a decision. The threshold belongs to whatever resolves ingestion
    /// against real food, which is M7 — an effector's job is to report what the brain
    /// asked for, not to decide whether it gets it.
    pub ingest: Vec<f32>,
    /// Raw drive, gated by [`ReproductionParams::gate`] where births are resolved at
    /// step 10 (spec §2.4).
    ///
    /// [`ReproductionParams::gate`]: crate::params::ReproductionParams::gate
    pub reproduce: Vec<f32>,
}

impl Intents {
    pub fn with_capacity(capacity: u32) -> Self {
        let n = capacity as usize;
        Self {
            thrust: vec![0.0; n],
            turn: vec![0.0; n],
            ingest: vec![0.0; n],
            reproduce: vec![0.0; n],
        }
    }

    /// Drops every request. Called at the top of step 4, so an agent that has lost an
    /// effector stops moving rather than repeating whatever it last asked for — and so
    /// a recycled slot never inherits the dead tenant's last wish.
    pub fn clear(&mut self) {
        self.thrust.fill(0.0);
        self.turn.fill(0.0);
        self.ingest.fill(0.0);
        self.reproduce.fill(0.0);
    }

    pub fn capacity(&self) -> u32 {
        self.thrust.len() as u32
    }
}

/// Compiles the effector genes of `genes` into `out`, resolving each source to a slot.
pub fn compile(genes: &[Gene], out: &mut [Effector]) {
    debug_assert_eq!(
        out.len(),
        effector_count(genes),
        "effector block is the wrong size"
    );
    for (slot, effector) in out.iter_mut().zip(compiled_effectors(genes)) {
        *slot = effector;
    }
}

/// The effector genes that compile to a working organ: the source neuron resolves.
fn compiled_effectors(genes: &[Gene]) -> impl Iterator<Item = Effector> + '_ {
    genes.iter().filter_map(move |gene| {
        let Gene::Effector(e) = gene else { return None };
        Some(Effector {
            action: e.action,
            params: e.params,
            source: crate::brain::slot_of(genes, e.source)?,
        })
    })
}

/// How many effectors `genes` compiles to. Sizes the arena block before `compile` fills
/// it.
pub fn effector_count(genes: &[Gene]) -> usize {
    compiled_effectors(genes).count()
}

/// Reads one agent's effectors and records what they ask for. Step 4 of the tick.
///
/// Two output neurons driving the same action sum, for the same reason two sensor
/// channels sharing an input neuron sum: the alternative is one of them silently
/// winning based on gene order.
pub fn drive(
    effectors: &[Effector],
    neurons: &[Neuron],
    params: &MovementParams,
    intents: &mut AgentIntents<'_>,
) {
    for effector in effectors {
        // A source that did not resolve is not compiled, so this only guards a slot the
        // arena zeroed and the block length should already exclude.
        let Some(neuron) = neurons.get(effector.source.index()) else {
            continue;
        };
        let output = neuron.output;
        match effector.action {
            // Forward force only. Spec §4.2 says "forward force", and an agent that can
            // reverse has less reason to evolve a turn — steering is the behaviour this
            // phase is trying to get.
            Action::Thrust => *intents.thrust += output * params.max_thrust,
            // Signed, unlike thrust, because an angular velocity that can only go one
            // way is a permanent circle rather than a control surface. A sigmoid sits
            // at 0.5 when its input is nothing, so 0.5 is straight ahead.
            Action::Turn => *intents.turn += (output - 0.5) * 2.0 * params.max_turn_rate,
            Action::Ingest => *intents.ingest += output,
            Action::Reproduce => *intents.reproduce += output,
        }
    }
}

/// One agent's slot in each intent array, so `drive` can write without knowing how the
/// buffer is laid out.
pub struct AgentIntents<'a> {
    pub thrust: &'a mut f32,
    pub turn: &'a mut f32,
    pub ingest: &'a mut f32,
    pub reproduce: &'a mut f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain;
    use crate::genome::{Activation, EffectorGene, NeuronGene, validate};
    use crate::ids::InnovationId;
    use crate::params::SimParams;

    /// A genome of one neuron per action, each driving its own effector.
    fn one_each(actions: &[Action]) -> Vec<Gene> {
        let mut genes: Vec<Gene> = (0..actions.len())
            .map(|i| {
                Gene::Neuron(NeuronGene {
                    id: InnovationId::new(i as u32),
                    bias: 0.0,
                    tau: 1.0,
                    activation: Activation::Sigmoid,
                    period: 0.0,
                })
            })
            .collect();
        for (i, &action) in actions.iter().enumerate() {
            genes.push(Gene::Effector(EffectorGene {
                id: InnovationId::new(100 + i as u32),
                action,
                params: [0.0, 0.0, 1.0, 0.0],
                source: InnovationId::new(i as u32),
            }));
        }
        genes.sort_by_key(Gene::sort_key);
        assert_eq!(validate(&genes), Ok(()), "fixture is incoherent");
        genes
    }

    /// Runs `drive` over a genome whose neurons hold `outputs`.
    fn ask(actions: &[Action], outputs: &[f32]) -> (f32, f32, f32, f32) {
        let genes = one_each(actions);
        let mut compiled = vec![Effector::default(); effector_count(&genes)];
        compile(&genes, &mut compiled);

        let mut neurons = vec![Neuron::default(); crate::genome::neuron_count(&genes)];
        let mut synapses = vec![brain::Synapse::default(); brain::synapse_count(&genes)];
        brain::compile(&genes, &mut neurons, &mut synapses);
        for (neuron, &output) in neurons.iter_mut().zip(outputs.iter()) {
            neuron.output = output;
        }

        let (mut thrust, mut turn, mut ingest, mut reproduce) = (0.0, 0.0, 0.0, 0.0);
        drive(
            &compiled,
            &neurons,
            &SimParams::default().movement,
            &mut AgentIntents {
                thrust: &mut thrust,
                turn: &mut turn,
                ingest: &mut ingest,
                reproduce: &mut reproduce,
            },
        );
        (thrust, turn, ingest, reproduce)
    }

    #[test]
    fn thrust_is_forward_only_and_scales_with_its_neuron() {
        let params = SimParams::default().movement;
        let (idle, ..) = ask(&[Action::Thrust], &[0.0]);
        let (half, ..) = ask(&[Action::Thrust], &[0.5]);
        let (full, ..) = ask(&[Action::Thrust], &[1.0]);
        assert_eq!(idle, 0.0);
        assert!((half - params.max_thrust * 0.5).abs() < 1e-6);
        assert!((full - params.max_thrust).abs() < 1e-6);
        assert!(idle >= 0.0 && half >= 0.0, "thrust must never be negative");
    }

    #[test]
    fn turn_is_signed_around_a_neutral_half() {
        // A sigmoid with nothing to say sits at 0.5, so 0.5 has to mean straight on.
        // An unsigned turn would make every agent circle forever.
        let params = SimParams::default().movement;
        let (_, left, ..) = ask(&[Action::Turn], &[1.0]);
        let (_, straight, ..) = ask(&[Action::Turn], &[0.5]);
        let (_, right, ..) = ask(&[Action::Turn], &[0.0]);
        assert!((left - params.max_turn_rate).abs() < 1e-6);
        assert_eq!(straight, 0.0);
        assert!((right + params.max_turn_rate).abs() < 1e-6);
    }

    #[test]
    fn ingest_and_reproduce_pass_the_raw_drive_through() {
        // No threshold here: the systems that resolve them own that, and baking a gate
        // into the effector would put the decision two milestones from its consequence.
        let (.., ingest, reproduce) = ask(&[Action::Ingest, Action::Reproduce], &[0.3, 0.9]);
        assert!((ingest - 0.3).abs() < 1e-6);
        assert!((reproduce - 0.9).abs() < 1e-6);
    }

    #[test]
    fn two_effectors_on_one_action_sum() {
        // Otherwise gene order silently decides which one wins.
        let params = SimParams::default().movement;
        let (thrust, ..) = ask(&[Action::Thrust, Action::Thrust], &[0.5, 0.25]);
        assert!((thrust - params.max_thrust * 0.75).abs() < 1e-6);
    }

    #[test]
    fn an_effector_whose_neuron_vanished_is_not_compiled() {
        let mut genes = one_each(&[Action::Thrust]);
        for gene in genes.iter_mut() {
            if let Gene::Effector(e) = gene {
                e.source = InnovationId::new(999);
            }
        }
        assert_eq!(effector_count(&genes), 0, "a dangling effector compiled");
    }

    #[test]
    fn clearing_drops_every_request() {
        // Step 4 clears before it writes, so an agent that lost its thrust effector
        // coasts to a stop rather than repeating its last wish forever — and a recycled
        // slot never inherits the dead tenant's.
        let mut intents = Intents::with_capacity(4);
        intents.thrust[2] = 5.0;
        intents.reproduce[0] = 1.0;
        intents.clear();
        assert!(intents.thrust.iter().all(|&t| t == 0.0));
        assert!(intents.reproduce.iter().all(|&r| r == 0.0));
    }

    #[test]
    fn the_turn_axis_is_carried_and_pinned_to_z() {
        // A forward-compatibility hedge, not a live parameter: V1 rotates about Z with
        // the yaw specialization and never reads this. It exists so a body that can
        // roll is an unclamping rather than a format break (spec §9.1).
        let genes = one_each(&[Action::Turn]);
        let mut compiled = vec![Effector::default(); effector_count(&genes)];
        compile(&genes, &mut compiled);
        assert_eq!(compiled[0].params[0..3], [0.0, 0.0, 1.0]);
    }
}
