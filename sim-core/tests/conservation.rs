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
//! written wrongly shows up as drift with no test having to anticipate it.

use glam::Vec3;
use sim_core::params::SimParams;
use sim_core::world::World;

/// Everything a tick does today, in spec §2.4's order. Steps that do not exist yet —
/// collision, interaction, births — are simply absent rather than stubbed.
fn tick(world: &mut World) {
    world.rebuild_spatial_hash();
    world.perceive_all();
    world.step_brains();
    world.drive_effectors();
    world.integrate_movement();
    world.grow_plants();
    world.update_chemo();
    world.charge_metabolism();
    world.resolve_deaths();
}

fn populated(seed: u64, agents: u32) -> (World, SimParams) {
    let mut params = SimParams::default();
    params.world.max_agents = agents.max(1);
    params.plants.max_plants = 400;
    let mut world = World::new(seed, params.clone()).expect("valid params");
    let size = params.world.size;
    for i in 0..agents {
        let angle = i as f32 * 2.399_963_2; // golden angle, so they spread out
        let r = size * 0.4 * (i as f32 / agents.max(1) as f32);
        world
            .spawn_founder(Vec3::new(
                size * 0.5 + r * sim_core::math::cos(angle),
                size * 0.5 + r * sim_core::math::sin(angle),
                0.0,
            ))
            .expect("pool has room");
    }
    (world, params)
}

/// Drift, expressed against the energy that has moved through the world rather than
/// against an absolute figure — a millijoule of float error means something very
/// different in a world holding 10 joules than one holding 10 million.
fn relative_drift(world: &World) -> f64 {
    let scale = world
        .ledger()
        .input()
        .max(world.total_energy() as f64)
        .max(1.0);
    world.energy_drift().abs() / scale
}

#[test]
fn energy_is_conserved_over_ten_thousand_ticks() {
    // The acceptance criterion, at the span the plan names.
    let (mut world, _) = populated(1, 200);
    for t in 0..10_000 {
        tick(&mut world);
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
fn conservation_holds_across_seeds() {
    // One seed passing is a weaker claim than it looks: the flows depend on where
    // agents start and how long they live.
    for seed in 1..=4 {
        let (mut world, _) = populated(seed, 120);
        for _ in 0..2_000 {
            tick(&mut world);
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
        tick(&mut world);
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
        tick(&mut world);
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
    let (mut world, params) = populated(5, 0);
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
