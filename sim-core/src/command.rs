//! `Command`: how anything outside the simulation asks it to change.
//!
//! Every mutation crossing into the sim from a shell — a click that places an agent, a
//! script driving a batch run, eventually a packet from another client — arrives as one
//! of these rather than as a method call. That single choice is what makes replay, batch
//! scripting, and the multi-client path of spec §9.5 fall out for free instead of being
//! three separate retrofits (spec §2.2b).
//!
//! The hedge is the **queue and its stamp**, not the vocabulary. Phase 1 has one thing a
//! shell can ask for, and inventing the others now would be guessing at a UI nobody has
//! built; adding a variant later is additive and costs nothing (spec §9.2).
//!
//! Deliberately not here: internal mutation. `World::spawn_founder` and the tick's own
//! systems stay ordinary Rust. This is the boundary crossing, not a rule that the crate
//! may not call its own methods.

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// One request, stamped with the tick it takes effect on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Command {
    /// The tick this applies on.
    ///
    /// A command whose tick has already passed applies at the next opportunity rather
    /// than being dropped. Dropping would make the result depend on how far the sim had
    /// run when the message arrived, which is wall-clock timing leaking into a
    /// deterministic system — the same run replayed on a slower machine would diverge.
    pub apply_at_tick: u64,
    pub kind: Kind,
}

/// What a command asks for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Kind {
    /// Place a founder at a position, as `World::spawn_founder` would.
    ///
    /// Refused silently when the pool is full: at the population ceiling a spawn is a
    /// normal failure rather than an error, and a command queue that could fail would
    /// need a reply channel that nothing has asked for yet.
    SpawnFounder { position: Vec3 },
}

impl Command {
    /// A command for the next tick that runs.
    pub fn now(kind: Kind) -> Self {
        Self {
            apply_at_tick: 0,
            kind,
        }
    }

    /// A command stamped for a specific tick.
    pub fn at(apply_at_tick: u64, kind: Kind) -> Self {
        Self {
            apply_at_tick,
            kind,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::SimParams;
    use crate::world::World;

    fn world() -> World {
        let mut params = SimParams::default();
        params.world.max_agents = 8;
        params.plants.max_plants = 8;
        World::new(3, params).expect("defaults are valid")
    }

    fn spawn_at(tick: u64, x: f32) -> Command {
        Command::at(
            tick,
            Kind::SpawnFounder {
                position: Vec3::new(x, 500.0, 0.0),
            },
        )
    }

    #[test]
    fn a_command_waits_for_the_tick_it_is_stamped_for() {
        let mut world = world();
        world.push_command(spawn_at(3, 100.0));
        // A step *runs* the tick it begins on, so the step that applies a command
        // stamped for tick 3 is the fourth: it opens with the clock already at 3.
        for _ in 0..3 {
            world.step();
            assert_eq!(world.population(), 0, "applied before its tick");
        }
        assert_eq!(world.tick_count(), 3);
        world.step();
        assert_eq!(world.population(), 1);
        assert_eq!(world.pending_commands(), 0);
    }

    #[test]
    fn a_command_stamped_for_the_past_still_runs() {
        // Dropping it would make the outcome depend on how far the sim had got when the
        // message arrived, which is wall-clock timing leaking into a deterministic
        // system: the same run replayed on a slower machine would diverge.
        let mut world = world();
        for _ in 0..10 {
            world.step();
        }
        world.push_command(spawn_at(2, 100.0));
        assert_eq!(world.population(), 0);
        world.step();
        assert_eq!(world.population(), 1);
    }

    #[test]
    fn commands_apply_in_submission_order() {
        // Two founders in one tick take two pool slots, and which one gets slot 0
        // decides every later free-list handout. Submission order is the only order a
        // serialized queue can promise.
        // `apply_commands` rather than `step`, because a full tick would then move both
        // agents and the assertion would be reading movement rather than order.
        let order = |first: f32, second: f32| {
            let mut world = world();
            world.push_command(spawn_at(0, first));
            world.push_command(spawn_at(0, second));
            world.apply_commands();
            (world.agents().position[0].x, world.agents().position[1].x)
        };
        assert_eq!(order(100.0, 200.0), (100.0, 200.0));
        assert_eq!(order(200.0, 100.0), (200.0, 100.0));
    }

    #[test]
    fn a_full_pool_refuses_a_spawn_without_failing() {
        let mut world = world();
        for i in 0..12 {
            world.push_command(spawn_at(0, 100.0 + i as f32));
        }
        world.step();
        assert_eq!(world.population(), 8, "pool holds 8");
        assert_eq!(world.pending_commands(), 0, "refused commands should not requeue");
    }

    #[test]
    fn a_pending_queue_is_part_of_the_world_s_identity() {
        // Two worlds alike in everything but what is queued diverge on the tick the
        // queue comes due, so the hash has to separate them before that (spec §7.8).
        let mut queued = world();
        queued.push_command(spawn_at(50, 100.0));
        assert_ne!(world().state_hash(), queued.state_hash());
    }

    #[test]
    fn a_command_survives_a_serde_round_trip() {
        // The whole point of the enum: a command has to cross a process boundary intact
        // for replay and for the multi-client path (spec §2.2b, §9.5).
        let command = spawn_at(7, 123.5);
        let json = serde_json::to_string(&command).expect("serializes");
        assert_eq!(
            serde_json::from_str::<Command>(&json).expect("round-trips"),
            command
        );
    }
}
