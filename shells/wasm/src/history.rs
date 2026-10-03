//! Shell-owned capture and boundary-only marshaling of species history.
//!
//! The shared event wire format is independent of cohorts and persistence; this
//! module neither hashes nor changes World state and never performs storage I/O.

use serde::Serialize;
use sim_core::genome::{self, Gene};
use sim_core::history::Record;
use wasm_bindgen::prelude::*;

use crate::cohort_capture::CohortCapture;
use crate::history_event_wire::{Decimal, EventRecord, RepresentativeRecord};
use crate::{Sim, js_error, parse_params};

#[derive(Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
enum HistoryRecord {
    Event {
        sequence: Decimal,
        tick: Decimal,
        event: EventRecord,
        /// Every origin carries one when representatives are captured; nothing else does.
        #[serde(skip_serializing_if = "Option::is_none")]
        representative: Option<RepresentativeRecord>,
    },
    Gap {
        first_sequence: Decimal,
        last_sequence: Decimal,
    },
}

impl HistoryRecord {
    fn drained(record: Record, capture: &mut CohortCapture) -> Self {
        match record {
            Record::Event { sequence, event } => {
                let tick = Decimal(event.tick);
                let event: EventRecord = event.kind.into();
                let origin = matches!(event, EventRecord::SpeciesOrigin { .. });
                Self::Event {
                    sequence: Decimal(sequence),
                    tick,
                    event,
                    representative: (origin && capture.captures_representatives())
                        .then(|| RepresentativeRecord::staged(capture.claim(sequence))),
                }
            }
            Record::Gap {
                first_sequence,
                last_sequence,
            } => Self::Gap {
                first_sequence: Decimal(first_sequence),
                last_sequence: Decimal(last_sequence),
            },
        }
    }
}

#[derive(Serialize)]
struct Drain {
    records: Vec<HistoryRecord>,
    through_tick: Decimal,
    next_sequence: Decimal,
    dropped_events: Decimal,
    sequence_exhausted: bool,
}

impl Sim {
    fn try_enable_history(
        &mut self,
        capacity: u32,
        representative_genes: Option<u32>,
    ) -> Result<(), String> {
        if self.history_enable_closed {
            return Err("history must be enabled once, before initial seeding or stepping".into());
        }
        let max_genes = self.world.params().storage.max_genes;
        if let Some(genes) = representative_genes
            && genes < max_genes
        {
            return Err(format!(
                "{genes} representative genes cannot hold one maximum-size genome ({max_genes} genes)"
            ));
        }
        let capture = CohortCapture::try_new(capacity, representative_genes)
            .map_err(|error| error.to_string())?;
        self.history = Some(capture);
        self.history_enable_closed = true;
        Ok(())
    }
}

/// Checks an archived representative the way the native reader does: within the
/// archive's `storage.max_genes` and a coherent core genome. Import-time only.
#[wasm_bindgen]
pub fn validate_representative(genes_json: &str, params_json: &str) -> Result<(), JsError> {
    let params = parse_params(Some(params_json))?;
    let genes: Vec<Gene> =
        serde_json::from_str(genes_json).map_err(|error| js_error("representative", error))?;
    if genes.len() > params.storage.max_genes as usize {
        return Err(js_error("representative", "exceeds storage.max_genes"));
    }
    genome::validate(&genes).map_err(|error| js_error("representative", format!("{error:?}")))
}

#[wasm_bindgen]
impl Sim {
    /// Enables a preallocated recorder before any initial seeding or stepping (§3.4).
    /// With `representative_genes`, origins also stage their representative genome in
    /// a buffer of that many genes, which must hold one `storage.max_genes` genome.
    ///
    /// Capture cannot be restarted on the same World, even after it is disabled.
    pub fn enable_history(
        &mut self,
        capacity: u32,
        representative_genes: Option<u32>,
    ) -> Result<(), JsError> {
        self.try_enable_history(capacity, representative_genes)
            .map_err(|error| js_error("history capture", error))
    }

