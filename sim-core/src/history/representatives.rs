//! Bounded staging of representative genomes captured with their origin events.
//!
//! Shells copy each new species' representative here from the history callback and
//! claim it when the matching origin record drains, in capture order. It never allocates after
//! construction and never looks genomes up later: a full buffer makes that
//! representative unavailable, not deferred (spec §3.4).

use std::collections::VecDeque;

use crate::genome::Gene;

use super::BuildError;

#[derive(Clone, Copy, Debug)]
struct Entry {
    sequence: u64,
    start: u32,
    len: u32,
}

impl Entry {
    fn end(self) -> u32 {
        self.start + self.len
    }
}

/// A FIFO ring of contiguous genomes. Claiming the oldest genome frees its space
/// immediately, so partial drains never strand capacity behind unclaimed entries.
#[derive(Debug)]
pub struct RepresentativeBuffer {
    genes: Vec<Gene>,
    entries: VecDeque<Entry>,
    entry_capacity: u32,
}

impl RepresentativeBuffer {
    /// Reserves `gene_capacity` genes shared by at most `genome_capacity` pending genomes.
    pub fn try_new(gene_capacity: u32, genome_capacity: u32) -> Result<Self, BuildError> {
        if gene_capacity == 0 || genome_capacity == 0 {
            return Err(BuildError::ZeroCapacity);
        }
        let too_large = |count: u32, size: usize| {
            u64::from(count)
                .checked_mul(size as u64)
                .is_none_or(|bytes| bytes > i32::MAX as u64)
        };
        if too_large(gene_capacity, size_of::<Gene>())
            || too_large(genome_capacity, size_of::<Entry>())
        {
            return Err(BuildError::CapacityTooLarge);
        }
        let mut genes = Vec::new();
        genes
            .try_reserve_exact(gene_capacity as usize)
            .map_err(BuildError::Reservation)?;
        genes.resize(gene_capacity as usize, Gene::default());
        let mut entries = VecDeque::new();
        entries
            .try_reserve_exact(genome_capacity as usize)
            .map_err(BuildError::Reservation)?;
        Ok(Self {
            genes,
            entries,
            entry_capacity: genome_capacity,
        })
    }

    pub fn gene_capacity(&self) -> u32 {
        self.genes.len() as u32
    }

    /// Copies `genes` for the origin event at `sequence`, or returns false when either
    /// bound would be exceeded. Sequences must be stored in increasing order.
    pub fn store(&mut self, sequence: u64, genes: &[Gene]) -> bool {
        debug_assert!(
            self.entries
                .back()
                .is_none_or(|last| last.sequence < sequence),
            "representatives are staged in capture order"
        );
        if self.entries.len() == self.entry_capacity as usize {
            return false;
        }
        let Some(start) = self.place(genes.len()) else {
            return false;
        };
        let len = genes.len() as u32;
        self.genes[start as usize..(start + len) as usize].copy_from_slice(genes);
        // Within the reserved capacity, so the push never reallocates.
        self.entries.push_back(Entry {
            sequence,
            start,
            len,
        });
        true
    }

    /// Where a contiguous genome of `len` genes fits without overlapping live ones.
    fn place(&self, len: usize) -> Option<u32> {
        let capacity = self.genes.len();
        let (Some(oldest), Some(newest)) = (self.entries.front(), self.entries.back()) else {
            return (len <= capacity).then_some(0);
        };
        let (oldest, end) = (oldest.start as usize, newest.end() as usize);
        if newest.start as usize >= oldest {
            // Live genomes are one run: free space follows it and precedes it.
            if len <= capacity - end {
                Some(end as u32)
            } else if len <= oldest {
                Some(0)
            } else {
                None
            }
        } else {
            // The newest genome wrapped to the front; only the gap before the oldest is free.
            (len <= oldest - end).then_some(end as u32)
        }
    }

