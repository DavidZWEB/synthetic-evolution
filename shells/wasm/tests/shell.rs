//! The JS boundary, exercised from the target it actually runs on.
//!
//! `sim-core` owns operator correctness. These check boundary integration, including
//! configured edits and observations: that a bad params string is an error rather than a
//! silent fallback, that the snapshot spans describe the buffer they claim to, and that
//! a structural retune is refused at the boundary rather than corrupting a pool JS holds
//! views over.
//!
//! Run with `wasm-pack test --node shells/wasm`.

#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::wasm_bindgen_test;

use sim_core::genome::Gene;
use sim_core::mutate::StructuralMutationCounts;
use sim_core::params::SimParams;
use wasm::{Sim, random_control, validate_params};

fn structural_params() -> SimParams {
    serde_json::from_str(include_str!("../../native/tests/fixtures/structural.json")).unwrap()
}

fn mutations(sim: &Sim) -> StructuralMutationCounts {
    serde_json::from_str(
        &sim.structural_mutation_diagnostics()
            .expect("mutation diagnostics"),
    )
    .expect("valid counters")
}

fn genome(sim: &Sim, slot: u32) -> Vec<Gene> {
    let inspection: serde_json::Value =
        serde_json::from_str(&sim.inspect_agent(slot, 1).expect("live first incarnation")).unwrap();
    serde_json::from_value(inspection["genome"].clone()).unwrap()
}

fn sim(agents: u32) -> Sim {
    let mut sim = Sim::new(7, None).expect("defaults are valid");
    assert_eq!(sim.seed_founders(agents), agents);
    sim
}

fn layout(sim: &Sim) -> serde_json::Value {
    serde_json::from_str(&sim.snapshot_layout().expect("layout serializes")).expect("valid JSON")
}

fn storage(sim: &Sim) -> serde_json::Value {
    serde_json::from_str(&sim.storage_diagnostics().expect("diagnostics serialize"))
        .expect("valid JSON")
}

#[wasm_bindgen_test]
fn a_new_sim_starts_empty_at_tick_zero() {
    let sim = Sim::new(7, None).expect("defaults are valid");
    assert_eq!(sim.tick(), 0);
    assert_eq!(sim.population(), 0);
    assert_eq!(sim.descendants(), 0);
    assert_eq!(sim.mean_energy(), 0.0);
}

#[wasm_bindgen_test]
fn random_control_matches_founders_then_breaks_neural_inheritance() {
    let params = include_str!("../../native/tests/fixtures/sustaining.json").to_owned();
    let mut evolving = Sim::new(7, Some(params.clone())).expect("valid params");
    let mut control = random_control(7, Some(params)).expect("valid params");
    assert_eq!(evolving.seed_founders(8), 8);
    assert_eq!(control.seed_founders(8), 8);
    assert_eq!(
        evolving.state_hash(),
        control.state_hash(),
        "control founders must match"
    );
    assert_eq!(evolving.descendants(), 0);
    assert_eq!(control.descendants(), 0);

    evolving.step_many(100);
    control.step_many(100);
    assert_ne!(
        evolving.state_hash(),
        control.state_hash(),
        "control births did not break neural heredity"
    );
    assert!(evolving.descendants() > 0);
    assert!(control.descendants() > 0);
    assert_eq!(mutations(&evolving), StructuralMutationCounts::default());
    assert_eq!(mutations(&control), StructuralMutationCounts::default());
}

