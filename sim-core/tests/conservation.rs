//! M7's acceptance criterion: energy is conserved.
//!
//! Spec §5.1's claim is that energy enters at a fixed rate and leaves only through
//! dissipation. Every way it breaks is silent — a birth that fills a tank without
//! charging anyone, a plant that regrows what was eaten, a charge taken past zero — and
//! not one of them fails a functional test. The world would run, look plausible, and
//! stop selecting for anything, because when energy is free every strategy works.
//!
//! So the test does not check that the code does what it says. It counts what entered
//! and what left, and compares against what is actually there:
//!
//! ```text
//! stock_now  ==  stock_at_start + input - dissipated
//! ```
//!
//! Transfers are deliberately absent from the ledger, which is what makes this catch
//! mistakes nobody predicted: energy moving between two holders cancels, so a transfer
//! written wrongly shows up as drift with no test having to anticipate it. Both of them
//! now exist — an agent eating a plant, and a parent splitting its tank with a child —
//! and neither is recorded anywhere. If either is wrong, these tests say so.

use sim_core::params::SimParams;
use sim_core::world::World;

fn populated(seed: u64, agents: u32) -> (World, SimParams) {
    populated_with_fill(seed, agents, SimParams::default().plants.initial_fill)
}

/// As [`populated`], with the larder stocked to `fill` of `max_energy`. Only the tests
/// that measure the larder *filling* want anything but the default, which is full.
fn populated_with_fill(seed: u64, agents: u32, fill: f32) -> (World, SimParams) {
    let mut params = SimParams::default();
    params.world.max_agents = agents.max(1);
    params.plants.max_plants = 400;
    params.plants.initial_fill = fill;
    let mut world = World::new(seed, params.clone()).expect("valid params");
    assert_eq!(world.seed_founders(agents), agents, "pool has room");
    (world, params)
}

/// Drift, expressed against the energy that has moved through the world rather than
/// against an absolute figure — a millijoule of float error means something very
/// different in a world holding 10 joules than one holding 10 million.
fn relative_drift(world: &World) -> f64 {
    let scale = world.ledger().input().max(world.total_energy()).max(1.0);
    world.energy_drift().abs() / scale
}

#[test]
fn energy_is_conserved_over_ten_thousand_ticks() {
    // The acceptance criterion, at the span the plan names.
    let (mut world, _) = populated(1, 200);
    for t in 0..10_000 {
        world.step();
        assert!(
            relative_drift(&world) < 1e-4,
            "tick {t}: {:.6} drifted from a ledger of {:.2} in / {:.2} out",
            world.energy_drift(),
            world.ledger().input(),
            world.ledger().dissipated(),
        );
    }
    assert!(
        world.ledger().input() > 0.0,
        "no energy ever entered; the test proved nothing"
    );
    assert!(
        world.ledger().dissipated() > 0.0,
        "no energy ever left; the test proved nothing"
    );
}

#[test]
fn fractional_growth_in_a_large_plant_pool_is_recorded_exactly() {
    let mut params = SimParams::default();
    params.world.max_agents = 1;
    params.plants.max_plants = 100_000;
    params.plants.initial_fill = 0.001;
    params.plants.max_energy = 100.0;
    params.plants.energy_input_rate = 60_000_000.0;
    let mut world = World::new(1, params).expect("valid params");

    let absorbed = world.grow_plants();
    assert!(absorbed > 0.0);
    assert_eq!(world.ledger().input(), absorbed);
    assert!(
        world.energy_drift().abs() < 1e-9,
        "{} joules drifted from aggregation alone",
        world.energy_drift()
    );
}

#[test]
fn lowering_the_live_plant_cap_does_not_destroy_stock() {
    let mut params = SimParams::default();
    params.world.max_agents = 1;
    params.plants.max_plants = 4;
    let mut world = World::new(2, params).expect("valid params");
    let before = world.total_energy();
    let mut retuned = world.params().clone();
    retuned.plants.max_energy *= 0.5;
    world.set_params(retuned).expect("live cap may be lowered");

    assert_eq!(world.grow_plants(), 0.0);
    assert_eq!(world.total_energy(), before);
    assert_eq!(world.energy_drift(), 0.0);
}

