//! Bounded staging of representative genomes captured with their origin events.
//!
//! Shells copy each new species' representative here from the history callback and
//! claim it when the matching origin record drains. It never allocates after
//! construction and never looks genomes up later: a full buffer makes that
//! representative unavailable, not deferred (spec §3.4).

use crate::genome::Gene;

use super::BuildError;

#[derive(Clone, Copy, Debug)]
struct Entry {
    sequence: u64,
    start: u32,
    len: u32,
}

#[derive(Debug)]
pub struct RepresentativeBuffer {
    genes: Vec<Gene>,
    gene_capacity: u32,
    entries: Vec<Entry>,
    entry_capacity: u32,
    cursor: usize,
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
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(genome_capacity as usize)
            .map_err(BuildError::Reservation)?;
        Ok(Self {
            genes,
            gene_capacity,
            entries,
            entry_capacity: genome_capacity,
            cursor: 0,
        })
    }

    pub fn gene_capacity(&self) -> u32 {
        self.gene_capacity
    }

    /// Copies `genes` for the origin event at `sequence`, or returns false when either
    /// bound would be exceeded. Sequences must be stored in increasing order.
    pub fn store(&mut self, sequence: u64, genes: &[Gene]) -> bool {
        self.reclaim();
        debug_assert!(
            self.entries
                .last()
                .is_none_or(|last| last.sequence < sequence),
            "representatives are staged in capture order"
        );
        let start = self.genes.len();
        if self.entries.len() == self.entry_capacity as usize
            || genes.len() > self.gene_capacity as usize - start
        {
            return false;
        }
        // Within the reserved capacity, so neither push reallocates.
        self.genes.extend_from_slice(genes);
        self.entries.push(Entry {
            sequence,
            start: start as u32,
            len: genes.len() as u32,
        });
        true
    }

    /// The genome staged for `sequence`, discarding older entries that were never
    /// claimed. `None` means it was never staged, not that it can be found elsewhere.
    pub fn take(&mut self, sequence: u64) -> Option<&[Gene]> {
        self.reclaim();
        while self
            .entries
            .get(self.cursor)
            .is_some_and(|entry| entry.sequence < sequence)
        {
            self.cursor += 1;
        }
        let entry = *self.entries.get(self.cursor)?;
        if entry.sequence != sequence {
            return None;
        }
        self.cursor += 1;
        let start = entry.start as usize;
        Some(&self.genes[start..start + entry.len as usize])
    }

    /// Frees all space once every staged genome has been claimed or skipped.
    fn reclaim(&mut self) {
        if self.cursor == self.entries.len() {
            self.entries.clear();
            self.genes.clear();
            self.cursor = 0;
        }
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
            !buffer.store(4, &genome(1)),
            "space returns only after every claim"
        );
        assert_eq!(buffer.take(2), Some(&genome(1)[..]));
        assert!(buffer.store(4, &genome(4)));
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