#[wasm_bindgen_test]
fn structural_diagnostics_cover_actual_edits_and_are_isolated_per_world_and_recreation() {
    let params = serde_json::to_string(&structural_params()).unwrap();
    let mut evolving = Sim::new(7, Some(params.clone())).unwrap();
    let mut control = random_control(7, Some(params.clone())).unwrap();
    let zero = StructuralMutationCounts::default();
    for sim in [&mut evolving, &mut control] {
        assert_eq!(sim.seed_founders(1), 1);
        assert_eq!(
            mutations(sim),
            zero,
            "founders are not structurally mutated"
        );
    }
    evolving.step_many(1);
    assert_eq!(
        mutations(&control),
        zero,
        "another world's steps changed the control"
    );
    let observed = mutations(&evolving);
    control.step_many(1);
    for sim in [&evolving, &control] {
        let counts = mutations(sim);
        for operator in [
            counts.remove_connection,
            counts.remove_neuron,
            counts.toggle_connection,
            counts.add_connection,
            counts.add_neuron,
        ] {
            assert_eq!(operator.attempted, 1);
            assert_eq!(operator.applied, 1);
            assert_eq!(
                operator.no_candidate
                    + operator.genome_limit
                    + operator.scratch_limit
                    + operator.innovation_exhausted,
                0
            );
        }
        assert_eq!(sim.population(), 2);
        assert_eq!(sim.descendants(), 1);
        let storage = storage(sim);
        assert_eq!(
            storage.as_object().unwrap().len(),
            2,
            "storage envelope changed"
        );
        assert!(
            storage["spawn_failures"]
                .as_object()
                .unwrap()
                .values()
                .all(|value| value.as_u64() == Some(0))
        );
    }
    let hash = evolving.state_hash();
    let layout = evolving.snapshot_layout().unwrap();
    evolving.step_many(0);
    assert_eq!(mutations(&evolving), observed);
    assert_eq!(evolving.state_hash(), hash);
    assert_eq!(evolving.snapshot_layout().unwrap(), layout);
    let mut recreated = Sim::new(7, Some(params)).unwrap();
    assert_eq!(recreated.seed_founders(1), 1);
    assert_eq!(mutations(&recreated), zero);
    assert_eq!(mutations(&evolving), observed);
}

#[wasm_bindgen_test]
fn splitting_is_inherited_but_scalar_control_redraws_the_resulting_brain() {
    let mut params = structural_params();
    params = params.without_structural_mutation();
    params.mutation.structural.add_neuron_rate = 1.0;
    params.mutation.structural.split_neuron_bias = 0.375;
    let json = serde_json::to_string(&params).unwrap();
    let mut evolving = Sim::new(7, Some(json.clone())).unwrap();
    let mut control = random_control(7, Some(json)).unwrap();
    for sim in [&mut evolving, &mut control] {
        assert_eq!(sim.seed_founders(1), 1);
    }
    let parent = genome(&evolving, 0);
    assert_eq!(genome(&control, 0), parent);
    let non_neural = |genes: &[Gene]| {
        genes
            .iter()
            .copied()
            .filter(|gene| !matches!(gene, Gene::Neuron(_) | Gene::Connection(_)))
            .collect::<Vec<_>>()
    };
    for (sim, is_control) in [(&mut evolving, false), (&mut control, true)] {
        sim.step_many(1);
        assert_eq!(mutations(sim).add_neuron.applied, 1);
        assert_eq!(sim.descendants(), 1);
        assert_eq!(genome(sim, 0), parent, "birth changed the parent");
        let child = genome(sim, 1);
        assert_eq!(child.len(), parent.len() + 3);
        assert_eq!(non_neural(&child), non_neural(&parent));
        let added = child
            .iter()
            .find_map(|gene| match gene {
                Gene::Neuron(neuron)
                    if !parent
                        .iter()
                        .any(|gene| gene.innovation() == Some(neuron.id)) =>
                {
                    Some(neuron)
                }
                _ => None,
            })
            .expect("split added a neuron");
        if is_control {
            assert_ne!(
                added.bias, params.mutation.structural.split_neuron_bias,
                "new neuron escaped scalar redraw"
            );
        } else {
            assert_eq!(added.bias, params.mutation.structural.split_neuron_bias);
        }
        for gene in &child {
            if let Gene::Neuron(neuron) = gene
                && let Some(Gene::Neuron(original)) = parent
                    .iter()
                    .find(|gene| gene.innovation() == Some(neuron.id))
            {
                if is_control {
                    assert_ne!(neuron.tau, original.tau, "inherited scalar tau");
                } else {
                    assert_eq!(
                        neuron, original,
                        "disabled scalar mutation changed a neuron"
                    );
                }
                assert_eq!(neuron.activation, original.activation);
            }
            if let Gene::Connection(connection) = gene
                && let Some(Gene::Connection(original)) = parent
                    .iter()
                    .find(|gene| gene.innovation() == Some(connection.id))
            {
                assert_eq!(
                    (connection.from, connection.to),
                    (original.from, original.to)
                );
                if is_control {
                    assert_ne!(
                        connection.weight, original.weight,
                        "existing connection, including disabled edges, escaped redraw"
                    );
                } else {
                    assert_eq!(connection.weight, original.weight);
                }
            }
        }
        assert_eq!(
            child
                .iter()
                .filter(|gene| matches!(gene, Gene::Connection(c) if !c.enabled))
                .count(),
            1
        );
    }
}