#[test]
fn repeated_feeding_into_a_large_balance_conserves_energy() {
    let mut params = SimParams::default();
    params.world.max_agents = 1;
    params.plants.max_plants = 1;
    params.plants.initial_fill = 0.001;
    params.plants.max_energy = 100.0;
    params.plants.energy_input_rate = 60.0;
    params.reproduction.start_energy = 1_000_000.0;
    params.reproduction.threshold = 2_000_000.0;
    params.metabolism.base = 0.0;
    params.metabolism.k_size = 0.0;
    params.metabolism.k_brain = 0.0;
    params.metabolism.k_sensor = 0.0;
    params.metabolism.k_move = 0.0;
    params.feeding.rate = 0.7;
    params.feeding.gate = 0.0;
    params.feeding.reach = 0.0;
    let mut world = World::new(9, params).expect("valid params");
    let at = world.plants().position()[0];
    let id = world.spawn_founder(at).expect("room");
    let agent_before = world.agents().energy[id.index()];

    for tick in 0..100_000 {
        world.intents_mut().ingest[id.index()] = 1.0;
        world.resolve_feeding();
        world.grow_plants();
        assert!(
            world.energy_drift().abs() < 1e-9,
            "tick {tick}: {} joules drifted",
            world.energy_drift()
        );
    }
    assert!(
        world.agents().energy[id.index()] > agent_before,
        "feeding never credited the large agent balance"
    );
    assert!(
        world.agents().energy_reserve[id.index()] < 1.0,
        "rounding reserve grew without being reclaimed"
    );
}

#[test]
fn sub_ulp_feeding_progress_stays_with_each_agent() {
    let mut params = SimParams::default();
    params.world.max_agents = 2;
    params.plants.max_plants = 1;
    params.plants.initial_fill = 1.0;
    params.plants.max_energy = 1_000_000.0;
    params.plants.energy_input_rate = 0.0;
    params.metabolism.base = 0.0;
    params.metabolism.k_size = 0.0;
    params.metabolism.k_brain = 0.0;
    params.metabolism.k_sensor = 0.0;
    params.metabolism.k_move = 0.0;
    params.feeding.rate = 0.01;
    params.feeding.gate = 0.0;
    params.feeding.reach = 0.0;
    let mut world = World::new(12, params).expect("valid params");
    let at = world.plants().position()[0];
    let a = world.spawn_founder(at).expect("room");
    let b = world.spawn_founder(at).expect("room");
    let before_a = world.agents().energy[a.index()] as f64;
    let before_b = world.agents().energy[b.index()] as f64;

    for _ in 0..10 {
        world.intents_mut().ingest[a.index()] = 1.0;
        world.intents_mut().ingest[b.index()] = 1.0;
        world.resolve_feeding();
    }

    let agents = world.agents();
    let after_a = agents.energy[a.index()] as f64 + agents.energy_reserve[a.index()];
    let after_b = agents.energy[b.index()] as f64 + agents.energy_reserve[b.index()];
    assert!((after_a - before_a - 0.1).abs() < 1e-8);
    assert!((after_b - before_b - 0.1).abs() < 1e-8);
    assert_eq!(world.energy_drift(), 0.0);
}

#[test]
fn conservation_holds_across_seeds() {
    // One seed passing is a weaker claim than it looks: the flows depend on where
    // agents start and how long they live.
    for seed in 1..=4 {
        let (mut world, _) = populated(seed, 120);
        for _ in 0..2_000 {
            world.step();
        }
        assert!(
            relative_drift(&world) < 1e-4,
            "seed {seed} drifted by {:.6}",
            world.energy_drift()
        );
    }
}

#[test]
fn an_agent_that_starves_takes_nothing_with_it() {
    // Death is where energy most easily vanishes unaccounted: an agent charged past
    // zero dissipates joules the world never held, and one despawned with energy still
    // in it deletes joules that were never spent.
    let (mut world, params) = populated(7, 50);
    let start = world.population();
    assert!(start > 0);

    // Long enough for the default budget to starve every one of them — an idle agent
    // lasts a few hundred ticks, and none of them can eat yet.
    for _ in 0..4_000 {
        world.step();
        assert!(
            relative_drift(&world) < 1e-4,
            "drifted while agents were dying: {:.6}",
            world.energy_drift()
        );
    }
    assert_eq!(
        world.population(),
        0,
        "nothing starved; is metabolism charging?"
    );

    // Everything those agents held has to have been dissipated, not deleted.
    let held = params.reproduction.start_energy as f64 * start as f64;
    assert!(
        world.ledger().dissipated() >= held * 0.99,
        "agents held {held:.0} between them but only {:.0} was ever dissipated",
        world.ledger().dissipated()
    );
}

