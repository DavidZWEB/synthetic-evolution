//! The genome: a list of typed genes, and the rules that make one coherent.
//!
//! Not a flat weight vector. Sensors, effectors, neurons, connections, and body
//! traits all evolve through the same machinery precisely because they are the same
//! kind of thing — a gene in a list (spec §3.1). Extensions stay additive: a new
//! organ is a new gene *kind*, never a new fixed field on an agent.
//!
//! Two invariants hold for every genome in the world, both checked by
//! [`validate`]:
//!
//! 1. **Genes are sorted** by [`Gene::sort_key`], neurons first. Crossover aligns two
//!    genomes by innovation id in a single linear walk, which needs sorted input, and
//!    neurons-first means a reader has seen every neuron before any gene that
//!    references one.
//! 2. **Every reference resolves.** A sensor's target, an effector's source, and a
//!    connection's endpoints all name neurons that exist in the same genome.
//!
//! Deliberately not here: mutation (`mutate`), recombination (`crossover`), the
//! founding topology (`founder`), and evaluation (`brain`). This module is the
//! representation and its rules.

use serde::{Deserialize, Serialize};

use crate::ids::InnovationId;

/// Parameter slots on a sensor or effector gene.
///
/// Fixed-size, not a `Vec`: a genome is copied on every birth, and a heap allocation
/// per gene would allocate in the tick (CLAUDE.md invariant 4). Four covers the widest
/// entry in the spec §4.1/§4.2 catalog — `vision_ray`'s azimuth, elevation, range, fov.
pub const GENE_PARAMS: usize = 4;

/// Values one sensor can write into the brain per tick.
///
/// Four covers `vision_ray`, the widest: distance plus the signature RGB of what it
/// hit (spec §2.2c).
pub const SENSOR_CHANNELS: usize = 4;

/// What a neuron does with its accumulated input.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Activation {
    #[default]
    Sigmoid,
    Tanh,
    /// A free-running oscillator on its own period, ignoring input.
    ///
    /// Always present in small numbers as a scaffold: evolution finds them fast and
    /// builds gaits, patience, and rhythmic signalling on top (spec §3.2).
    Oscillator,
}

/// Sensor modalities available in Phase 1. The catalog in spec §4.1 is larger; the
/// rest arrive with the phases that need them, as new variants.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Modality {
    /// Params `[azimuth, elevation, range, fov]`. Returns distance and the signature
    /// RGB of the first hit.
    #[default]
    VisionRay,
    /// Params `[channel, radius, _, _]`. Returns concentration and gradient x/y.
    Chemo,
    /// Params `[which, _, _, _]`. Returns one of the agent's own scalars.
    Interoception,
}

impl Modality {
    /// How many channels this modality writes.
    pub const fn channels(self) -> usize {
        match self {
            Modality::VisionRay => 4,
            Modality::Chemo => 3,
            Modality::Interoception => 1,
        }
    }
}

/// Effector actions available in Phase 1. Spec §4.2 lists more.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Action {
    #[default]
    Thrust,
    Turn,
    Ingest,
    /// Brain-gated, not fired at an energy threshold — the agent decides when, which
    /// is what makes life-history strategy evolvable (spec §4.2).
    Reproduce,
}

/// Body traits carried genetically rather than as fixed agent fields.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum BodyTrait {
    #[default]
    Size,
    /// The displayed colour `vision_ray` returns. Once `set_signature` exists this is
    /// the channel aposematism, crypsis, and mimicry all evolve on (spec §4.2).
    SignatureR,
    SignatureG,
    SignatureB,
}

/// Traits governing the genome's own evolution. Present and inherited in Phase 1; no
/// operator mutates them yet, the same treatment sensor `elevation` gets (spec §3.3).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum MetaTrait {
    #[default]
    MutationRate,
    WeightSigma,
    CrossoverRate,
}

#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct NeuronGene {
    pub id: InnovationId,
    pub bias: f32,
    /// Time constant. Small reacts, large integrates; the spread across a brain is
    /// what gives it memory of any length (spec §3.2).
    pub tau: f32,
    pub activation: Activation,
    /// Period in ticks. Only read when `activation` is [`Activation::Oscillator`].
    pub period: f32,
}

#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct ConnectionGene {
    pub id: InnovationId,
    pub from: InnovationId,
    pub to: InnovationId,
    pub weight: f32,
    /// Disabled connections stay in the genome so the innovation id survives for
    /// alignment, and so re-enabling is a mutation rather than a re-invention.
    pub enabled: bool,
}