#[wasm_bindgen_test]
fn a_refused_candidate_edit_can_birth_and_an_applied_edit_can_fail_to_birth() {
    let mut params = structural_params();
    params = params.without_structural_mutation();
    params.mutation.structural.add_neuron_rate = 1.0;
    let mut probe = Sim::new(7, Some(serde_json::to_string(&params).unwrap())).unwrap();
    assert_eq!(probe.seed_founders(1), 1);
    let founder_genes = genome(&probe, 0).len() as u32;
    params.storage.max_genes = founder_genes;
    let mut capped = Sim::new(7, Some(serde_json::to_string(&params).unwrap())).unwrap();
    assert_eq!(capped.seed_founders(1), 1);
    capped.step_many(1);
    assert_eq!(mutations(&capped).add_neuron.genome_limit, 1);
    assert_eq!(mutations(&capped).add_neuron.applied, 0);
    assert_eq!(
        capped.descendants(),
        1,
        "a mutation refusal is not a birth refusal"
    );
    assert_eq!(storage(&capped)["spawn_failures"]["genome_limit"], 0);

    params.storage.max_genes = SimParams::default().storage.max_genes;
    params.world.max_agents = 2;
    params.storage.genes_per_slot = founder_genes.div_ceil(2);
    let mut full = Sim::new(7, Some(serde_json::to_string(&params).unwrap())).unwrap();
    assert_eq!(full.seed_founders(1), 1);
    full.step_many(1);
    assert_eq!(mutations(&full).add_neuron.applied, 1);
    assert_eq!(storage(&full)["spawn_failures"]["arena_capacity"], 1);
    assert_eq!(full.population(), 1);
    assert_eq!(
        full.descendants(),
        0,
        "applied edits are not successful births"
    );
}

#[wasm_bindgen_test]
fn disabling_structural_edits_preserves_finite_redraw_on_retained_topology() {
    let mut params = structural_params();
    params = params.without_structural_mutation();
    params.mutation.structural.add_neuron_rate = 1.0;
    let mut sim = random_control(7, Some(serde_json::to_string(&params).unwrap())).unwrap();
    assert_eq!(sim.seed_founders(1), 1);
    sim.step_many(1);
    assert_eq!(mutations(&sim).add_neuron.applied, 1);
    params.mutation.structural.add_neuron_rate = 0.0;
    sim.set_params(&serde_json::to_string(&params).unwrap())
        .unwrap();
    params.brain.weight_init_scale = 3e38;
    sim.set_params(&serde_json::to_string(&params).unwrap())
        .unwrap();
    sim.step_many(1);
    let population = sim.population();
    assert!(
        population > 2,
        "a post-retune scalar-control birth must occur"
    );
    sim.step_many(1);
    for slot in 0..population {
        sim_core::genome::validate(&genome(&sim, slot)).unwrap();
        let inspection: serde_json::Value =
            serde_json::from_str(&sim.inspect_agent(slot, 1).unwrap()).unwrap();
        assert!(
            inspection["activations"]
                .as_array()
                .unwrap()
                .iter()
                .all(|value| value.as_f64().is_some_and(f64::is_finite))
        );
    }
}