#[test]
fn energy_never_goes_negative() {
    // The charge is clamped to what an agent actually holds. Without that, a big
    // movement cost on a nearly-empty agent would push it below zero and the ledger
    // would report a leak that is really an overdraft.
    let (mut world, _) = populated(3, 80);
    for _ in 0..4_000 {
        world.step();
        for id in world.pool().iter_live() {
            let energy = world.agents().energy[id.index()];
            assert!(energy >= 0.0, "agent {id:?} went to {energy}");
        }
    }
}

#[test]
fn a_world_at_carrying_capacity_stops_absorbing() {
    // The input rate is nominal, not guaranteed. Once every plant is full the surplus
    // never enters the world, and the ledger has to record what happened rather than
    // what was asked for — otherwise conservation fails against a rate nobody supplied.
    // Empty on purpose: absorption stopping at the ceiling is only observable if there
    // is a ceiling left to reach. The shipped default stocks the larder at construction
    // instead, and puts that energy in the ledger's opening balance rather than its
    // input — see `PlantParams::initial_fill`.
    let (mut world, params) = populated_with_fill(5, 0, 0.0);
    for _ in 0..200_000 {
        world.grow_plants();
    }
    let ceiling = params.plants.max_energy as f64 * 400.0;
    assert!(
        (world.ledger().input() - ceiling).abs() < ceiling * 1e-3,
        "absorbed {:.0}, carrying capacity is {ceiling:.0}",
        world.ledger().input()
    );
    assert!(relative_drift(&world) < 1e-4);
}

/// A world with a handful of grown plants and one agent standing on the first of them.
///
/// Everything the reproduction tests need is earned through the economy rather than
/// written into the arrays: an agent made rich by assignment would be holding joules the
/// ledger never saw, and every drift assertion after it would be measuring the setup.
fn fed_agent(seed: u64, target: f32) -> (World, SimParams, sim_core::ids::AgentId) {
    let mut params = SimParams::default();
    params.world.max_agents = 4;
    params.plants.max_plants = 6;
    let mut world = World::new(seed, params.clone()).expect("valid params");

    // Grow the plants first, so there is something to eat.
    for _ in 0..2_000 {
        world.grow_plants();
    }
    let larder = world.plants().position()[0];
    let id = world.spawn_founder(larder).expect("room");

    // Eat until rich enough, without any of the rest of the tick running — this is
    // about the transfer, not about surviving long enough to make it.
    for _ in 0..20_000 {
        if world.agents().energy[id.index()] >= target {
            break;
        }
        world.intents_mut().ingest[id.index()] = 1.0;
        world.resolve_feeding();
        world.grow_plants();
    }
    (world, params, id)
}

#[test]
fn eating_moves_energy_without_creating_it() {
    // The first transfer. An agent gaining what a plant loses is invisible to the ledger
    // by construction, so a rate credited to the agent but not deducted from the plant —
    // or deducted twice — shows up here and nowhere else.
    let mut params = SimParams::default();
    params.world.max_agents = 4;
    params.plants.max_plants = 6;
    let mut world = World::new(13, params.clone()).expect("valid params");
    for _ in 0..2_000 {
        world.grow_plants();
    }
    let larder = world.plants().position()[0];
    let id = world.spawn_founder(larder).expect("room");

    let plants_before = world.plants().total_energy();
    let agent_before = world.agents().energy[id.index()];
    let stock_before = world.total_energy();

    world.intents_mut().ingest[id.index()] = 1.0;
    world.resolve_feeding();

    let eaten = world.agents().energy[id.index()] - agent_before;
    assert!(eaten > 0.0, "nothing was eaten; is the gate right?");
    assert!(
        (world.plants().total_energy() - (plants_before - eaten as f64)).abs() < 1e-3,
        "the plant did not lose what the agent gained"
    );
    assert!(
        (world.total_energy() - stock_before).abs() < 1e-3,
        "eating changed the world's total energy"
    );
    assert!(relative_drift(&world) < 1e-4, "{:.6}", world.energy_drift());
}