#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct SensorGene {
    pub id: InnovationId,
    pub modality: Modality,
    /// Directional params are `(azimuth, elevation)` pairs with elevation clamped to
    /// 0 and no operator that varies it. The slot exists in the serialized genome so
    /// going 3D is an unclamping rather than a format break (spec §4.1, §9.1).
    pub params: [f32; GENE_PARAMS],
    /// One target neuron per channel, bound **by id**, never by index.
    ///
    /// The spec writes this as a single `target`, but §2.2c has modalities returning
    /// two to four values, so one id per channel is what "bind by ID" actually means
    /// here. Only the first `modality.channels()` entries are read; the rest are
    /// [`InnovationId::NULL`].
    pub targets: [InnovationId; SENSOR_CHANNELS],
}

#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct EffectorGene {
    pub id: InnovationId,
    pub action: Action,
    pub params: [f32; GENE_PARAMS],
    /// The neuron whose activation drives this effector, bound by id.
    pub source: InnovationId,
}

#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct BodyGene {
    pub trait_: BodyTrait,
    pub value: f32,
}

#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct MetaGene {
    pub trait_: MetaTrait,
    pub value: f32,
}

/// One gene. `Copy`, so copying a genome at birth is a `memcpy` and allocates nothing.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum Gene {
    Neuron(NeuronGene),
    Sensor(SensorGene),
    Effector(EffectorGene),
    Connection(ConnectionGene),
    Body(BodyGene),
    Meta(MetaGene),
}

impl Default for Gene {
    /// The value an unwritten arena slot holds. Never read — a genome's block length
    /// bounds every access — but `Arena` resets slots on allocation so that a recycled
    /// block cannot show the previous tenant's genes.
    fn default() -> Self {
        Gene::Neuron(NeuronGene {
            id: InnovationId::NULL,
            tau: 1.0,
            ..NeuronGene::default()
        })
    }
}

impl Gene {
    /// Total order over genes: class first, then innovation id or trait.
    ///
    /// Neurons sort first so a single forward pass has seen every neuron before
    /// reaching anything that references one. Body and meta genes have no innovation
    /// id — there is exactly one per trait — so they sort by trait and align by trait
    /// during crossover.
    pub fn sort_key(&self) -> (u8, u32) {
        match self {
            Gene::Neuron(g) => (0, g.id.raw()),
            Gene::Sensor(g) => (1, g.id.raw()),
            Gene::Effector(g) => (2, g.id.raw()),
            Gene::Connection(g) => (3, g.id.raw()),
            Gene::Body(g) => (4, g.trait_ as u32),
            Gene::Meta(g) => (5, g.trait_ as u32),
        }
    }

    /// The innovation id, for the gene classes that carry one.
    pub fn innovation(&self) -> Option<InnovationId> {
        match self {
            Gene::Neuron(g) => Some(g.id),
            Gene::Sensor(g) => Some(g.id),
            Gene::Effector(g) => Some(g.id),
            Gene::Connection(g) => Some(g.id),
            Gene::Body(_) | Gene::Meta(_) => None,
        }
    }

    pub fn as_neuron(&self) -> Option<&NeuronGene> {
        match self {
            Gene::Neuron(g) => Some(g),
            _ => None,
        }
    }
}

/// Why a genome is not coherent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GenomeError {
    /// Genes are not in [`Gene::sort_key`] order, or two share a key.
    Unsorted,
    /// A sensor target, effector source, or connection endpoint names a neuron that
    /// is not in this genome. This is the failure mutation must never introduce.
    DanglingReference,
    /// A time constant or oscillator period that would divide by zero or blow up.
    BadNeuronParameter,
    /// A weight or bias that is not finite. One NaN reaches every downstream neuron
    /// within a tick and the agent goes permanently inert.
    NonFinite,
}

/// The neuron ids in a sorted genome, as the leading run of the slice.
fn neuron_ids(genes: &[Gene]) -> impl Iterator<Item = InnovationId> + '_ {
    genes
        .iter()
        .take_while(|g| matches!(g, Gene::Neuron(_)))
        .filter_map(|g| g.innovation())
}

/// Position of `id` within the genome's leading neuron run, or `None` if it names no
/// neuron here.
///
/// That position is also the neuron's slot in a brain compiled from this genome — the
/// two orderings are the same by construction, which is what lets `brain::compile`
/// resolve a connection endpoint with a binary search and no lookup table at all.
///
/// Binary search rather than a set: a `HashMap` would be faster to write and its
/// iteration order would be a determinism bug waiting to happen (spec §2.4).
pub fn neuron_index(genes: &[Gene], id: InnovationId) -> Option<usize> {
    let neurons = genes.partition_point(|g| matches!(g, Gene::Neuron(_)));
    genes[..neurons]
        .binary_search_by_key(&id.raw(), |g| g.sort_key().1)
        .ok()
}

