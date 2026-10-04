//! Params fields added after archives were first written, read as the values a run
//! that predates each one actually had.
//!
//! Shared so the native history reader and the browser's archive tools read an older
//! archive's params identically. Not a migration: nothing is rewritten, and a field an
//! archive wrote keeps its own value.

use sim_core::params::SimParams;

/// Leaf names an older archive may omit; every other params field must be present.
pub const LATER_PARAMS: &[&str] = &[
    "add_oscillator_rate",
    "grazing_lag",
    "patchiness",
    "patch_scale",
    "death_stock",
    "death_seconds",
    "local_dispersal",
    "dispersal_radius",
    "corpses",
    "k_muscle",
    "k_mouth",
    "body_trait_rate",
    "body_trait_sigma",
    "size_range",
    "muscle_range",
    "mouth_range",
    "combat",
    "founder",
];

/// Serde fills an omitted later field from today's default; this resets it to what the
/// archived run actually had. `wire` is the params object exactly as written.
///
/// Most fields read as zero, because a run that predates them ran without them.
/// Body-trait ranges read as one point at the founders' traits instead: bodies did not
/// evolve before Phase 3, and a zero range would neither describe that nor validate.
///
/// Refuses an archive whose founders bite but which records no `combat`: today's
/// defaults could describe attack rules the run never had.
pub fn restore(params: &mut SimParams, wire: &serde_json::Value) -> Result<(), &'static str> {
    let biting = wire
        .pointer("/founder/bite")
        .and_then(serde_json::Value::as_bool);
    if biting == Some(true) && wire.pointer("/combat").is_none() {
        return Err("an archive whose founders bite must record its combat parameters");
    }
    let absent = |path: &str| wire.pointer(path).is_none();
    if absent("/mutation/structural/add_oscillator_rate") {
        params.mutation.structural.add_oscillator_rate = 0.0;
    }
    if absent("/plants/grazing_lag") {
        params.plants.grazing_lag = 0.0;
    }
    if absent("/plants/patchiness") {
        params.plants.patchiness = 0.0;
    }
    if absent("/plants/patch_scale") {
        params.plants.patch_scale = 0.0;
    }
    if absent("/plants/death_stock") {
        params.plants.death_stock = 0.0;
    }
    if absent("/plants/death_seconds") {
        params.plants.death_seconds = 0.0;
    }
    if absent("/plants/local_dispersal") {
        params.plants.local_dispersal = 0.0;
    }
    if absent("/plants/dispersal_radius") {
        params.plants.dispersal_radius = 0.0;
    }
    // A run from before Phase 3 left no corpses: a zero share reproduces it, and no
    // slots keep its memory ceiling from paying for a pool it never had.
    if absent("/corpses") {
        params.corpses.energy_fraction = 0.0;
        params.corpses.max_corpses = 0;
    }
    if absent("/metabolism/k_muscle") {
        params.metabolism.k_muscle = 0.0;
    }
    if absent("/metabolism/k_mouth") {
        params.metabolism.k_mouth = 0.0;
    }
    if absent("/mutation/body_trait_rate") {
        params.mutation.body_trait_rate = 0.0;
    }
    if absent("/mutation/body_trait_sigma") {
        params.mutation.body_trait_sigma = 0.0;
    }
    // Bodies did not evolve before Phase 3, so a one-point range at the founders'
    // traits is exactly such a run, whatever body.size it used.
    if absent("/body/size_range") {
        params.body.size_range = [params.body.size; 2];
    }
    if absent("/body/muscle_range") {
        params.body.muscle_range = [1.0; 2];
    }
    if absent("/body/mouth_range") {
        params.body.mouth_range = [1.0; 2];
    }
    // Founders could not bite before Phase 3, whatever a later default says. Without a
    // bite no agent swings, so an absent `combat` is inert and keeps today's values.
    if absent("/founder") {
        params.founder.bite = false;
    }
    Ok(())
}
