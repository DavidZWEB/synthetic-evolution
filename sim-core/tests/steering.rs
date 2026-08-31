//! M6's acceptance check: an agent with hand-written weights climbs a food gradient.
//!
//! **This is a wiring test, not an evolution test.** The weights are written by hand,
//! which is what the milestone asks for and what makes the claim narrow: it says the
//! sensorimotor chain closes correctly — perceive → think → act → move, with every sign
//! convention agreeing — and says nothing about whether food-seeking *evolves*. That is
//! spec §8's Phase 1 criterion, it needs plants and selection, and CLAUDE.md is explicit
//! that a human has to watch it rather than a test asserting it (M12).
//!
//! Worth being blunt about why it is here anyway. Four systems land at M6 and compose
//! for the first time. A gradient rotated into the wrong frame, a turn signed the wrong
//! way, or an intent buffer read a tick late all produce a simulation that runs, looks
//! plausible, and fails to evolve anything — and every one of them is far cheaper to
//! find now than tangled up with plants, metabolism, and the energy ledger at M7.
//!
//! Two things keep it from being a strawman. The field diffuses and decays every tick
//! while the agent moves, rather than being a frozen blob nothing like what plants will
//! produce. And the agent has to succeed from eight starting headings, so it cannot pass
//! on one lucky trajectory — including the heading pointing directly away from the food.
//!
//! What is measured is **closest approach**, not where the agent finished. Thrust is held
//! on and the only brake is drag, so a pursuit controller sails past a point source and
//! takes a wide turn to come back; that is a property of a one-rule controller, not of
//! the wiring, and station-keeping is not a behaviour the real world will ever need —
//! at M7 an agent that reaches a plant eats it, and there is nothing left to orbit.

use glam::Vec3;
use sim_core::agents::SpawnSpec;
use sim_core::genome::{
    Action, Activation, ConnectionGene, EffectorGene, GENE_PARAMS, Gene, Modality, NeuronGene,
    SENSOR_CHANNELS, SensorGene, validate,
};
use sim_core::ids::{AgentId, InnovationId};
use sim_core::params::{ChemoParams, SimParams};
use sim_core::world::World;

/// Neuron slots, in the order they sort. The chemo sensor writes the first three.
const STRENGTH: u32 = 0;
const UPHILL_AHEAD: u32 = 1;
const UPHILL_LEFT: u32 = 2;
const TURN: u32 = 3;
const THRUST: u32 = 4;

/// A genome wired by hand to steer up a chemical gradient.
///
/// The whole strategy is one connection: the nose's "uphill is to my left" channel
/// drives the turn neuron. Thrust is held on by a bias, so the agent always moves and
/// only its heading is under control — the simplest arrangement that can climb, and the
/// one that fails loudly if any sign in the chain is inverted.
///
/// `TURN`'s bias cancels the connection's midpoint. A sigmoid input neuron sits at 0.5
/// when it senses nothing, so without the offset the turn neuron would sit well above
/// 0.5 and the agent would circle forever regardless of what it smelled.
fn steering_genome(turn_gain: f32) -> Vec<Gene> {
    let neuron = |id: u32, bias: f32, tau: f32| {
        Gene::Neuron(NeuronGene {
            id: InnovationId::new(id),
            bias,
            tau,
            activation: Activation::Sigmoid,
            period: 0.0,
        })
    };
    // Fast enough to track its input within a tick or two; steering that lags by ten
    // ticks is steering at where the food used to be.
    let quick = 1.0 / 30.0;

    let mut targets = [InnovationId::NULL; SENSOR_CHANNELS];
    targets[0] = InnovationId::new(STRENGTH);
    targets[1] = InnovationId::new(UPHILL_AHEAD);
    targets[2] = InnovationId::new(UPHILL_LEFT);

    let mut genes = vec![
        neuron(STRENGTH, 0.0, quick),
        neuron(UPHILL_AHEAD, 0.0, quick),
        neuron(UPHILL_LEFT, 0.0, quick),
        // Bias cancels `turn_gain * 0.5`, so a nose that senses nothing steers straight.
        neuron(TURN, -turn_gain * 0.5, quick),
        // Held on: sigmoid(2.5) is about 0.92 of full thrust.
        neuron(THRUST, 2.5, quick),
        Gene::Sensor(SensorGene {
            id: InnovationId::new(10),
            modality: Modality::Chemo,
            params: [0.0, 40.0, 0.0, 0.0],
            targets,
        }),
        Gene::Effector(EffectorGene {
            id: InnovationId::new(20),
            action: Action::Turn,
            // Rotation axis, pinned to Z (spec §9.1).
            params: [0.0, 0.0, 1.0, 0.0],
            source: InnovationId::new(TURN),
        }),
        Gene::Effector(EffectorGene {
            id: InnovationId::new(21),
            action: Action::Thrust,
            params: [0.0; GENE_PARAMS],
            source: InnovationId::new(THRUST),
        }),
        Gene::Connection(ConnectionGene {
            id: InnovationId::new(30),
            from: InnovationId::new(UPHILL_LEFT),
            to: InnovationId::new(TURN),
            weight: turn_gain,
            enabled: true,
        }),
    ];
    genes.sort_by_key(Gene::sort_key);
    assert_eq!(validate(&genes), Ok(()), "hand-built genome is incoherent");
    genes
}