/// Whether `id` names a neuron in this genome.
fn has_neuron(genes: &[Gene], id: InnovationId) -> bool {
    neuron_index(genes, id).is_some()
}

/// Checks both genome invariants. Cheap enough for a `debug_assert!` after mutation
/// and cheap enough to run on anything arriving from outside.
/// `!(x > 0.0)` rather than `x <= 0.0`: the negated form also rejects NaN, which is
/// exactly the value validation exists to catch here.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn validate(genes: &[Gene]) -> Result<(), GenomeError> {
    for pair in genes.windows(2) {
        if pair[0].sort_key() >= pair[1].sort_key() {
            return Err(GenomeError::Unsorted);
        }
    }

    for gene in genes {
        match gene {
            Gene::Neuron(n) => {
                if !(n.tau > 0.0) || !n.tau.is_finite() {
                    return Err(GenomeError::BadNeuronParameter);
                }
                if n.activation == Activation::Oscillator && !(n.period > 0.0) {
                    return Err(GenomeError::BadNeuronParameter);
                }
                if !n.bias.is_finite() {
                    return Err(GenomeError::NonFinite);
                }
            }
            Gene::Sensor(s) => {
                for &target in &s.targets[..s.modality.channels()] {
                    if !has_neuron(genes, target) {
                        return Err(GenomeError::DanglingReference);
                    }
                }
                if s.params.iter().any(|p| !p.is_finite()) {
                    return Err(GenomeError::NonFinite);
                }
            }
            Gene::Effector(e) => {
                if !has_neuron(genes, e.source) {
                    return Err(GenomeError::DanglingReference);
                }
                if e.params.iter().any(|p| !p.is_finite()) {
                    return Err(GenomeError::NonFinite);
                }
            }
            Gene::Connection(c) => {
                if !has_neuron(genes, c.from) || !has_neuron(genes, c.to) {
                    return Err(GenomeError::DanglingReference);
                }
                if !c.weight.is_finite() {
                    return Err(GenomeError::NonFinite);
                }
            }
            Gene::Body(b) => {
                if !b.value.is_finite() {
                    return Err(GenomeError::NonFinite);
                }
            }
            Gene::Meta(m) => {
                if !m.value.is_finite() {
                    return Err(GenomeError::NonFinite);
                }
            }
        }
    }
    Ok(())
}

/// Neurons plus connections: what the metabolic `k_brain` term charges for (spec §5.2).
///
/// Deliberately **not** the gene count. Sensors are charged separately by `k_sensor`,
/// at ten times the rate, because eyes are meant to be expensive — counting sensor
/// genes here too would bill them twice and collapse the one knob that decides whether
/// evolution grows more eyes or bigger brains. Body and meta genes are not brain at
/// all. The difference is invisible while every genome is identical, and becomes a
/// real distortion in Phase 2 when they stop being.
///
/// **Open decision for Phase 2: this counts disabled connections too.** Nothing in
/// Phase 1 disables one, so today the choice is invisible — but spec §3.3's
/// disable/enable operator is how a connection is ever pruned (the gene stays, so its
/// innovation id survives crossover alignment), and this filter does not look at
/// `enabled`. Charging for them means disabling buys behaviour and no energy back, so
/// nothing selects for tidying a brain up; not charging makes disable the pruning
/// mechanism with a real payoff. Neither is obviously right — a disabled gene still
/// costs memory and is still copied on every birth — and there is no remove-neuron
/// operator in §3.3 at all, so `k_brain` is the *only* thing bounding brain size
/// (CLAUDE.md). Settle it deliberately when the operator lands, not by leaving this
/// pattern as it is.
pub fn brain_complexity(genes: &[Gene]) -> u32 {
    genes
        .iter()
        .filter(|g| matches!(g, Gene::Neuron(_) | Gene::Connection(_)))
        .count() as u32
}

/// Number of sensor genes, weighted by modality: what `k_sensor` charges for.
///
/// Weighting is by channel count as a stand-in for the real cost of an organ — an eye
/// returning distance and colour costs four times a chemoreceptor's single scalar.
/// Perception is 60–80% of tick time, and metering rays as a metabolic cost is what
/// makes evolution pay for its own compute (spec §2.2c, §5.2).
pub fn sensor_load(genes: &[Gene]) -> f32 {
    genes
        .iter()
        .filter_map(|g| match g {
            Gene::Sensor(s) => Some(s.modality.channels() as f32),
            _ => None,
        })
        .sum()
}

