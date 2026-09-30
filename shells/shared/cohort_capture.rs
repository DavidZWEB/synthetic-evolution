//! One cohort's bounded history capture, shared by the native and WASM shells.
//!
//! Pairs the event recorder with optional representative staging so both shells
//! stage and claim origin genomes identically. It owns no wire format, persistence,
//! or I/O, and never changes World state (spec §3.4).

use sim_core::genome::Gene;
use sim_core::history::{BuildError, Capture, Event, Recorder, RepresentativeBuffer};

pub(crate) struct CohortCapture {
    pub recorder: Recorder,
    representatives: Option<RepresentativeBuffer>,
}

impl CohortCapture {
    pub fn try_new(capacity: u32, representative_genes: Option<u32>) -> Result<Self, BuildError> {
        Ok(Self {
            recorder: Recorder::try_new(capacity)?,
            // A recorder holds at most `capacity` undrained origins, so staging never
            // needs more genome slots than that.
            representatives: representative_genes
                .map(|genes| RepresentativeBuffer::try_new(genes, capacity))
                .transpose()?,
        })
    }

    /// History callback: queue the event and stage an origin's representative.
    pub fn record(&mut self, event: Event, representative: Option<&[Gene]>) {
        // Recorder latches exhaustion; the shell reports it outside the tick, never
        // panicking or doing I/O from a callback.
        if self.recorder.record(event) == Ok(Capture::Recorded)
            && let (Some(buffer), Some(genes)) = (&mut self.representatives, representative)
        {
            // A refusal leaves nothing staged, so the drain archives it as unavailable.
            let _ = buffer.store(self.recorder.next_sequence() - 1, genes);
        }
    }

    pub fn captures_representatives(&self) -> bool {
        self.representatives.is_some()
    }

    /// For a drained origin, the genome staged at `sequence`; `None` when staging
    /// refused it or representative capture is off.
    pub fn claim(&mut self, sequence: u64) -> Option<&[Gene]> {
        self.representatives.as_mut()?.take(sequence)
    }
}