    /// True for queued events/gaps or a latched sequence error, without allocating.
    pub fn history_pending(&self) -> bool {
        self.history.as_ref().is_some_and(|capture| {
            capture.recorder.pending_len() > 0 || capture.recorder.sequence_exhausted()
        })
    }

    /// Drains every available record at the current World boundary, including a final gap.
    ///
    /// Counters are cumulative canonical decimal strings. This allocates only at the
    /// shell boundary and may detach snapshot views (§7.3); it does not hash the World.
    /// Sequence exhaustion remains explicit on every drain until capture is disabled.
    pub fn drain_history(&mut self) -> Result<String, JsError> {
        let capture = self
            .history
            .as_mut()
            .ok_or_else(|| JsError::new("history capture is not enabled"))?;
        let mut records = Vec::new();
        while let Some(record) = capture.recorder.pop() {
            records.push(HistoryRecord::drained(record, capture));
        }
        let drain = Drain {
            records,
            through_tick: Decimal(self.world.tick_count()),
            next_sequence: Decimal(capture.recorder.next_sequence()),
            dropped_events: Decimal(capture.recorder.dropped_events()),
            sequence_exhausted: capture.recorder.sequence_exhausted(),
        };
        serde_json::to_string(&drain).map_err(|error| js_error("history drain", error))
    }

    /// Drops capture buffers outside the tick without changing the simulation.
    ///
    /// Callers that need the remaining prefix must drain at their stop barrier first.
    pub fn disable_history(&mut self) {
        self.history = None;
    }
}

#[cfg(test)]
mod tests {
    //! Native and WASM checks of capture isolation, exact wire values, and boundaries.
    //!
    //! Shared native fixtures exercise the same evolving and scalar-control runs.

    use super::*;
    use serde_json::{Value, json};
    use sim_core::control::BrainInheritance;
    use sim_core::history::{Event, EventKind, Parent};
    use sim_core::ids::{BirthId, SpeciesId};
    use sim_core::params::SimParams;

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test;

    const MODES: [BrainInheritance; 2] = [
        BrainInheritance::Evolving,
        BrainInheritance::RandomizedAtBirth,
    ];

    fn params() -> SimParams {
        let mut params: SimParams =
            serde_json::from_str(include_str!("../../native/tests/fixtures/structural.json"))
                .unwrap();
        params.species.threshold = 1e-12;
        params
    }

    fn make(seed: u64, mode: BrainInheritance) -> Sim {
        Sim::with_brain_inheritance(seed, Some(&serde_json::to_string(&params()).unwrap()), mode)
            .unwrap()
    }

    fn drain(sim: &mut Sim) -> Value {
        let hash = sim.state_hash();
        let snapshot = sim.snapshot_layout().unwrap();
        let drain: Value = serde_json::from_str(&sim.drain_history().unwrap()).unwrap();
        assert_eq!(drain["through_tick"], sim.tick().to_string());
        assert_eq!(drain["sequence_exhausted"], false);
        assert!(!sim.history_pending());
        assert_eq!(sim.state_hash(), hash, "draining changed World/RNG state");
        assert_eq!(sim.snapshot_layout().unwrap(), snapshot);
        drain
    }