/// The value of a body trait, or `None` if the genome does not carry it.
pub fn body_trait(genes: &[Gene], want: BodyTrait) -> Option<f32> {
    genes.iter().find_map(|g| match g {
        Gene::Body(b) if b.trait_ == want => Some(b.value),
        _ => None,
    })
}

/// Count of neurons, for sizing a compiled brain.
pub fn neuron_count(genes: &[Gene]) -> usize {
    neuron_ids(genes).count()
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    /// A minimal coherent genome: two neurons, a sensor, an effector, a connection,
    /// and one gene of each id-less class.
    pub fn tiny() -> Vec<Gene> {
        let n0 = InnovationId::new(0);
        let n1 = InnovationId::new(1);
        let mut targets = [InnovationId::NULL; SENSOR_CHANNELS];
        targets[0] = n0;
        let mut genes = vec![
            Gene::Neuron(NeuronGene {
                id: n0,
                bias: 0.1,
                tau: 0.5,
                activation: Activation::Sigmoid,
                period: 0.0,
            }),
            Gene::Neuron(NeuronGene {
                id: n1,
                bias: -0.2,
                tau: 1.5,
                activation: Activation::Oscillator,
                period: 30.0,
            }),
            Gene::Sensor(SensorGene {
                id: InnovationId::new(2),
                modality: Modality::Interoception,
                params: [0.0; GENE_PARAMS],
                targets,
            }),
            Gene::Effector(EffectorGene {
                id: InnovationId::new(3),
                action: Action::Thrust,
                params: [0.0; GENE_PARAMS],
                source: n1,
            }),
            Gene::Connection(ConnectionGene {
                id: InnovationId::new(4),
                from: n0,
                to: n1,
                weight: 0.75,
                enabled: true,
            }),
            Gene::Body(BodyGene {
                trait_: BodyTrait::Size,
                value: 3.0,
            }),
            Gene::Meta(MetaGene {
                trait_: MetaTrait::MutationRate,
                value: 0.8,
            }),
        ];
        genes.sort_by_key(Gene::sort_key);
        genes
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::tiny;
    use super::*;

    #[test]
    fn a_well_formed_genome_validates() {
        assert_eq!(validate(&tiny()), Ok(()));
        assert_eq!(
            validate(&[]),
            Ok(()),
            "an empty genome is vacuously coherent"
        );
    }

    #[test]
    fn neurons_sort_before_everything_that_references_them() {
        // A single forward pass must see every neuron before any reference to one.
        let genes = tiny();
        let last_neuron = genes
            .iter()
            .rposition(|g| matches!(g, Gene::Neuron(_)))
            .unwrap();
        let first_other = genes
            .iter()
            .position(|g| !matches!(g, Gene::Neuron(_)))
            .unwrap();
        assert!(last_neuron < first_other);
    }

    #[test]
    fn unsorted_genes_are_rejected() {
        let mut genes = tiny();
        genes.swap(0, 4);
        assert_eq!(validate(&genes), Err(GenomeError::Unsorted));
    }

    #[test]
    fn duplicate_innovation_ids_are_rejected() {
        // Two genes with one id would make crossover alignment ambiguous.
        let mut genes = tiny();
        genes.push(genes[0]);
        genes.sort_by_key(Gene::sort_key);
        assert_eq!(validate(&genes), Err(GenomeError::Unsorted));
    }

    #[test]
    fn dangling_references_are_rejected() {
        let missing = InnovationId::new(999);

        let mut sensor_dangles = tiny();
        for gene in sensor_dangles.iter_mut() {
            if let Gene::Sensor(s) = gene {
                s.targets[0] = missing;
            }
        }
        assert_eq!(
            validate(&sensor_dangles),
            Err(GenomeError::DanglingReference)
        );

        let mut effector_dangles = tiny();
        for gene in effector_dangles.iter_mut() {
            if let Gene::Effector(e) = gene {
                e.source = missing;
            }
        }
        assert_eq!(
            validate(&effector_dangles),
            Err(GenomeError::DanglingReference)
        );

        let mut connection_dangles = tiny();
        for gene in connection_dangles.iter_mut() {
            if let Gene::Connection(c) = gene {
                c.to = missing;
            }
        }
        assert_eq!(
            validate(&connection_dangles),
            Err(GenomeError::DanglingReference)
        );
    }

    #[test]
    fn a_default_id_reads_as_dangling_rather_than_neuron_zero() {
        // Default is NULL on purpose: a field left unset must fail loudly, not bind
        // silently to whichever neuron happens to be first.
        let mut genes = tiny();
        for gene in genes.iter_mut() {
            if let Gene::Effector(e) = gene {
                e.source = InnovationId::default();
            }
        }
        assert_eq!(validate(&genes), Err(GenomeError::DanglingReference));
    }

    #[test]
    fn degenerate_neuron_parameters_are_rejected() {
        for break_it in [
            (|n: &mut NeuronGene| n.tau = 0.0) as fn(&mut NeuronGene),
            |n| n.tau = -1.0,
            |n| n.tau = f32::NAN,
            |n| n.period = 0.0,
        ] {
            let mut genes = tiny();
            for gene in genes.iter_mut() {
                if let Gene::Neuron(n) = gene
                    && n.activation == Activation::Oscillator
                {
                    break_it(n);
                }
            }
            assert_eq!(validate(&genes), Err(GenomeError::BadNeuronParameter));
        }
    }

    #[test]
    fn non_finite_scalars_are_rejected() {
        // One NaN weight reaches every downstream neuron within a tick.
        let mut genes = tiny();
        for gene in genes.iter_mut() {
            if let Gene::Connection(c) = gene {
                c.weight = f32::NAN;
            }
        }
        assert_eq!(validate(&genes), Err(GenomeError::NonFinite));
    }

    #[test]
    fn only_the_used_sensor_channels_must_resolve() {
        // Interoception writes one value; the three unused target slots stay NULL and
        // must not be checked, or every narrow sensor would fail validation.
        let genes = tiny();
        let sensor = genes
            .iter()
            .find_map(|g| match g {
                Gene::Sensor(s) => Some(s),
                _ => None,
            })
            .unwrap();
        assert_eq!(sensor.modality.channels(), 1);
        assert!(sensor.targets[1..].iter().all(|t| t.is_null()));
        assert_eq!(validate(&genes), Ok(()));
    }

    #[test]
    fn round_trips_through_postcard() {
        let genes = tiny();
        let bytes = postcard::to_allocvec(&genes).expect("serializes");
        let back: Vec<Gene> = postcard::from_bytes(&bytes).expect("deserializes");
        assert_eq!(genes, back);
        assert_eq!(validate(&back), Ok(()));
    }

    #[test]
    fn a_gene_is_copy_and_small() {
        // Copy is what makes a birth a memcpy rather than an allocation. If a variant
        // ever grows a Vec this stops compiling, which is the point.
        fn assert_copy<T: Copy>() {}
        assert_copy::<Gene>();
        assert!(
            size_of::<Gene>() <= 64,
            "Gene grew to {} bytes",
            size_of::<Gene>()
        );
    }

    #[test]
    fn helpers_read_what_they_claim() {
        let genes = tiny();
        assert_eq!(neuron_count(&genes), 2);
        assert_eq!(body_trait(&genes, BodyTrait::Size), Some(3.0));
        assert_eq!(body_trait(&genes, BodyTrait::SignatureR), None);
    }

    #[test]
    fn the_two_metabolic_terms_do_not_overlap() {
        // k_brain charges neurons and connections; k_sensor charges sensors, at ten
        // times the rate. A gene counted by both would be billed twice and the two
        // coefficients would stop being independent knobs (spec §5.2).
        let genes = tiny();
        assert_eq!(brain_complexity(&genes), 3, "2 neurons + 1 connection");

        let counted_by_brain = genes
            .iter()
            .filter(|g| matches!(g, Gene::Neuron(_) | Gene::Connection(_)))
            .count();
        let counted_by_sensor = genes
            .iter()
            .filter(|g| matches!(g, Gene::Sensor(_)))
            .count();
        assert_eq!(counted_by_brain + counted_by_sensor, 4);
        assert!(
            counted_by_brain + counted_by_sensor < genes.len(),
            "body and meta are not metered"
        );
    }

    #[test]
    fn sensor_load_weights_wide_organs_more() {
        // An eye returning distance and colour should cost more than a nose.
        let eye = Modality::VisionRay.channels() as f32;
        let nose = Modality::Chemo.channels() as f32;
        assert!(eye > nose);
        // `tiny` carries one interoceptor, the narrowest sensor there is.
        assert_eq!(sensor_load(&tiny()), 1.0);
    }
}
