//! The golden hash: the highest-value test in the project (spec §7.8).
//!
//! Every other test here asserts something someone thought of. This one asserts that
//! nothing changed at all, which is how it catches the changes nobody predicted — a
//! summation reordered, a `HashMap` introduced, a platform `sin` slipping past the
//! lint, a default nudged in passing.
//!
//! **A failure here is not automatically a bug.** It means behaviour moved. If the move
//! was intended, update the constant in the same commit as the change and say in the
//! message why the behaviour is different. An unexplained update is the red flag; a
//! well-explained one is ordinary work.
//!
//! The runs use `SimParams::default()` on purpose. That makes the test sensitive to
//! every shipped default, which is the point: a default is behaviour, and changing one
//! should require the same deliberate step as changing code.

use glam::Vec3;
use sim_core::params::SimParams;
use sim_core::world::World;

/// Founders on a golden-angle spiral, so the layout is a pure function of the count and
/// carries no accidental symmetry.
fn seeded(seed: u64, params: SimParams, founders: u32) -> World {
    let mut world = World::new(seed, params).expect("valid params");
    let size = world.params().world.size;
    for i in 0..founders {
        let a = i as f32 * 2.399_963_2;
        let r = size * 0.4 * (i as f32 / founders as f32);
        world
            .spawn_founder(Vec3::new(
                size * 0.5 + r * sim_core::math::cos(a),
                size * 0.5 + r * sim_core::math::sin(a),
                0.0,
            ))
            .expect("pool has room");
    }
    world
}

fn advanced(seed: u64, params: SimParams, founders: u32, ticks: u64) -> World {
    let mut world = seeded(seed, params, founders);
    for _ in 0..ticks {
        world.step();
    }
    world
}

fn run(seed: u64, params: SimParams, founders: u32, ticks: u64) -> u64 {
    advanced(seed, params, founders, ticks).state_hash()
}

/// Whether anything was born during the run. An agent present since tick 0 has an age
/// equal to the tick count; anything younger arrived through `resolve_births`.
fn saw_a_birth(world: &World, ticks: u64) -> bool {
    world
        .pool()
        .iter_live()
        .any(|id| (world.agents().age[id.index()] as u64) < ticks)
}

/// The shipped configuration, untouched.
fn shipped() -> SimParams {
    let mut params = SimParams::default();
    params.world.max_agents = 512;
    params
}

/// A configuration that actually reproduces, so the golden run covers the birth path.
/// Measured at M7: the defaults produce no births at all, and a golden hash over a
/// population that only ever starves would not pin `resolve_births` at all.
fn breeding() -> SimParams {
    let mut params = shipped();
    params.body.size = 1.5;
    params.metabolism.k_sensor = 0.003_125;
    params.feeding.reach = 8.0;
    // Brought forward so births land inside a short run. At the shipped 300 and 150 the
    // first birth arrives somewhere past tick 2000, which would make this a slow test
    // that spent most of its time not covering the thing it exists to cover.
    params.reproduction.maturity_ticks = 100;
    params.reproduction.threshold = 120.0;
    params
}

#[test]
fn the_shipped_defaults_hash_to_their_golden_value() {
    // 300 ticks, inside the ~400 an idle founder survives. Run past extinction and the
    // hash pins an empty world plus a decayed field, which is a stable number that has
    // stopped covering agents at all.
    let world = advanced(42, shipped(), 200, 300);
    assert!(world.population() > 0, "pinned a world with nothing left in it");
    assert_eq!(world.state_hash(), 0x8f85_89e2_b663_bc69);
}

#[test]
fn a_breeding_population_hashes_to_its_golden_value() {
    // 500 ticks: measured, the first offspring arrive well before this and the last of
    // them is dead by 1000, so a longer run would pin the birth path without covering it.
    let world = advanced(42, breeding(), 200, 500);
    // Checked rather than assumed. If this config stopped reproducing, the pinned value
    // below would go on passing while covering none of the birth path it exists for.
    assert!(
        saw_a_birth(&world, 500),
        "nothing was born, so this pins a run that never reached resolve_births"
    );
    assert_eq!(world.state_hash(), 0x00a9_fa9c_3c68_127f);
}

#[test]
fn two_runs_in_one_process_agree() {
    // Spec §7.8's stated criterion for M8. Distinct from the pinned values above: those
    // catch behaviour moving between commits, this catches a world whose result depends
    // on anything left over from a previous one — a `static`, an allocator address, a
    // clock.
    assert_eq!(run(7, shipped(), 64, 400), run(7, shipped(), 64, 400));
    assert_eq!(run(7, breeding(), 64, 400), run(7, breeding(), 64, 400));
}

#[test]
fn interleaved_worlds_do_not_contaminate_each_other() {
    // Three worlds stepped in lockstep rather than one after another. A `static`
    // anywhere in `sim-core` passes the test above and fails this one (spec §7.2).
    let alone = run(11, shipped(), 32, 200);
    let mut worlds: Vec<World> = (0..3).map(|_| seeded(11, shipped(), 32)).collect();
    for _ in 0..200 {
        for w in &mut worlds {
            w.step();
        }
    }
    for w in &worlds {
        assert_eq!(w.state_hash(), alone);
    }
}

#[test]
fn a_single_changed_tick_changes_the_hash() {
    // Guards the guard: if `state_hash` folded too little, every assertion above could
    // pass while the run underneath had changed completely.
    assert_ne!(run(42, shipped(), 200, 1_000), run(42, shipped(), 200, 999));
}
