//! Asserts the sim allocates nothing once it is warm.
//!
//! An allocation in the hot loop does not fail a functional test — it shows up as
//! frame-time jitter in the browser and as a slow drift in an overnight headless run,
//! which is exactly the kind of thing nobody traces back to a `Vec::push`. So it is
//! measured directly (spec §2.2a, §7.8 tier 2).
//!
//! Counting is per-thread rather than global: this binary's test harness allocates on
//! its own thread while the measured region runs, and a global counter would fold that
//! noise in and flake.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use glam::Vec3;
use sim_core::arena::{AllocationFailure, VariableArena};
use sim_core::control::BrainInheritance;
use sim_core::ids::AgentId;
use sim_core::params::SimParams;
use sim_core::world::World;

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
    static COUNTING: Cell<bool> = const { Cell::new(false) };
}

struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note_allocation();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // A realloc is a growing collection, which is the failure this test exists to
        // catch — count it like a fresh allocation.
        note_allocation();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

fn note_allocation() {
    // `try_with` because TLS is unavailable during thread teardown, and panicking
    // inside the allocator would abort the process rather than fail the test.
    let _ = COUNTING.try_with(|counting| {
        if counting.get() {
            let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
        }
    });
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Runs `body` with allocation counting on, and reports how many it made.
fn count_allocations(body: impl FnOnce()) -> u64 {
    ALLOCATIONS.with(|n| n.set(0));
    COUNTING.with(|c| c.set(true));
    body();
    COUNTING.with(|c| c.set(false));
    ALLOCATIONS.with(|n| n.get())
}

fn at(i: u32) -> Vec3 {
    Vec3::new(i as f32 % 500.0, (i / 500) as f32, 0.0)
}

#[test]
fn the_counter_actually_counts() {
    // Without this, a broken counter makes every other assertion in the file vacuous.
    let observed = count_allocations(|| {
        let v: Vec<u64> = (0..1_000).collect();
        std::hint::black_box(&v);
    });
    assert!(
        observed > 0,
        "the counting allocator is not observing allocations"
    );
}

#[test]
fn genetic_distance_never_allocates_or_changes_a_world() {
    let mut params = SimParams::default();
    params.world.max_agents = 4;
    params.plants.max_plants = 8;
    let mut world = World::new(42, params).unwrap();
    world.seed_founders(2);
    let before = world.state_hash();
    let observed = count_allocations(|| {
        for _ in 0..100 {
            std::hint::black_box(sim_core::distance::between(
                world.genome(AgentId::new(0)),
                world.genome(AgentId::new(1)),
                &world.params().distance,
            ));
        }
    });
    assert_eq!(observed, 0, "distance allocated {observed} times");
    assert_eq!(world.state_hash(), before);
}

#[test]
fn species_assignment_refusal_and_retirement_never_allocate() {
    use sim_core::species::{Classifier, Departure, Unclassified};
    let params = SimParams::default();
    let mut world_params = params.clone();
    world_params.world.max_agents = 4;
    world_params.plants.max_plants = 8;
    let mut world = World::new(42, world_params).unwrap();
    world.seed_founders(2);
    let genes = world.genome(AgentId::new(0));
    let other = world.genome(AgentId::new(1));
    let before = world.state_hash();
    let mut classifier =
        Classifier::try_new(1, 1024, f64::MIN_POSITIVE, params.distance, 65536).unwrap();
    let observed = count_allocations(|| {
        for _ in 0..100 {
            let id = classifier.classify(genes).unwrap().species;
            assert_eq!(classifier.classify(other), Err(Unclassified::Capacity));
            assert_eq!(classifier.classify(genes).unwrap().species, id);
            assert_eq!(classifier.remove_member(id), Ok(Departure::MemberRemoved));
            assert_eq!(classifier.remove_member(id), Ok(Departure::Extinct));
            assert!(classifier.remove_member(id).is_err());
        }
    });
    assert_eq!(observed, 0, "species operations allocated {observed} times");
    assert_eq!(world.state_hash(), before);
}

#[test]
fn variable_arena_churn_and_refusals_never_allocate() {
    let mut arena = VariableArena::<u32>::try_with_capacity(96, 8).unwrap();
    let mut limited = VariableArena::<u32>::try_with_capacity(8, 1).unwrap();

    let observed = count_allocations(|| {
        for _ in 0..1_000 {
            let a = arena.alloc(8).unwrap();
            let b = arena.alloc(16).unwrap();
            let c = arena.alloc(24).unwrap();
            let d = arena.alloc(32).unwrap();
            arena.get_mut(b).fill(42);
            arena.free(b);
            arena.free(d);
            assert_eq!(arena.alloc(56), Err(AllocationFailure::Fragmented));
            assert_eq!(arena.alloc(65), Err(AllocationFailure::InsufficientSpace));
            let replacement = arena.alloc(16).unwrap();
            assert!(arena.get(replacement).iter().all(|&value| value == 0));
            arena.free(a);
            arena.free(c);
            arena.free(replacement);
            let whole = arena.alloc(96).unwrap();
            arena.free(whole);

            let block = limited.alloc(1).unwrap();
            assert_eq!(limited.alloc(1), Err(AllocationFailure::BlockLimit));
            let empty = limited.alloc(0).unwrap();
            limited.free(empty);
            limited.free(block);
        }
    });
    assert_eq!(observed, 0, "variable arena allocated {observed} times");
}

#[test]
fn variable_arena_resets_by_copy_without_calling_default() {
    #[derive(Clone, Copy)]
    struct AllocatingDefault(u32);

    impl Default for AllocatingDefault {
        fn default() -> Self {
            let values = std::hint::black_box(vec![17u32; 4]);
            Self(values[0])
        }
    }

    let mut arena = VariableArena::<AllocatingDefault>::try_with_capacity(8, 1).unwrap();
    let observed = count_allocations(|| {
        for _ in 0..20 {
            let block = arena.alloc(8).unwrap();
            assert!(arena.get(block).iter().all(|value| value.0 == 17));
            for value in arena.get_mut(block) {
                value.0 = 99;
            }
            arena.free(block);
        }
    });
    assert_eq!(observed, 0, "reset invoked an allocating Default");
}

#[test]
fn variable_world_births_refusals_and_observers_never_allocate() {
    use sim_core::genome::{Activation, Gene, NeuronGene};
    use sim_core::spawn::SpawnFailureCounts;
    use sim_core::{InnovationId, SpawnSpec};

    let mut params = SimParams::default();
    params.world.max_agents = 8;
    params.plants.max_plants = 8;
    params.reproduction.maturity_ticks = 0;
    let genes: Vec<_> = (0..96)
        .map(|id| {
            Gene::Neuron(NeuronGene {
                id: InnovationId::new(id),
                bias: 0.0,
                tau: 1.0,
                activation: Activation::Sigmoid,
                period: 1.0,
            })
        })
        .collect();
    for mode in [
        BrainInheritance::Evolving,
        BrainInheritance::RandomizedAtBirth,
    ] {
        let mut world = World::new_with_brain_inheritance(7, params.clone(), mode).unwrap();
        let parent = world
            .spawn(
                &SpawnSpec {
                    energy: 300.0,
                    position: Vec3::ZERO,
                    yaw: 0.0,
                    size: 3.0,
                    signature: Vec3::ONE,
                    parent_a: AgentId::NULL,
                },
                &genes,
            )
            .unwrap();
        let mut failures = SpawnFailureCounts::default();
        let observed = count_allocations(|| {
            for _ in 0..20 {
                world.agents_mut().energy[parent.index()] = 300.0;
                world.intents_mut().reproduce[parent.index()] = 1.0;
                assert_eq!(
                    world.resolve_births_with_observer(|error| failures.record(error)),
                    1
                );
                world.agents_mut().energy[parent.index()] = 300.0;
                assert_eq!(
                    world.resolve_births_with_observer(|error| failures.record(error)),
                    0
                );
                let child = world.pool().iter_live().find(|&id| id != parent).unwrap();
                world.despawn(child);
                let founder = world.spawn_founder(Vec3::ZERO).unwrap();
                world.despawn(founder);
            }
        });
        assert_eq!(observed, 0, "variable world lifecycle allocated");
        assert!(failures.arena_capacity > 0);
    }
}

#[test]
fn neural_structural_births_and_observers_never_allocate() {
    use sim_core::mutate::structural::StructuralMutationCounts;
    let mut params = SimParams::default();
    params.world.max_agents = 8;
    params.plants.max_plants = 8;
    params.reproduction.maturity_ticks = 0;
    let rates = &mut params.mutation.structural;
    rates.remove_connection_rate = 1.0;
    rates.remove_neuron_rate = 1.0;
    rates.toggle_connection_rate = 1.0;
    rates.add_connection_rate = 1.0;
    rates.add_neuron_rate = 1.0;
    for mode in [
        BrainInheritance::Evolving,
        BrainInheritance::RandomizedAtBirth,
    ] {
        let mut world = World::new_with_brain_inheritance(42, params.clone(), mode).unwrap();
        let parent = world.spawn_founder(Vec3::ZERO).unwrap();
        let mut counts = StructuralMutationCounts::default();
        let observed = count_allocations(|| {
            for _ in 0..20 {
                world.agents_mut().energy[parent.index()] = 300.0;
                world.intents_mut().reproduce[parent.index()] = 1.0;
                assert_eq!(
                    world.resolve_births_with_observers(
                        |_| panic!("birth refused"),
                        |event| counts.record(event),
                    ),
                    1
                );
                let child = world.pool().iter_live().find(|&id| id != parent).unwrap();
                world.despawn(child);
            }
        });
        assert_eq!(observed, 0, "structural birth allocated");
        for applied in [
            counts.remove_connection.applied,
            counts.remove_neuron.applied,
            counts.toggle_connection.applied,
            counts.add_connection.applied,
            counts.add_neuron.applied,
        ] {
            assert_eq!(applied, 20, "an operator did not execute");
        }
    }
}

#[test]
fn sensor_edits_and_combined_mutation_observers_never_allocate() {
    use sim_core::mutate::StructuralMutationCounts;
    use sim_core::species::SpeciesEventCounts;
    let mut params = SimParams::default();
    params.world.max_agents = 8;
    params.plants.max_plants = 8;
    params.reproduction.maturity_ticks = 0;
    params.mutation.organs.remove_sensor_rate = 1.0;
    params.mutation.organs.add_sensor_rate = 1.0;
    params.species.capacity = 1;
    params.species.threshold = f64::MIN_POSITIVE;
    let rates = &mut params.mutation.structural;
    rates.remove_connection_rate = 1.0;
    rates.remove_neuron_rate = 1.0;
    rates.toggle_connection_rate = 1.0;
    rates.add_connection_rate = 1.0;
    rates.add_neuron_rate = 1.0;
    for mode in [
        BrainInheritance::Evolving,
        BrainInheritance::RandomizedAtBirth,
    ] {
        let mut world = World::new_with_brain_inheritance(42, params.clone(), mode).unwrap();
        let parent = world.spawn_founder(Vec3::ZERO).unwrap();
        let mut counts = StructuralMutationCounts::default();
        let mut species = SpeciesEventCounts::default();
        let observed = count_allocations(|| {
            for _ in 0..20 {
                world.agents_mut().energy[parent.index()] = 300.0;
                world.intents_mut().reproduce[parent.index()] = 1.0;
                assert_eq!(
                    world.resolve_births_with_all_observers(
                        |_| panic!("birth refused"),
                        |event| counts.record(event),
                        |event| species.record(event),
                    ),
                    1
                );
                let child = world.pool().iter_live().find(|&id| id != parent).unwrap();
                world.despawn_with_species_observer(child, |event| species.record(event));
            }
        });
        assert_eq!(observed, 0);
        assert_eq!(counts.add_sensor.unwrap().applied, 20);
        assert_eq!(counts.remove_sensor.unwrap().applied, 20);
        assert_eq!(species.unclassified_capacity, 20);
    }
}

#[test]
fn invalid_founder_params_do_not_allocate_a_plan() {
    let mut params = SimParams::default();
    params.brain.hidden_neurons = 200;
    let observed = count_allocations(|| {
        let result =
            sim_core::founder::FounderPlan::new(&params, &mut sim_core::Rng::from_seed(0), || {
                panic!("invalid params must not request innovation ids")
            });
        assert!(result.is_err());
    });
    assert_eq!(observed, 0, "invalid params allocated a founder plan");
}

#[test]
fn spawn_and_despawn_never_allocate() {
    let mut params = SimParams::default();
    params.world.max_agents = 10_000;
    params.storage.max_memory_bytes = 192 * 1_048_576;
    let mut world = World::new(42, params).expect("valid params");

    // Warmup: the first pass touches every free-list and arena path. Construction
    // allocates by design — the pools are sized once, up front.
    let mut ids: Vec<AgentId> = Vec::with_capacity(10_000);
    for i in 0..10_000 {
        ids.push(world.spawn_founder(at(i)).expect("pool sized for 10k"));
    }
    for id in ids.drain(..) {
        world.despawn(id);
    }

    let observed = count_allocations(|| {
        for round in 0..5 {
            for i in 0..10_000u32 {
                let id = world
                    .spawn_founder(at(i + round * 10_000))
                    .expect("pool has room");
                ids.push(id);
            }
            // Churn in reverse, so the free list is exercised rather than replayed.
            while let Some(id) = ids.pop() {
                world.despawn(id);
            }
        }
        std::hint::black_box(&world);
    });

    assert_eq!(
        observed, 0,
        "spawn/despawn allocated {observed} times after warmup"
    );
}

#[test]
fn rebuilding_and_querying_the_spatial_hash_never_allocates() {
    // This runs every tick for every agent — step 1 of the tick, then once per sensor
    // during perception. It is the hottest path in the simulation (spec §2.2c).
    let mut params = SimParams::default();
    params.world.max_agents = 5_000;
    let mut world = World::new(9, params).expect("valid params");
    for i in 0..5_000 {
        world.spawn_founder(at(i)).expect("pool sized for 5k");
    }

    // Warmup: first rebuild touches every bucket and cursor.
    world.rebuild_spatial_hash();

    let observed = count_allocations(|| {
        for _ in 0..20 {
            world.rebuild_spatial_hash();
            let positions = &world.agents().position;
            let radius = world.params().sensing.max_sense_radius();
            let mut seen = 0u64;
            for id in 0..500u32 {
                world.spatial_hash().for_each_within(
                    positions,
                    positions[id as usize],
                    radius,
                    |_, _, _| seen += 1,
                );
            }
            std::hint::black_box(seen);
        }
    });

    assert_eq!(
        observed, 0,
        "spatial hash allocated {observed} times after warmup"
    );
}

#[test]
fn a_whole_tick_of_systems_never_allocates() {
    // Every step of spec §2.4 that exists: rebuild, perceive, think, decide, move, and
    // the field update. Perception alone is expected to be 60–80% of tick cost once
    // vision is real (spec §2.2c) — it runs a spatial-hash query per eye per agent per
    // tick — so an allocation anywhere in here is the most expensive one in the project.
    let mut params = SimParams::default();
    params.world.max_agents = 2_000;
    let mut world = World::new(13, params).expect("valid params");
    for i in 0..2_000 {
        world.spawn_founder(at(i)).expect("pool sized for 2k");
    }

    // Something to smell and something to see, or the sensors take their cheapest path.
    world.deposit_chemo(0, Vec3::new(250.0, 250.0, 0.0), 500.0);
    world.rebuild_spatial_hash();
    world.perceive_all();
    world.drive_effectors();
    world.integrate_movement();
    world.grow_plants();
    world.update_chemo();
    // Warm the death path too: `dying` is sized at capacity up front, and a `Vec::push`
    // that reallocated would be an allocation inside step 10.
    world.charge_metabolism();
    world.resolve_deaths();
    // Warm the birth path too: a birth copies a genome into the world's scratch buffer
    // and claims six arena blocks, and any of those reallocating is an allocation in
    // step 10.
    world.resolve_births();
    world.advance_tick();

    let observed = count_allocations(|| {
        for _ in 0..20 {
            world.step();
        }
        std::hint::black_box(&world);
    });

    assert_eq!(
        observed, 0,
        "a tick allocated {observed} times after warmup"
    );
}

#[test]
fn random_control_births_never_allocate() {
    let mut params = SimParams::default();
    params.world.max_agents = 4;
    params.plants.max_plants = 8;
    params.reproduction.maturity_ticks = 0;
    let mut world =
        World::new_with_brain_inheritance(23, params, BrainInheritance::RandomizedAtBirth)
            .expect("valid params");
    let parent = world.spawn_founder(at(0)).expect("pool has room");

    let birth_and_remove = |world: &mut World| {
        let rich = world.params().reproduction.threshold + 100.0;
        world.agents_mut().energy[parent.index()] = rich;
        world.intents_mut().reproduce[parent.index()] = 1.0;
        assert_eq!(world.resolve_births(), 1);
        let child = world
            .pool()
            .iter_live()
            .find(|&id| id != parent)
            .expect("child was born");
        world.despawn(child);
    };
    birth_and_remove(&mut world);

    let observed = count_allocations(|| {
        for _ in 0..20 {
            birth_and_remove(&mut world);
        }
        std::hint::black_box(&world);
    });
    assert_eq!(
        observed, 0,
        "a random-control birth allocated {observed} times"
    );
}

#[test]
fn stepping_every_brain_never_allocates() {
    // Step 3 of the tick, run for every agent every tick (spec §2.4). The scratch
    // buffer the Euler step writes into is owned by the world and sized once; if it
    // ever becomes a per-agent `Vec` this is what says so.
    let mut params = SimParams::default();
    params.world.max_agents = 2_000;
    let mut world = World::new(11, params).expect("valid params");
    for i in 0..2_000 {
        world.spawn_founder(at(i)).expect("pool sized for 2k");
    }

    world.step_brains();

    let observed = count_allocations(|| {
        for _ in 0..20 {
            world.step_brains();
        }
        std::hint::black_box(&world);
    });

    assert_eq!(
        observed, 0,
        "stepping brains allocated {observed} times after warmup"
    );
}

#[test]
fn a_full_pool_refuses_without_allocating() {
    // The interesting case: at the population ceiling, every birth is a rejection.
    let mut params = SimParams::default();
    params.world.max_agents = 64;
    let mut world = World::new(1, params).expect("valid params");
    for i in 0..64 {
        world.spawn_founder(at(i)).expect("pool sized for 64");
    }
    let observed = count_allocations(|| {
        for i in 0..10_000 {
            assert!(world.spawn_founder(at(i)).is_err());
        }
    });
    assert_eq!(observed, 0, "rejecting a birth allocated {observed} times");
}

#[test]
fn first_and_growing_command_batches_do_not_allocate_inside_a_tick() {
    use sim_core::command::{Command, Kind};
    use sim_core::species::SpeciesEventCounts;

    let mut params = SimParams::default();
    params.world.max_agents = 32;
    params.plants.max_plants = 8;
    params.chemo.cells = [8, 8, 1];
    let mut world = World::new(21, params).expect("valid params");
    let mut species = SpeciesEventCounts::default();

    for count in [1, 8, 64] {
        for i in 0..count {
            world.push_command(Command::now(Kind::SpawnFounder { position: at(i) }));
        }
        let observed = count_allocations(|| {
            world.step_with_all_observers(|_| {}, |_| {}, |event| species.record(event))
        });
        assert_eq!(
            observed, 0,
            "draining a new batch of {count} commands allocated inside the tick"
        );
        assert_eq!(world.pending_commands(), 0);
        assert!(world.population() > 0, "commands did not spawn agents");
    }
    assert!(species.created > 0);
}

#[test]
fn draining_the_command_queue_never_allocates() {
    // The drain runs through two reusable buffers, and it is easy to write it so one of
    // them is freed and regrown every tick. `a_whole_tick_of_systems_never_allocates`
    // cannot see that: its queue is empty, so the drain returns before touching either.
    use sim_core::command::{Command, Kind};

    let mut params = SimParams::default();
    params.world.max_agents = 256;
    let mut world = World::new(21, params).expect("valid params");

    // Warm both queues, and every pool a spawn touches.
    for i in 0..64u32 {
        world.push_command(Command::at(
            i as u64,
            Kind::SpawnFounder { position: at(i) },
        ));
    }
    for _ in 0..64 {
        world.step();
    }
    assert_eq!(world.pending_commands(), 0, "warmup left work behind");
    assert!(
        world.population() > 0,
        "nothing spawned, so nothing was warmed"
    );

    let observed = count_allocations(|| {
        for i in 0..32u32 {
            let tick = world.tick_count();
            world.push_command(Command::at(tick, Kind::SpawnFounder { position: at(i) }));
            world.step();
        }
        std::hint::black_box(&world);
    });

    assert_eq!(
        observed, 0,
        "draining the command queue allocated {observed} times after warmup"
    );
}

#[test]
fn writing_the_render_snapshot_never_allocates() {
    // Written once per tick for the life of a run (spec §2.2b). It is also the one
    // buffer JS holds views over, so a reallocation here would not merely cost time —
    // growing WASM memory detaches every existing view, silently (spec §7.3).
    use sim_core::snapshot::Snapshot;

    let mut params = SimParams::default();
    params.world.max_agents = 2_000;
    let mut world = World::new(23, params).expect("valid params");
    for i in 0..2_000 {
        world.spawn_founder(at(i)).expect("pool sized for 2k");
    }
    let mut snapshot = Snapshot::for_world(&world);
    snapshot.update(&world);

    let observed = count_allocations(|| {
        for _ in 0..20 {
            world.step();
            snapshot.update(&world);
        }
        std::hint::black_box(&snapshot);
    });

    assert_eq!(
        observed, 0,
        "writing the snapshot allocated {observed} times after warmup"
    );
    assert!(
        snapshot.population() > 0,
        "everything died; nothing was written"
    );
}