#[wasm_bindgen_test]
fn structural_configuration_validates_at_all_boundaries_and_retunes_without_resetting_counts() {
    let mut params = structural_params();
    params = params.without_structural_mutation();
    let canonical = serde_json::to_value(&params).unwrap();
    let mut sim = Sim::new(7, Some(canonical.to_string())).unwrap();
    assert_eq!(sim.seed_founders(1), 1);
    for rate in [
        "remove_connection_rate",
        "remove_neuron_rate",
        "toggle_connection_rate",
        "add_connection_rate",
        "add_neuron_rate",
    ] {
        let mut valid = canonical.clone();
        valid["mutation"]["structural"][rate] = 0.5.into();
        validate_params(Some(valid.to_string())).expect("rate exists and is valid");
        sim.set_params(&valid.to_string())
            .expect("rates can be retuned");
        for value in [-0.1, 1.1] {
            let mut invalid = valid.clone();
            invalid["mutation"]["structural"][rate] = value.into();
            assert!(validate_params(Some(invalid.to_string())).is_err());
            assert!(Sim::new(7, Some(invalid.to_string())).is_err());
            assert!(random_control(7, Some(invalid.to_string())).is_err());
            let hash = sim.state_hash();
            let before = sim.params_json().unwrap();
            assert!(sim.set_params(&invalid.to_string()).is_err());
            assert_eq!(sim.params_json().unwrap(), before);
            assert_eq!(sim.state_hash(), hash);
        }
    }
    for invalid in [
        r#"{"mutation":{"structural":{"split_neuron_bias":1e40}}}"#,
        r#"{"mutation":{"structural":{"split_input_weight":1e40}}}"#,
        r#"{"mutation":{"weight_limit":0.25,"structural":{"add_neuron_rate":1,"split_input_weight":1}}}"#,
        r#"{"brain":{"weight_init_scale":3e38},"mutation":{"structural":{"add_connection_rate":1}}}"#,
    ] {
        assert!(validate_params(Some(invalid.into())).is_err());
        assert!(Sim::new(7, Some(invalid.into())).is_err());
        assert!(random_control(7, Some(invalid.into())).is_err());
        assert!(sim.set_params(invalid).is_err());
    }
    params.mutation.structural.add_neuron_rate = 1.0;
    sim.set_params(&serde_json::to_string(&params).unwrap())
        .unwrap();
    sim.step_many(1);
    assert_eq!(mutations(&sim).add_neuron.applied, 1);
    let counts = mutations(&sim);
    params.mutation.structural.add_neuron_rate = 0.0;
    sim.set_params(&serde_json::to_string(&params).unwrap())
        .unwrap();
    sim.step_many(1);
    assert_eq!(
        mutations(&sim),
        counts,
        "disabling mutation reset or incremented counters"
    );
}