    fn check_order(batch: &Value, next: &mut u64) {
        let through = batch["through_tick"]
            .as_str()
            .unwrap()
            .parse::<u64>()
            .unwrap();
        for row in batch["records"].as_array().unwrap() {
            let data = &row["data"];
            assert!(data.get("cohort").is_none());
            match row["kind"].as_str().unwrap() {
                "event" => {
                    assert_eq!(data["sequence"], next.to_string());
                    assert!(data["tick"].as_str().unwrap().parse::<u64>().unwrap() <= through);
                    *next += 1;
                }
                "gap" => {
                    assert_eq!(data["first_sequence"], next.to_string());
                    let last = data["last_sequence"]
                        .as_str()
                        .unwrap()
                        .parse::<u64>()
                        .unwrap();
                    assert!(last >= *next);
                    *next = last + 1;
                }
                kind => panic!("unexpected history row {kind}"),
            }
        }
        assert_eq!(batch["next_sequence"], next.to_string());
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn founders_are_captured_at_tick_zero_before_any_step() {
        for mode in MODES {
            let mut sim = make(7, mode);
            assert!(!sim.history_pending());
            let hash = sim.state_hash();
            sim.enable_history(4, None).unwrap();
            assert_eq!(sim.state_hash(), hash);
            assert!(!sim.history_pending());
            assert_eq!(sim.seed_founders(2), 2);
            assert!(sim.history_pending());
            let batch = drain(&mut sim);
            assert_eq!(batch["records"].as_array().unwrap().len(), 2);
            for (index, row) in batch["records"].as_array().unwrap().iter().enumerate() {
                assert_eq!(
                    row,
                    &json!({
                        "kind": "event",
                        "data": {
                            "sequence": index.to_string(),
                            "tick": "0",
                            "event": {
                                "kind": "species_origin",
                                "species_id": index,
                                "founder_birth_id": index.to_string(),
                                "parent_a": {"status": "absent"},
                                "parent_b": {"status": "absent"}
                            }
                        }
                    })
                );
            }
            assert_eq!(
                drain(&mut sim),
                json!({
                    "records": [],
                    "through_tick": "0",
                    "next_sequence": "2",
                    "dropped_events": "0",
                    "sequence_exhausted": false
                })
            );
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn capture_drain_and_overflow_preserve_hash_rng_and_existing_diagnostics() {
        for seed in [7, 42, 99] {
            for mode in MODES {
                let mut plain = make(seed, mode);
                let mut full = make(seed, mode);
                let mut lossy = make(seed, mode);
                full.enable_history(4096, None).unwrap();
                lossy.enable_history(1, None).unwrap();
                for sim in [&mut plain, &mut full, &mut lossy] {
                    assert_eq!(sim.seed_founders(4), 4);
                }
                let mut next_full = 0;
                let mut next_lossy = 0;
                let mut saw_observed_parent = false;
                for ticks in [0, 1, 3, 8, 20] {
                    for sim in [&mut plain, &mut full, &mut lossy] {
                        sim.step_many(ticks);
                    }
                    assert!(!plain.history_pending());
                    for observed in [&full, &lossy] {
                        assert_eq!(observed.state_hash(), plain.state_hash());
                        assert_eq!(
                            observed.storage_diagnostics().unwrap(),
                            plain.storage_diagnostics().unwrap()
                        );
                        assert_eq!(
                            observed.structural_mutation_diagnostics().unwrap(),
                            plain.structural_mutation_diagnostics().unwrap()
                        );
                        assert_eq!(
                            observed.species_diagnostics().unwrap(),
                            plain.species_diagnostics().unwrap()
                        );
                    }
                    let batch = drain(&mut full);
                    check_order(&batch, &mut next_full);
                    assert_eq!(batch["dropped_events"], "0");
                    saw_observed_parent |= batch["records"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|row| row["data"]["event"]["parent_a"]["status"] == "observed");
                    check_order(&drain(&mut lossy), &mut next_lossy);
                    assert_eq!(next_full, next_lossy);
                }
                assert!(plain.descendants() > 0);
                assert!(
                    saw_observed_parent,
                    "birth history callback was not exercised"
                );
                assert!(lossy.history.as_ref().unwrap().recorder.dropped_events() > 0);
            }
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn overflow_drains_final_gap_then_resumes_in_sequence_at_the_next_boundary() {
        for mode in MODES {
            let mut sim = make(7, mode);
            sim.enable_history(1, None).unwrap();
            assert_eq!(sim.seed_founders(4), 4);
            let first = drain(&mut sim);
            assert_eq!(first["records"].as_array().unwrap().len(), 2);
            assert_eq!(first["records"][0]["data"]["sequence"], "0");
            assert_eq!(
                first["records"][1],
                json!({"kind":"gap","data":{"first_sequence":"1","last_sequence":"3"}})
            );
            assert_eq!(first["dropped_events"], "3");
            let mut next = 0;
            check_order(&first, &mut next);
            sim.step_many(3);
            assert!(sim.history_pending());
            let second = drain(&mut sim);
            assert_eq!(second["through_tick"], "3");
            assert_eq!(second["records"][0]["data"]["sequence"], "4");
            check_order(&second, &mut next);
            assert!(
                second["records"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|row| row["kind"] == "event")
                    .all(|row| row["data"]["tick"]
                        .as_str()
                        .unwrap()
                        .parse::<u64>()
                        .unwrap()
                        < 3)
            );
            let empty = drain(&mut sim);
            assert_eq!(empty["next_sequence"], next.to_string());
            assert_eq!(empty["dropped_events"], second["dropped_events"]);
            assert_eq!(empty["records"], json!([]));
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn extinction_and_command_events_keep_processed_ticks_at_a_later_drain_boundary() {
        for mode in MODES {
            let mut params = params();
            params.feeding.rate = 0.0;
            params.reproduction.threshold = 2.0;
            params.metabolism.base = 10.0;
            let mut sim = Sim::with_brain_inheritance(
                7,
                Some(&serde_json::to_string(&params).unwrap()),
                mode,
            )
            .unwrap();
            sim.enable_history(8, None).unwrap();
            sim.seed_founders(1);
            sim.push_command(r#"{"apply_at_tick":2,"kind":{"SpawnFounder":{"position":[0,0,0]}}}"#)
                .unwrap();
            sim.step_many(4);
            assert_eq!(sim.population(), 0);
            let batch = drain(&mut sim);
            assert_eq!(batch["through_tick"], "4");
            let rows = batch["records"].as_array().unwrap();
            assert_eq!(rows.len(), 4);
            for (index, tick, kind) in [
                (0, "0", "species_origin"),
                (1, "0", "species_extinct"),
                (2, "2", "species_origin"),
                (3, "2", "species_extinct"),
            ] {
                assert_eq!(rows[index]["data"]["tick"], tick);
                assert_eq!(rows[index]["data"]["event"]["kind"], kind);
            }
            let mut next = 0;
            check_order(&batch, &mut next);
            assert_eq!(next, 4);
            sim.step_many(3);
            assert!(
                !sim.history_pending(),
                "idle ticks should not request persistence"
            );
            assert_eq!(
                drain(&mut sim),
                json!({
                    "records": [],
                    "through_tick": "7",
                    "next_sequence": "4",
                    "dropped_events": "0",
                    "sequence_exhausted": false
                })
            );
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn disabling_capture_preserves_the_world_and_cannot_restart_a_partial_archive() {
        for mode in MODES {
            let mut plain = make(7, mode);
            let mut observed = make(7, mode);
            observed.enable_history(1, None).unwrap();
            plain.seed_founders(4);
            observed.seed_founders(4);
            assert!(observed.history_pending());
            let hash = observed.state_hash();
            observed.disable_history();
            observed.disable_history();
            assert!(!observed.history_pending());
            assert!(observed.history.is_none());
            assert_eq!(observed.state_hash(), hash);
            assert!(observed.try_enable_history(4, None).is_err());
            plain.step_many(20);
            observed.step_many(20);
            assert_eq!(plain.state_hash(), observed.state_hash());
            assert_eq!(
                plain.species_diagnostics().unwrap(),
                observed.species_diagnostics().unwrap()
            );
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn capture_admission_rejects_all_prior_seed_step_and_successful_enable_attempts() {
        for mode in MODES {
            for (seed, amount) in [(true, 0), (true, 1), (false, 0), (false, 1)] {
                let mut sim = make(7, mode);
                if seed {
                    sim.seed_founders(amount);
                } else {
                    sim.step_many(amount);
                }
                let hash = sim.state_hash();
                assert!(sim.try_enable_history(4, None).is_err());
                assert_eq!(sim.state_hash(), hash);
                assert!(!sim.history_pending());
            }
            let mut sim = make(7, mode);
            assert!(sim.try_enable_history(0, None).is_err());
            assert!(sim.try_enable_history(u32::MAX, None).is_err());
            assert!(
                !sim.history_enable_closed,
                "failed reservation consumed admission"
            );
            sim.try_enable_history(4, None).unwrap();
            assert!(sim.try_enable_history(4, None).is_err());
            sim.disable_history();
            assert!(sim.try_enable_history(4, None).is_err());
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn exact_decimal_envelope_uses_shared_parent_and_event_encoding() {
        // Representatives off: rows keep the event-only encoding.
        let mut capture = CohortCapture::try_new(1, None).unwrap();
        let drain = Drain {
            records: vec![
                HistoryRecord::drained(
                    Record::Event {
                        sequence: 9_007_199_254_740_993,
                        event: Event {
                            tick: u64::MAX,
                            kind: EventKind::SpeciesOrigin {
                                species_id: SpeciesId::new(1),
                                founder_birth_id: BirthId::new(u64::MAX - 1),
                                parent_a: Parent::Observed {
                                    birth_id: BirthId::new(9_007_199_254_740_993),
                                    species_id: None,
                                },
                                parent_b: Parent::Observed {
                                    birth_id: BirthId::NULL,
                                    species_id: Some(SpeciesId::new(0)),
                                },
                            },
                        },
                    },
                    &mut capture,
                ),
                HistoryRecord::drained(
                    Record::Gap {
                        first_sequence: 9_007_199_254_740_994,
                        last_sequence: u64::MAX - 1,
                    },
                    &mut capture,
                ),
            ],
            through_tick: Decimal(u64::MAX),
            next_sequence: Decimal(u64::MAX),
            dropped_events: Decimal(18_437_736_874_454_810_621),
            sequence_exhausted: true,
        };
        assert_eq!(
            serde_json::to_value(drain).unwrap(),
            json!({
                "records": [
                    {"kind":"event","data":{
                        "sequence":"9007199254740993",
                        "tick":"18446744073709551615",
                        "event":{
                            "kind":"species_origin",
                            "species_id":1,
                            "founder_birth_id":"18446744073709551614",
                            "parent_a":{"status":"observed","birth_id":"9007199254740993","species_id":null},
                            "parent_b":{"status":"observed","birth_id":null,"species_id":0}
                        }
                    }},
                    {"kind":"gap","data":{
                        "first_sequence":"9007199254740994",
                        "last_sequence":"18446744073709551614"
                    }}
                ],
                "through_tick":"18446744073709551615",
                "next_sequence":"18446744073709551615",
                "dropped_events":"18437736874454810621",
                "sequence_exhausted":true
            })
        );
    }

    #[cfg(target_arch = "wasm32")]
    #[wasm_bindgen_test]
    fn invalid_capture_operations_return_js_errors_without_mutation() {
        for mode in MODES {
            let mut sim = make(7, mode);
            let hash = sim.state_hash();
            assert!(sim.drain_history().is_err());
            assert!(sim.enable_history(0, None).is_err());
            assert!(sim.enable_history(u32::MAX, None).is_err());
            assert_eq!(sim.state_hash(), hash);
            sim.enable_history(1, None).unwrap();
            assert!(sim.enable_history(1, None).is_err());
            sim.disable_history();
            assert!(sim.enable_history(1, None).is_err());
            assert!(sim.drain_history().is_err());
            assert_eq!(sim.state_hash(), hash);
        }
    }

    fn origins(batch: &Value) -> Vec<&Value> {
        batch["records"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["data"]["event"]["kind"] == "species_origin")
            .collect()
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn origins_carry_the_classifiers_stored_representative() {
        for mode in MODES {
            let mut plain = make(7, mode);
            let mut observed = make(7, mode);
            observed.enable_history(64, Some(65_536)).unwrap();
            assert_eq!(plain.seed_founders(8), 8);
            assert_eq!(observed.seed_founders(8), 8);
            plain.step_many(20);
            observed.step_many(20);
            assert_eq!(
                observed.state_hash(),
                plain.state_hash(),
                "staging changed the World"
            );
            let batch = drain(&mut observed);
            let origins = origins(&batch);
            assert!(origins.len() >= 8, "every founder originated a species");
            for row in &batch["records"].as_array().unwrap()[..] {
                let origin = row["data"]["event"]["kind"] == "species_origin";
                assert_eq!(row["data"].get("representative").is_some(), origin);
            }
            for row in origins {
                let representative = &row["data"]["representative"];
                assert_eq!(representative["status"], "recorded");
                let genes: Vec<Gene> =
                    serde_json::from_value(representative["genes"].clone()).unwrap();
                let id =
                    SpeciesId::new(row["data"]["event"]["species_id"].as_u64().unwrap() as u32);
                // Extinct species are no longer in the classifier, which is why capture
                // happens at origin rather than by a later lookup (spec §3.4).
                if let Some(live) = observed.world.species().representative(id) {
                    assert_eq!(genes, live);
                }
                validate_representative(
                    &serde_json::to_string(&genes).unwrap(),
                    &serde_json::to_string(&params()).unwrap(),
                )
                .unwrap();
            }
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn full_staging_archives_capture_pressure_and_recovers_after_a_drain() {
        // Genomes may grow to two founders' size, and staging holds exactly one such
        // genome, so it cannot stage all twenty founders seeded below.
        let mut small = params();
        let founder = sim_core::World::new(7, small.clone())
            .unwrap()
            .founder_plan()
            .len() as u32;
        small.storage.max_genes = 2 * founder;
        let max_genes = small.storage.max_genes;
        let mut sim = Sim::with_brain_inheritance(
            7,
            Some(&serde_json::to_string(&small).unwrap()),
            BrainInheritance::Evolving,
        )
        .unwrap();
        sim.enable_history(64, Some(max_genes)).unwrap();
        // Fewer founders than the pool holds, so births can originate species later.
        assert_eq!(sim.seed_founders(20), 20);
        let first = drain(&mut sim);
        let statuses: Vec<_> = origins(&first)
            .iter()
            .map(|row| row["data"]["representative"].clone())
            .collect();
        let recorded = statuses
            .iter()
            .filter(|r| r["status"] == "recorded")
            .count();
        assert!(recorded > 0, "the buffer holds some genomes");
        assert!(
            recorded < statuses.len(),
            "the buffer cannot hold every founder"
        );
        for status in statuses.iter().filter(|r| r["status"] != "recorded") {
            assert_eq!(
                *status,
                json!({"status": "unavailable", "reason": "capture_pressure"})
            );
        }
        let staged_genes: usize = statuses
            .iter()
            .filter_map(|r| r["genes"].as_array())
            .map(Vec::len)
            .sum();
        assert!(staged_genes <= max_genes as usize);
        let mut recovered = None;
        for _ in 0..50 {
            sim.step_many(4);
            let batch = drain(&mut sim);
            if let Some(row) = origins(&batch).first() {
                recovered = Some(row["data"]["representative"]["status"].clone());
                break;
            }
        }
        assert_eq!(
            recovered.expect("a birth originated a species"),
            "recorded",
            "claimed space is reused after a drain"
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn staging_must_hold_one_maximum_size_genome() {
        let mut sim = make(7, BrainInheritance::Evolving);
        let max_genes = params().storage.max_genes;
        assert!(sim.try_enable_history(4, Some(max_genes - 1)).is_err());
        assert!(sim.history.is_none(), "a refused enable leaves capture off");
        sim.try_enable_history(4, Some(max_genes)).unwrap();
    }

    #[cfg(target_arch = "wasm32")]
    #[wasm_bindgen_test]
    fn imported_representatives_are_validated_like_the_native_reader() {
        let mut sim = make(7, BrainInheritance::Evolving);
        sim.enable_history(8, Some(65_536)).unwrap();
        assert_eq!(sim.seed_founders(1), 1);
        let batch = drain(&mut sim);
        let genes = origins(&batch)[0]["data"]["representative"]["genes"].clone();
        let params_json = serde_json::to_string(&params()).unwrap();
        assert!(validate_representative(&genes.to_string(), &params_json).is_ok());
        let mut unsorted: Vec<Value> = genes.as_array().unwrap().clone();
        unsorted.swap(0, 1);
        assert!(validate_representative(&Value::from(unsorted).to_string(), &params_json).is_err());
        let mut small = params();
        small.storage.max_genes = genes.as_array().unwrap().len() as u32 - 1;
        assert!(
            validate_representative(&genes.to_string(), &serde_json::to_string(&small).unwrap())
                .is_err()
        );
    }
}