#[test]
fn an_agent_below_the_gate_does_not_eat() {
    // The gate is what makes eating a decision. Without it every agent would feed
    // constantly and the ingest effector would be decoration.
    let (mut world, params, id) = fed_agent(17, 120.0);
    let plants_before = world.plants().total_energy();
    let agent_before = world.agents().energy[id.index()];

    world.intents_mut().ingest[id.index()] = params.feeding.gate - 0.01;
    world.resolve_feeding();
    assert_eq!(
        world.agents().energy[id.index()],
        agent_before,
        "ate anyway"
    );
    assert_eq!(world.plants().total_energy(), plants_before);
}

#[test]
fn a_birth_splits_a_tank_rather_than_filling_one() {
    // The second transfer, and the one most likely to be written as a grant: an
    // offspring handed `start_energy` instead of a share of its parent conjures a full
    // tank on every birth, and a population grows on free energy forever.
    let (mut world, params, parent) = fed_agent(21, params_threshold());
    assert!(
        world.agents().energy[parent.index()] >= params.reproduction.threshold,
        "the parent never got rich enough to breed"
    );
    // Age is not energy, so setting it directly leaves the books alone.
    world.agents_mut().age[parent.index()] = params.reproduction.maturity_ticks;
    world.intents_mut().reproduce[parent.index()] = 1.0;

    let before = world.total_energy();
    let parent_before = world.agents().energy[parent.index()];
    let born = world.resolve_births();

    assert_eq!(born, 1, "no offspring");
    assert_eq!(world.population(), 2);
    assert!(
        (world.total_energy() - before).abs() < 1e-3,
        "a birth changed the world's energy: {before} -> {}",
        world.total_energy()
    );
    assert!(
        world.agents().energy[parent.index()] < parent_before,
        "the parent paid nothing for its child"
    );
    assert!(relative_drift(&world) < 1e-4, "{:.6}", world.energy_drift());
}

/// The reproduction threshold, so `fed_agent` knows how rich to get.
fn params_threshold() -> f32 {
    SimParams::default().reproduction.threshold
}

#[test]
fn a_refused_birth_leaves_the_parent_whole() {
    // At the population ceiling a birth is refused. Charging the parent for a child that
    // never existed would destroy energy on the busiest tick of a run — and the ceiling
    // is exactly when a run is busiest.
    let mut params = SimParams::default();
    params.world.max_agents = 1;
    params.plants.max_plants = 6;
    let mut world = World::new(31, params.clone()).expect("valid params");
    for _ in 0..2_000 {
        world.grow_plants();
    }
    let larder = world.plants().position()[0];
    let parent = world.spawn_founder(larder).expect("room for one");
    for _ in 0..20_000 {
        if world.agents().energy[parent.index()] >= params.reproduction.threshold {
            break;
        }
        world.intents_mut().ingest[parent.index()] = 1.0;
        world.resolve_feeding();
        world.grow_plants();
    }
    world.agents_mut().age[parent.index()] = params.reproduction.maturity_ticks;
    world.intents_mut().reproduce[parent.index()] = 1.0;

    let held = world.agents().energy[parent.index()];
    let stock = world.total_energy();
    let born = world.resolve_births();

    assert_eq!(born, 0, "spawned past the ceiling");
    assert_eq!(
        world.agents().energy[parent.index()],
        held,
        "the parent paid for a child that was never born"
    );
    assert!((world.total_energy() - stock).abs() < 1e-3);
    assert!(relative_drift(&world) < 1e-4, "{:.6}", world.energy_drift());
}

#[test]
fn a_population_that_eats_and_breeds_still_conserves() {
    // Everything at once, for the span the milestone names. This is the run where a
    // mistake in either transfer compounds: births make more eaters, eating funds more
    // births, and a leak grows with the population rather than staying constant.
    let (mut world, _) = populated(41, 150);
    for t in 0..10_000 {
        world.step();
        assert!(
            relative_drift(&world) < 1e-4,
            "tick {t}: drifted {:.6} with {} alive",
            world.energy_drift(),
            world.population()
        );
    }
}