#[wasm_bindgen_test]
fn bad_params_are_an_error_not_a_fallback() {
    // A client that thinks it set a value and did not would be tuning something it
    // cannot see, which is worse than being told no.
    assert!(Sim::new(7, Some("{ not json".into())).is_err());

    // Prove the key parses before trusting what rejecting it means. `SimParams` denies
    // unknown fields, so a misspelled key errors too and this would pass for the wrong
    // reason — which it did, until the name was checked.
    assert!(Sim::new(7, Some(r#"{"plants":{"initial_fill":0.5}}"#.into())).is_ok());
    assert!(Sim::new(7, Some(r#"{"plants":{"initial_fill":2.0}}"#.into())).is_err());

    // And an unknown field is itself an error, so a typo cannot silently do nothing.
    assert!(Sim::new(7, Some(r#"{"nonsense":1}"#.into())).is_err());
    assert!(validate_params(Some(r#"{"nonsense":1}"#.into())).is_err());
    let canonical =
        validate_params(Some(r#"{ "world": { "dt": 0.02 } }"#.into())).expect("valid params");
    let parsed: serde_json::Value = serde_json::from_str(&canonical).expect("canonical JSON");
    assert_eq!(parsed["world"]["dt"], 0.02);
}

#[wasm_bindgen_test]
fn stepping_advances_the_tick_and_the_snapshot() {
    let mut sim = sim(16);
    assert!(sim.mean_energy() > 0.0);
    assert_eq!(layout(&sim)["tick"], 0);
    sim.step_many(25);
    assert_eq!(sim.tick(), 25);
    assert_eq!(layout(&sim)["tick"], 25);
    assert_eq!(layout(&sim)["population"], sim.population());
}

#[wasm_bindgen_test]
fn storage_diagnostics_are_on_demand_cumulative_and_per_world() {
    let params = r#"{"world":{"max_agents":2}}"#.to_owned();
    let mut sim = Sim::new(7, Some(params.clone())).expect("valid params");
    let control = random_control(7, Some(params)).expect("valid params");
    let empty = storage(&sim);
    let arenas = empty["arena_usage"].as_array().expect("arena usage");
    assert_eq!(arenas.len(), 5);
    for (arena, name) in arenas
        .iter()
        .zip(["Genes", "Neurons", "Synapses", "Sensors", "Effectors"])
    {
        assert_eq!(arena["arena"], name);
        assert_eq!(arena["free_elements"], arena["capacity"]);
        assert_eq!(arena["largest_free_block"], arena["capacity"]);
        assert_eq!(arena["live_blocks"], 0);
    }
    assert_eq!(sim.seed_founders(100), 2);
    assert_eq!(
        storage(&sim)["spawn_failures"]["pool_full"],
        0,
        "clamped requests are not attempted refusals"
    );
    for arena in storage(&sim)["arena_usage"].as_array().unwrap() {
        assert_eq!(arena["live_blocks"], 2);
    }
    sim.push_command(
        r#"{"apply_at_tick":0,"kind":{"SpawnFounder":{"position":[500.0,500.0,0.0]}}}"#,
    )
    .unwrap();
    sim.step_many(1);
    let observed = storage(&sim);
    assert_eq!(observed["spawn_failures"]["pool_full"], 1);
    assert_eq!(storage(&control)["spawn_failures"]["pool_full"], 0);
    let hash = sim.state_hash();
    sim.step_many(0);
    assert_eq!(storage(&sim), observed);
    assert_eq!(sim.state_hash(), hash, "sampling changed simulation state");
}

#[wasm_bindgen_test]
fn construction_rejects_low_budgets_and_overflow_without_fallback() {
    let canonical: serde_json::Value =
        serde_json::from_str(&validate_params(None).unwrap()).unwrap();
    assert_eq!(canonical["storage"]["max_memory_bytes"], 96 * 1024 * 1024);
    for params in [
        r#"{"storage":{"max_memory_bytes":1}}"#,
        r#"{"world":{"max_agents":4294967295},"storage":{"max_memory_bytes":18446744073709551615}}"#,
    ] {
        let validation = validate_params(Some(params.into())).expect_err("unsafe layout accepted");
        let message = format!("{:?}", wasm_bindgen::JsValue::from(validation));
        assert!(!message.contains("unknown field"), "{message}");
        assert!(message.contains("invalid"), "{message}");
        assert!(Sim::new(7, Some(params.into())).is_err());
        assert!(random_control(7, Some(params.into())).is_err());
    }
}

#[wasm_bindgen_test]
fn arena_refusals_observe_seeding_and_natural_births_without_hiding_undersupply() {
    let params = r#"{
        "world":{"size":100.0,"max_agents":2},
        "storage":{"genes_per_slot":142},
        "sensing":{"vision_range":20.0,"chemo_radius":20.0,"vision_rays":3,"chemo_sensors":1,"energy_sensors":1},
        "brain":{"hidden_neurons":6,"oscillators":2,"connections_per_target":null},
        "reproduction":{"start_energy":1.0,"threshold":1.1,"gate":0.0,"maturity_ticks":0},
        "feeding":{"rate":100.0,"gate":0.0,"reach":20.0},
        "plants":{"max_plants":100,"max_energy":100.0,"initial_fill":1.0},
        "metabolism":{"base":0.0,"k_size":0.0,"k_brain":0.0,"k_sensor":0.0,"k_move":0.0}
    }"#;
    let mut sim = Sim::new(7, Some(params.into())).expect("one founder fits");
    assert_eq!(sim.seed_founders(2), 1, "undersupply must remain visible");
    let seeded = storage(&sim);
    assert_eq!(seeded["spawn_failures"]["arena_capacity"], 1);
    assert_eq!(seeded["spawn_failures"]["pool_full"], 0);
    assert_eq!(seeded["arena_usage"][0]["capacity"], 284);
    assert_eq!(seeded["arena_usage"][0]["free_elements"], 0);
    assert_eq!(seeded["arena_usage"][0]["live_blocks"], 1);
    sim.step_many(2);
    let stepped = storage(&sim);
    assert!(
        stepped["spawn_failures"]["arena_capacity"]
            .as_u64()
            .unwrap()
            > 1
    );
    assert_eq!(sim.population(), 1);
    assert_eq!(sim.descendants(), 0);
    assert_eq!(layout(&sim)["population"], 1);
}

#[wasm_bindgen_test]
fn the_snapshot_spans_describe_the_buffer_they_claim_to() {
    let sim = sim(8);
    let l = layout(&sim);
    let capacity = l["capacity"].as_u64().expect("capacity");
    assert!(capacity > 0);
    for (field, stride) in [
        ("position", 3),
        ("orientation", 4),
        ("size", 1),
        ("signature", 3),
        ("alive", 1),
        ("species", 1),
        ("part_offset", 1),
        ("part_count", 1),
        ("incarnation", 1),
    ] {
        let span = &l[field];
        assert_eq!(
            span["len"].as_u64().expect(field),
            capacity * stride,
            "{field} is not {stride} per slot"
        );
        assert!(
            span["ptr"].as_u64().expect(field) > 0,
            "{field} has a null pointer"
        );
    }
}

#[wasm_bindgen_test]
fn retuning_goes_through_but_resizing_does_not() {
    let mut sim = sim(4);
    let params: serde_json::Value =
        serde_json::from_str(&sim.params_json().expect("params serialize")).expect("valid JSON");

    let mut tuned = params.clone();
    tuned["metabolism"]["base"] = serde_json::json!(0.02);
    tuned["world"]["dt"] = serde_json::json!(1.0 / 30.0);
    assert!(sim.set_params(&tuned.to_string()).is_ok());
    let hints: serde_json::Value =
        serde_json::from_str(&sim.render_hints().expect("hints serialize")).expect("valid JSON");
    let dt = hints["seconds_per_tick"]
        .as_f64()
        .expect("seconds_per_tick");
    assert!((dt - 1.0 / 30.0).abs() < 1e-6);

    // The pool, the arenas and the snapshot are sized once, and JS holds views over
    // them. Accepting this would detach every one of them (spec §7.3).
    // Same trap as above: `max_agents` has to be a key the parser knows, or this
    // rejects a typo rather than the resize.
    let mut resized = params.clone();
    assert!(
        resized["world"]["max_agents"].is_number(),
        "key name changed"
    );
    resized["world"]["max_agents"] = serde_json::json!(99_999);
    assert!(sim.set_params(&resized.to_string()).is_err());

    let mut storage_retune = params;
    storage_retune["storage"]["max_memory_bytes"] = serde_json::json!(128 * 1024 * 1024);
    validate_params(Some(storage_retune.to_string())).expect("valid construction budget");
    assert!(sim.set_params(&storage_retune.to_string()).is_err());
}

#[wasm_bindgen_test]
fn a_command_crosses_the_boundary_and_applies() {
    let mut sim = Sim::new(7, None).expect("defaults are valid");
    let command = r#"{"apply_at_tick":0,"kind":{"SpawnFounder":{"position":[500.0,500.0,0.0]}}}"#;
    sim.push_command(command).expect("well-formed command");
    assert_eq!(sim.pending_commands(), 1);
    assert_eq!(sim.population(), 0, "applied before its tick ran");

    sim.step_many(1);
    assert_eq!(sim.population(), 1);
    assert_eq!(sim.pending_commands(), 0);

    assert!(sim.push_command("{ not json").is_err());
}

#[wasm_bindgen_test]
fn inspecting_an_agent_returns_its_genome_and_live_activations() {
    let mut sim = sim(4);
    sim.step_many(10);
    let json = sim.inspect_agent(0, 1).expect("slot 0 is alive");
    let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

    assert_eq!(v["index"], 0);
    assert_eq!(v["incarnation"], 1);
    assert_eq!(v["tick"], "10");
    assert!(v["energy"].as_f64().expect("energy") > 0.0);
    assert_eq!(v["age"], 10);
    // Asexual until Phase 6, so the second parent is always the null id (spec §9.1).
    assert_eq!(v["parent_b"], u32::MAX);

    let activations = v["activations"].as_array().expect("activations");
    assert!(!activations.is_empty(), "a brain with no neurons");
    assert!(
        activations.iter().any(|a| a.as_f64() != Some(0.0)),
        "every activation is zero, so this would pass on an empty brain"
    );
    let genome = v["genome"].as_array().expect("genome");
    assert_eq!(
        genome.len(),
        25,
        "slot 0 is still a shipped minimal founder"
    );
    assert!(
        sim.inspect_agent(0, 2).is_err(),
        "a stale slot incarnation inspected its replacement"
    );
}

#[wasm_bindgen_test]
fn inspecting_an_empty_slot_is_an_error() {
    // Returning a plausible-looking zeroed agent would put a ghost in the inspector.
    let sim = sim(2);
    assert!(sim.inspect_agent(50, 1).is_err(), "slot 50 holds nothing");
    assert!(
        sim.inspect_agent(u32::MAX, 1).is_err(),
        "past capacity entirely"
    );
}

#[wasm_bindgen_test]
fn inspector_reports_compensated_energy() {
    let params = r#"{
        "world":{"max_agents":1},
        "feeding":{"rate":0.0},
        "metabolism":{"base":1.0,"k_size":0.0,"k_brain":0.0,"k_sensor":0.0,"k_move":0.0},
        "reproduction":{"start_energy":10000000000.0,"threshold":20000000000.0}
    }"#;
    let mut sim = Sim::new(7, Some(params.into())).expect("valid params");
    assert_eq!(sim.seed_founders(1), 1);
    sim.step_many(1);

    let json = sim.inspect_agent(0, 1).expect("slot 0 is alive");
    let inspection: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(inspection["energy"].as_f64(), Some(9_999_999_999.0));
}

#[wasm_bindgen_test]
fn sensing_retunes_do_not_rebudget_existing_grids() {
    let mut params = sim_core::SimParams::default();
    params.world.max_agents = 2;
    params.plants.max_plants = 0;
    let budget = params.estimated_construction_bytes().unwrap();
    let initial = format!(
        r#"{{"world":{{"max_agents":2}},"plants":{{"max_plants":0}},"storage":{{"max_memory_bytes":{budget}}}}}"#
    );
    let mut sim = Sim::new(42, Some(initial)).unwrap();
    let before = sim.snapshot_layout().unwrap();
    let next = format!(
        r#"{{"world":{{"max_agents":2}},"plants":{{"max_plants":0}},"storage":{{"max_memory_bytes":{budget}}},"sensing":{{"vision_range":50.0}}}}"#
    );
    assert!(wasm::validate_params(Some(next.clone())).is_err());
    sim.set_params(&next)
        .expect("retuning must retain the current grids");
    assert_eq!(sim.snapshot_layout().unwrap(), before);
}

#[wasm_bindgen_test]
fn structural_births_keep_extreme_scalar_biases_finite() {
    let mut params = structural_params();
    params.mutation.neuron_perturb_rate = 1.0;
    params.mutation.bias_perturb_sigma = f32::MAX;
    params = params.without_structural_mutation();
    params.mutation.structural.toggle_connection_rate = 1.0;
    let mut sim = Sim::new(7, Some(serde_json::to_string(&params).unwrap())).unwrap();
    assert_eq!(sim.seed_founders(1), 1);
    sim.step_many(1);
    assert_eq!(sim.descendants(), 1);
    assert_eq!(mutations(&sim).toggle_connection.applied, 1);
    sim_core::genome::validate(&genome(&sim, 1)).unwrap();
}
