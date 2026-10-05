//! Params fields added after archives were first written, read as the values a run
//! that predates each one actually had.
//!
//! Shared so the native history reader and the browser's archive tools read an older
//! archive's params identically. Not a migration: nothing is rewritten, and a field an
//! archive wrote keeps its own value.

use sim_core::params::{CombatParams, SimParams};

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

/// A present `founder` section must record `bite`, and a present `combat` section every
/// combat parameter, because serde would fill a missing one from today's defaults,
/// which could describe a run that never had them. An archive whose founders bite must
/// record its combat. One written before the bite may omit both sections. `wire` is
/// the params object exactly as written.
pub fn require_complete_bite_params(wire: &serde_json::Value) -> Result<(), &'static str> {
    let bite = match wire.get("founder") {
        None => None,
        Some(founder) => Some(
            founder
                .get("bite")
                .and_then(serde_json::Value::as_bool)
                .ok_or("a recorded founder section must record bite")?,
        ),
    };
    let Some(combat) = wire.get("combat") else {
        return if bite == Some(true) {
            Err("an archive whose founders bite must record its combat parameters")
        } else {
            Ok(())
        };
    };
    let recorded = combat.as_object();
    let fields = serde_json::to_value(CombatParams::default()).ok();
    let complete = fields
        .as_ref()
        .and_then(serde_json::Value::as_object)
        .is_some_and(|fields| {
            fields
                .keys()
                .all(|field| recorded.is_some_and(|combat| combat.contains_key(field)))
        });
    if complete {
        Ok(())
    } else {
        Err("a recorded combat section must record every combat parameter")
    }
}

/// Serde fills an omitted later field from today's default; this resets it to what the
/// archived run actually had. `wire` is the params object exactly as written.
///
/// Most fields read as zero, because a run that predates them ran without them.
/// Body-trait ranges read as one point at the founders' traits instead: bodies did not
/// evolve before Phase 3, and a zero range would neither describe that nor validate.
///
/// Refuses incomplete bite sections, as [`require_complete_bite_params`] describes.
pub fn restore(params: &mut SimParams, wire: &serde_json::Value) -> Result<(), &'static str> {
    require_complete_bite_params(wire)?;
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
