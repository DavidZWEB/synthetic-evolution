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

/// An archive records `founder` and `combat` together, as every writer since the bite
/// does, or neither, as every writer before it did. Founders that could not bite do not
/// make a world biteless, because an imported genome may carry a bite, so only an
/// archive from before the bite existed reads with no combat. A recorded `founder` must
/// record `bite`, and a recorded `combat` every combat parameter, because serde would
/// fill a missing one from today's defaults, which could describe a run that never had
/// them. `wire` is the params object exactly as written.
pub fn require_complete_bite_params(wire: &serde_json::Value) -> Result<(), &'static str> {
    let (founder, combat) = match (wire.get("founder"), wire.get("combat")) {
        (None, None) => return Ok(()),
        (Some(founder), Some(combat)) => (founder, combat),
        _ => return Err("an archive records founder and combat params together, or neither"),
    };
    if !founder
        .get("bite")
        .is_some_and(serde_json::Value::is_boolean)
    {
        return Err("a recorded founder section must record bite");
    }
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

/// Combat as a run from before the bite had it: founders that could not bite, so every
/// combat number reads as zero. Today's defaults are not inert even there, because
/// validation measures them against the run's other params: a half-second cooldown is
/// uncountable at a tiny enough timestep, and a default mouthful overflows a wide
/// enough mouth range. Zero is valid against any params.
pub fn no_combat() -> CombatParams {
    CombatParams {
        gate: 0.0,
        reach: 0.0,
        arc: 0.0,
        attack_cost: 0.0,
        attack_damage: 0.0,
        cooldown_seconds: 0.0,
        health_regen: 0.0,
        dormant_bias: 0.0,
        mouthful: 0.0,
        assimilation: 0.0,
    }
}

/// Serde fills an omitted later field from today's default; this resets it to what the
/// archived run actually had. `wire` is the params object exactly as written.
///
/// Most fields read as zero, because a run that predates them ran without them; an
/// absent `combat` reads as [`no_combat`]. Body-trait ranges read as one point at the
/// founders' traits instead: bodies did not evolve before Phase 3, and a zero range
/// would neither describe that nor validate.
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
    // Founders could not bite before Phase 3, whatever a later default says.
    if absent("/founder") {
        params.founder.bite = false;
    }
    if absent("/combat") {
        params.combat = no_combat();
    }
    Ok(())
}