fn params() -> SimParams {
    SimParams {
        chemo: ChemoParams {
            // Coarser than the default 128². Seventeen runs of four thousand ticks each
            // spend nearly all their time diffusing cells, and at a 40-unit sensing
            // radius a 64² grid resolves the same gradient — this is the one knob here
            // that costs minutes of everybody's `cargo test` and buys nothing.
            cells: [64, 64, 1],
            // A slow leak rather than none, so the field is one a running world could
            // hold: it is being replenished and lost at the same time, not frozen.
            decay: vec![0.999],
            diffuse: 0.5,
        },
        ..SimParams::default()
    }
}

/// The minimum-image distance between two points on the torus.
fn separation(a: Vec3, b: Vec3, size: f32) -> f32 {
    let d = sim_core::spatial::min_image(a - b, size);
    d.length()
}

/// Runs one agent for `ticks`, and reports how far it started from the food and the
/// closest it ever got.
///
/// The tick order is spec §2.4's, minus the steps that do not exist yet: rebuild the
/// hash, perceive, think, decide, move, then update the field.
fn approach(start_yaw: f32, ticks: usize, turn_gain: f32) -> (f32, f32) {
    let params = params();
    let size = params.world.size;
    let food = Vec3::new(size * 0.5, size * 0.5, 0.0);
    let mut world = World::new(1, params.clone()).expect("valid params");

    // A source with a head start, so there is a slope to read from the first tick.
    for _ in 0..400 {
        world.deposit_chemo(0, food, 200.0);
        world.update_chemo();
    }

    let start = Vec3::new(size * 0.5 - 220.0, size * 0.5, 0.0);
    let spec = SpawnSpec {
        position: start,
        yaw: start_yaw,
        energy: params.reproduction.start_energy,
        size: params.body.size,
        signature: Vec3::new(0.5, 0.5, 0.5),
        parent_a: AgentId::NULL,
    };
    let id = world
        .spawn(&spec, &steering_genome(turn_gain))
        .expect("an empty world has room");

    let start_distance = separation(world.agents().position[id.index()], food, size);
    let mut closest = start_distance;
    for _ in 0..ticks {
        world.rebuild_spatial_hash();
        world.perceive_all();
        world.step_brains();
        world.drive_effectors();
        world.integrate_movement();
        // The plants that would keep the source topped up arrive at M7; standing in for
        // them keeps the field from decaying out from under the test.
        world.deposit_chemo(0, food, 200.0);
        world.update_chemo();
        closest = closest.min(separation(world.agents().position[id.index()], food, size));
    }
    (start_distance, closest)
}

/// Within this many units an agent is on top of the food — a few body radii, and well
/// inside the range an `ingest` effector will reach at M7.
const ARRIVED: f32 = 20.0;

/// Every heading a run is tried from, including the one pointing straight away.
fn headings() -> impl Iterator<Item = f32> {
    (0..8).map(|step| step as f32 * core::f32::consts::TAU / 8.0)
}

#[test]
fn a_hand_wired_agent_climbs_a_food_gradient() {
    // Started pointing directly away from the food, which is the demanding case: the
    // agent has to read the gradient behind it and turn around, not merely hold a
    // heading it was handed. A minute of sim time, from 220 units out.
    let (start, closest) = approach(core::f32::consts::PI, 3_600, 6.0);
    assert!(
        closest < ARRIVED,
        "started {start:.1} away pointing in the opposite direction and never got \
         closer than {closest:.1}"
    );
}

#[test]
fn it_arrives_from_any_starting_heading() {
    // One lucky trajectory is not evidence the loop is wired correctly. A sign inverted
    // anywhere in the chain — the gradient rotated into the wrong frame, the turn signed
    // backwards — would show up as arriving from some headings and not others.
    let missed: Vec<String> = headings()
        .filter_map(|yaw| {
            let (start, closest) = approach(yaw, 3_600, 6.0);
            (closest >= ARRIVED).then(|| format!("yaw {yaw:.2}: {start:.1} -> {closest:.1}"))
        })
        .collect();
    assert!(
        missed.is_empty(),
        "did not reach the food from {} of 8 headings:\n  {}",
        missed.len(),
        missed.join("\n  ")
    );
}

#[test]
fn a_blind_agent_does_not_find_the_food() {
    // The check above is only worth having if it can fail. Cutting the one connection
    // that carries the gradient leaves an agent with the same body and the same thrust
    // and no way to steer, so it flies in a straight line: it can only pass near the
    // food if it happened to start pointing at it, which one of eight headings does.
    let arrivals = headings()
        .filter(|&yaw| approach(yaw, 3_600, 0.0).1 < ARRIVED)
        .count();
    assert!(
        arrivals <= 1,
        "an agent that cannot steer reached the food from {arrivals} of 8 headings; \
         the gradient is not what the steering test is measuring"
    );
}
