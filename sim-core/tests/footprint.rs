//! Guards the memory a world commits up front.
//!
//! Shared arena allowances scale with the pool, independently of founder composition.
//! The runtime core budget includes cumulative construction requests, not only retained
//! buffers (spec §2.2a). Compare the allocation-free estimate against a counting
//! allocator so new buffers cannot silently escape boundary validation.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use sim_core::founder::FounderPlan;
use sim_core::genome::Gene;
use sim_core::ids::InnovationId;
use sim_core::params::SimParams;
use sim_core::rng::Rng;
use sim_core::world::World;

// Per-thread, not global: tests in this binary run in parallel, and a shared counter
// means each one zeroes the others' measurement mid-flight. The same reason
// `no_alloc.rs` counts per-thread.
thread_local! {
    static BYTES: Cell<usize> = const { Cell::new(0) };
}

fn note(size: usize) {
    let _ = BYTES.try_with(|b| b.set(b.get() + size));
}

fn take() -> usize {
    BYTES.with(|b| b.replace(0))
}

struct Tally;

unsafe impl GlobalAlloc for Tally {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Tally = Tally;

fn measure(params: SimParams) -> (World, u64, u64) {
    // The parameter Vec was allocated before this measurement but moves into World.
    let parameter_bytes = (params.chemo.decay.capacity() * size_of::<f32>()) as u64;
    take();
    let estimate = params.estimated_construction_bytes().expect("valid params");
    assert_eq!(take(), 0, "layout validation must not allocate");
    let world = World::new(1, params).expect("valid params");
    let used = take() as u64 + parameter_bytes;
    (world, used, estimate)
}

fn assert_estimate_covers_requests(world: &World, used: u64, estimate: u64) {
    assert!(
        used <= estimate,
        "constructor requested {used} bytes but the layout estimated only {estimate}",
    );
    // Stable sort may use stack scratch or request less than a full founder buffer.
    // Pointer-sized constructor indices are charged at 8 bytes even on WASM32.
    let sort_allowance = (world.founder_plan().len() * size_of::<Gene>()) as u64;
    let index_count = world.founder_plan().neuron_count()
        + world.params().brain.hidden_neurons as usize
        + world.founder_plan().len();
    let portable_indices = (index_count * (size_of::<u64>() - size_of::<usize>())) as u64;
    let allowance = sort_allowance + portable_indices;
    assert!(
        estimate - used <= allowance,
        "estimate exceeds measured requests by {} bytes, more than the \
         {allowance}-byte stable-sort and portable-index allowance",
        estimate - used,
    );
}

#[test]
fn a_default_world_stays_inside_its_declared_core_budget() {
    let params = SimParams::default().with_dense_founder();
    let budget = params.storage.max_memory_bytes;
    let (world, used, estimate) = measure(params);
    assert_estimate_covers_requests(&world, used, estimate);
    assert!(
        used <= budget,
        "a default world requests {used} bytes, over its {budget}-byte core budget",
    );
}

#[test]
fn standalone_species_estimate_covers_all_requested_buffers_exactly() {
    use sim_core::species::Classifier;
    let params = SimParams::default().with_dense_founder();
    let bytes = Classifier::estimated_construction_bytes(256, params.storage.max_genes).unwrap();
    take();
    let classifier =
        Classifier::try_new(256, params.storage.max_genes, 0.5, params.distance, bytes).unwrap();
    let requested = take() as u64;
    assert_eq!(requested, bytes);
    assert_eq!(requested, 10_492_936);
    std::hint::black_box(classifier);
}

#[test]
fn world_species_storage_is_charged_without_increasing_the_core_budget() {
    let mut params = SimParams::default().with_dense_founder();
    let classified = params.estimated_construction_bytes().unwrap();
    let species = sim_core::species::Classifier::estimated_construction_bytes(
        params.species.capacity,
        params.storage.max_genes,
    )
    .unwrap();
    assert_eq!(params.storage.max_memory_bytes, 96 * 1024 * 1024);
    params.species.capacity = 0;
    let unclassified = params.estimated_construction_bytes().unwrap();
    assert_eq!(classified - unclassified, species);
    params.storage.max_memory_bytes = classified - 1;
    params.species.capacity = 256;
    assert!(params.validate().is_err());
    assert!(World::new(42, params).is_err());
}

#[test]
fn birth_identity_arrays_fit_the_unchanged_budget_and_legacy_validation_excludes_them() {
    let mut params = SimParams::default().with_dense_founder();
    let current = params.estimated_construction_bytes().unwrap();
    assert_eq!(current, 88_613_076);
    assert_eq!(params.storage.max_memory_bytes, 96 * 1024 * 1024);
    params.storage.max_memory_bytes = current - 3 * 8 * u64::from(params.world.max_agents);
    assert!(params.validate().is_err());
    params
        .validate_for_layout(sim_core::LayoutEra::Species)
        .unwrap();
    assert!(
        World::new(42, params.clone()).is_err(),
        "legacy validation must not authorize a current World"
    );
    params.storage.max_memory_bytes -= 1;
    assert!(
        params
            .validate_for_layout(sim_core::LayoutEra::Species)
            .is_err()
    );
}

#[test]
fn estimate_covers_nondefault_constructor_shapes() {
    type Profile = fn(&mut SimParams);
    let profiles: [Profile; 11] = [
        |p| p.world.max_agents = 2_000,
        |p| {
            p.world.max_agents = 1;
            p.plants.max_plants = 0;
            p.chemo.cells = [1, 1, 1];
            p.sensing.vision_rays = 0;
            p.sensing.chemo_sensors = 0;
            p.sensing.energy_sensors = 0;
            p.brain.hidden_neurons = 0;
            p.brain.oscillators = 0;
        },
        |p| {
            p.sensing.vision_rays = 8;
            p.brain.hidden_neurons = 10;
        },
        |p| {
            p.world.max_agents = 2_000;
            p.storage.genes_per_slot = 500;
            p.storage.neurons_per_slot = 50;
            p.storage.synapses_per_slot = 400;
            p.storage.sensors_per_slot = 10;
            p.storage.effectors_per_slot = 8;
            p.storage.max_genes = 2_048;
            p.storage.max_neurons = 256;
        },
        |p| {
            p.chemo.cells = [192, 96, 1];
            p.chemo.decay = vec![0.98, 0.8, 0.9];
            p.plants.max_plants = 10_000;
            p.sensing.vision_range = 30.0;
            p.sensing.chemo_radius = 20.0;
        },
        |p| p.brain.connections_per_target = Some(1),
        |p| {
            p.brain.connections_per_target = Some(0);
            p.storage.synapses_per_slot = 0;
            p.storage.max_connections = 0;
        },
        |p| {
            p.sensing.vision_rays = 0;
            p.sensing.energy_sensors = 0;
            p.brain.hidden_neurons = 0;
            p.brain.oscillators = 0;
            p.brain.connections_per_target = Some(1);
        },
        |p| {
            p.sensing.vision_rays = 0;
            p.sensing.chemo_sensors = 0;
            p.sensing.energy_sensors = 0;
            p.brain.hidden_neurons = 0;
            p.brain.oscillators = 0;
            p.storage.sensors_per_slot = 0;
            p.storage.synapses_per_slot = 0;
            p.storage.max_sensors = 0;
            p.storage.max_connections = 0;
        },
        |p| {
            p.sensing.vision_rays = 2;
            p.sensing.chemo_sensors = 3;
            p.sensing.energy_sensors = 2;
            p.brain.hidden_neurons = 3;
            p.brain.oscillators = 1;
            p.brain.connections_per_target = Some(2);
        },
        |p| {
            p.brain.hidden_neurons = 64;
            p.brain.connections_per_target = Some(1);
            p.chemo.decay.reserve_exact(15);
        },
    ];
    for profile in profiles {
        let mut params = SimParams::default().with_dense_founder();
        profile(&mut params);
        let (world, used, estimate) = measure(params);
        assert_estimate_covers_requests(&world, used, estimate);
    }
}

#[test]
fn explicit_full_connectivity_has_the_dense_constructor_footprint() {
    let params = SimParams::default().with_dense_founder();
    let (world, dense_used, dense_estimate) = measure(params.clone());
    assert_estimate_covers_requests(&world, dense_used, dense_estimate);
    drop(world);
    for fan_in in [24, u32::MAX] {
        let mut explicit = params.clone();
        explicit.brain.connections_per_target = Some(fan_in);
        let (world, used, estimate) = measure(explicit);
        assert_estimate_covers_requests(&world, used, estimate);
        assert_eq!(used, dense_used);
        assert_eq!(estimate, dense_estimate);
    }
}

#[test]
fn sparse_constructor_fits_its_own_budget_without_charging_dense_edges() {
    let mut params = SimParams::default().with_dense_founder();
    let dense_estimate = params.estimated_construction_bytes().unwrap();
    params.brain.connections_per_target = Some(1);
    let sparse_estimate = params.estimated_construction_bytes().unwrap();
    assert!(sparse_estimate < dense_estimate);
    params.storage.max_memory_bytes = sparse_estimate;
    let (world, used, estimate) = measure(params);
    assert_eq!(estimate, sparse_estimate);
    assert_estimate_covers_requests(&world, used, estimate);
}

#[test]
fn invalid_sparse_template_is_rejected_without_allocation_ids_or_rng() {
    let mut params = SimParams::default().with_dense_founder();
    params.brain.connections_per_target = Some(1);
    params.storage.max_connections = 9;
    let mut rng = Rng::from_seed(42);
    let before = rng.clone();
    let mut requested = 0;
    take();
    let result = FounderPlan::new(&params, &mut rng, || {
        let id = InnovationId::new(requested);
        requested += 1;
        id
    });
    let used = take();
    assert!(result.is_err());
    assert_eq!(used, 0);
    assert_eq!(requested, 0);
    assert_eq!(rng, before);
}

#[test]
fn footprint_scales_with_the_pool_not_the_population() {
    // Doubling capacity should roughly double the commitment, with no dependence on
    // how many agents are actually spawned. If this stops holding, something started
    // allocating lazily and the no-grow guarantee is gone.
    let requests = |agents: u32| {
        let mut params = SimParams::default().with_dense_founder();
        params.world.max_agents = agents;
        let species_bytes = sim_core::species::Classifier::estimated_construction_bytes(
            params.species.capacity,
            params.storage.max_genes,
        )
        .unwrap();
        let (world, used, estimate) = measure(params);
        assert_estimate_covers_requests(&world, used, estimate);
        // Representative capacity is independent of the agent pool.
        used - species_bytes
    };

    let small = requests(1_000);
    let large = requests(2_000);
    let ratio = large as f64 / small as f64;
    assert!(
        (1.8..=2.2).contains(&ratio),
        "capacity doubled but memory changed by {ratio:.2}x"
    );
}