    /// The genome staged for `sequence`, discarding older entries that were never
    /// claimed. `None` means it was never staged, not that it can be found elsewhere.
    pub fn take(&mut self, sequence: u64) -> Option<&[Gene]> {
        while self
            .entries
            .front()
            .is_some_and(|entry| entry.sequence < sequence)
        {
            self.entries.pop_front();
        }
        if self.entries.front()?.sequence != sequence {
            return None;
        }
        let entry = self.entries.pop_front()?;
        Some(&self.genes[entry.start as usize..entry.end() as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::NeuronGene;
    use crate::ids::InnovationId;

    fn genome(len: u32) -> Vec<Gene> {
        (0..len)
            .map(|id| {
                Gene::Neuron(NeuronGene {
                    id: InnovationId::new(id),
                    ..Default::default()
                })
            })
            .collect()
    }

    #[test]
    fn staged_genomes_are_claimed_by_sequence_and_skipped_entries_are_discarded() {
        let mut buffer = RepresentativeBuffer::try_new(8, 3).unwrap();
        assert!(buffer.store(2, &genome(2)));
        assert!(buffer.store(5, &genome(3)));
        assert!(buffer.store(9, &genome(1)));
        assert_eq!(buffer.take(1), None, "never staged");
        assert_eq!(buffer.take(5), Some(&genome(3)[..]), "skips 2");
        assert_eq!(
            buffer.take(2),
            None,
            "skipped entries cannot be claimed later"
        );
        assert_eq!(buffer.take(9), Some(&genome(1)[..]));
        assert_eq!(buffer.take(9), None, "claimed at most once");
        assert!(
            buffer.store(10, &genome(8)),
            "space reclaimed after a full drain"
        );
    }

    #[test]
    fn bounds_refuse_rather_than_evict_or_grow() {
        let mut buffer = RepresentativeBuffer::try_new(4, 2).unwrap();
        assert!(buffer.store(0, &genome(3)));
        assert!(!buffer.store(1, &genome(2)), "gene bound");
        assert!(buffer.store(2, &genome(1)));
        assert!(!buffer.store(3, &[]), "genome bound");
        assert_eq!(buffer.take(0), Some(&genome(3)[..]));
        assert!(
            buffer.store(4, &genome(3)),
            "claimed space returns immediately"
        );
        assert!(!buffer.store(5, &[]), "genome bound again");
        assert_eq!(buffer.take(2), Some(&genome(1)[..]));
        assert_eq!(buffer.take(4), Some(&genome(3)[..]));
        assert!(buffer.store(5, &genome(4)));
    }

    #[test]
    fn partial_drains_free_claimed_space_and_wrap_without_overlap() {
        let mut buffer = RepresentativeBuffer::try_new(10, 3).unwrap();
        assert!(buffer.store(0, &genome(4)));
        assert!(buffer.store(1, &genome(4)));
        assert!(buffer.store(2, &genome(2)));
        assert!(
            !buffer.store(3, &genome(1)),
            "genome bound while all are pending"
        );
        assert_eq!(buffer.take(0), Some(&genome(4)[..]));
        // Claiming one frees its slot and its genes, while 1 and 2 are still queued.
        assert!(
            buffer.store(3, &genome(3)),
            "wraps into the space genome 0 freed"
        );
        assert!(!buffer.store(4, &genome(2)), "would overlap genome 1");
        assert_eq!(buffer.take(1), Some(&genome(4)[..]));
        assert!(buffer.store(4, &genome(2)));
        assert_eq!(buffer.take(2), Some(&genome(2)[..]));
        assert_eq!(buffer.take(3), Some(&genome(3)[..]));
        assert_eq!(buffer.take(4), Some(&genome(2)[..]));
        assert!(
            buffer.store(5, &genome(10)),
            "an empty ring offers its whole capacity"
        );
    }

    #[test]
    fn construction_rejects_zero_and_unportable_bounds() {
        assert!(matches!(
            RepresentativeBuffer::try_new(0, 1),
            Err(BuildError::ZeroCapacity)
        ));
        assert!(matches!(
            RepresentativeBuffer::try_new(1, 0),
            Err(BuildError::ZeroCapacity)
        ));
        assert!(matches!(
            RepresentativeBuffer::try_new(u32::MAX, 1),
            Err(BuildError::CapacityTooLarge)
        ));
    }
}
