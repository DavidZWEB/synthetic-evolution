//! The JS boundary, exercised from the target it actually runs on.
//!
//! These do not check the simulation — `sim-core` owns that and tests it natively. What
//! they check is the marshalling: that a bad params string is an error rather than a
//! silent fallback, that the snapshot spans describe the buffer they claim to, and that
//! a structural retune is refused at the boundary rather than corrupting a pool JS holds
//! views over.
//!
//! Run with `wasm-pack test --node shells/wasm`.

#![cfg(target_arch = "wasm32")]

use wasm_bindgen_test::wasm_bindgen_test;

use wasm::{Sim, validate_params};

fn sim(agents: u32) -> Sim {
    let mut sim = Sim::new(7, None).expect("defaults are valid");
    assert_eq!(sim.seed_founders(agents), agents);
    sim
}

fn layout(sim: &Sim) -> serde_json::Value {
    serde_json::from_str(&sim.snapshot_layout().expect("layout serializes")).expect("valid JSON")
}

#[wasm_bindgen_test]
fn a_new_sim_starts_empty_at_tick_zero() {
    let sim = Sim::new(7, None).expect("defaults are valid");
    assert_eq!(sim.tick(), 0);
    assert_eq!(sim.population(), 0);
    assert_eq!(sim.mean_energy(), 0.0);
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
    assert_eq!(v["tick"], 10);
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
    assert!(genome.len() > 100, "the default topology is 284 genes");
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
