//! Optional mutation observations shared by neural and organ operators.
//!
//! Counters describe candidate edits, not successful births or simulation state.
//! Legacy organ observations stay unavailable rather than becoming measured zeros.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StructuralOperator {
    RemoveConnection,
    RemoveNeuron,
    ToggleConnection,
    AddConnection,
    AddNeuron,
    AddOscillator,
    RemoveSensor,
    AddSensor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StructuralMutationResult {
    Applied,
    NoCandidate,
    GenomeLimit,
    ScratchLimit,
    InnovationExhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuralMutationEvent {
    pub operator: StructuralOperator,
    pub outcome: StructuralMutationResult,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorCounts {
    pub attempted: u64,
    pub applied: u64,
    pub no_candidate: u64,
    pub genome_limit: u64,
    pub scratch_limit: u64,
    pub innovation_exhausted: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuralMutationCounts {
    pub remove_connection: OperatorCounts,
    pub remove_neuron: OperatorCounts,
    pub toggle_connection: OperatorCounts,
    pub add_connection: OperatorCounts,
    pub add_neuron: OperatorCounts,
    /// Absent before the operator existed, so older observations stay unknown.
    #[serde(default)]
    pub add_oscillator: Option<OperatorCounts>,
    #[serde(default)]
    pub remove_sensor: Option<OperatorCounts>,
    #[serde(default)]
    pub add_sensor: Option<OperatorCounts>,
}

impl Default for StructuralMutationCounts {
    fn default() -> Self {
        Self {
            remove_connection: OperatorCounts::default(),
            remove_neuron: OperatorCounts::default(),
            toggle_connection: OperatorCounts::default(),
            add_connection: OperatorCounts::default(),
            add_neuron: OperatorCounts::default(),
            add_oscillator: Some(OperatorCounts::default()),
            remove_sensor: Some(OperatorCounts::default()),
            add_sensor: Some(OperatorCounts::default()),
        }
    }
}

impl StructuralMutationCounts {
    /// Unknown historical totals remain unknown even if later events are observed.
    pub fn record(&mut self, event: StructuralMutationEvent) {
        let counts = match event.operator {
            StructuralOperator::RemoveConnection => Some(&mut self.remove_connection),
            StructuralOperator::RemoveNeuron => Some(&mut self.remove_neuron),
            StructuralOperator::ToggleConnection => Some(&mut self.toggle_connection),
            StructuralOperator::AddConnection => Some(&mut self.add_connection),
            StructuralOperator::AddNeuron => Some(&mut self.add_neuron),
            StructuralOperator::AddOscillator => self.add_oscillator.as_mut(),
            StructuralOperator::RemoveSensor => self.remove_sensor.as_mut(),
            StructuralOperator::AddSensor => self.add_sensor.as_mut(),
        };
        if let Some(counts) = counts {
            counts.attempted = counts.attempted.saturating_add(1);
            let outcome = match event.outcome {
                StructuralMutationResult::Applied => &mut counts.applied,
                StructuralMutationResult::NoCandidate => &mut counts.no_candidate,
                StructuralMutationResult::GenomeLimit => &mut counts.genome_limit,
                StructuralMutationResult::ScratchLimit => &mut counts.scratch_limit,
                StructuralMutationResult::InnovationExhausted => &mut counts.innovation_exhausted,
            };
            *outcome = outcome.saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_legacy_organ_counts_stay_unknown() {
        let mut encoded = serde_json::to_value(StructuralMutationCounts::default()).unwrap();
        encoded.as_object_mut().unwrap().remove("remove_sensor");
        encoded.as_object_mut().unwrap().remove("add_sensor");
        let mut legacy: StructuralMutationCounts = serde_json::from_value(encoded).unwrap();
        assert_eq!(legacy.add_sensor, None);
        assert_eq!(legacy.remove_sensor, None);
        legacy.record(StructuralMutationEvent {
            operator: StructuralOperator::AddSensor,
            outcome: StructuralMutationResult::Applied,
        });
        assert_eq!(legacy.add_sensor, None);
        let mut current = StructuralMutationCounts::default();
        current.record(StructuralMutationEvent {
            operator: StructuralOperator::AddSensor,
            outcome: StructuralMutationResult::Applied,
        });
        assert_eq!(current.add_sensor.unwrap().applied, 1);
    }
}
