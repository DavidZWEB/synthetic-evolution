//! The CTRNN: a genome's network in runnable form, and the Euler step that advances it.
//!
//! A brain is **compiled** from a genome once, at birth, and never reads it again. The
//! genome binds neurons by [`InnovationId`] — a genome-level identity, resolved by
//! binary search — while evaluation touches a few hundred connections per agent per
//! tick, so doing that lookup in the tick would cost several times the arithmetic it
//! feeds. Compilation resolves every endpoint to a [`NeuronId`], a slot in this
//! agent's own activation array, and the tick is then a flat scan over two slices.
//!
//! Evaluation is one Euler step per tick, taken from the outputs the previous step
//! left behind (spec §3.2, §2.4 step 3). No neuron sees another's output from the same
//! step, so behaviour cannot depend on the order neurons happen to sit in — which is
//! also why a topological sort would be pointless here. Recurrence is the point.
//!
//! Deliberately not here: which sensor writes which input and which effector reads
//! which output — sensors and effectors bind to neurons by id and resolving that
//! binding is `perceive`'s job at M6 — and the tick that calls this (M8).

use serde::{Deserialize, Serialize};

use crate::genome::{self, Activation, Gene};
use crate::ids::{InnovationId, NeuronId};
use crate::math;

/// One neuron of a compiled brain: its state and everything needed to advance it.
///
/// Self-contained on purpose — a compiled brain does not consult the genome it came
/// from, so nothing has to keep the two in step during a tick. The reciprocals are
/// stored rather than `tau` and `period` themselves to keep a division out of the hot
/// loop; `dt` is deliberately *not* folded in, because `SimParams` is settable at
/// runtime and a baked-in timestep would silently ignore the new value.
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Neuron {
    /// Membrane potential `y`. For an [`Activation::Oscillator`] this holds a phase in
    /// radians instead: a free-running neuron ignores its input, so it has no potential
    /// to integrate and the slot is free for the one piece of state it does need.
    pub state: f32,
    /// What downstream neurons and effectors read: the activation function applied to
    /// `state + bias`, as of the last step.
    ///
    /// Stored rather than recomputed on demand so that a step reads only values written
    /// before it began. That is the whole of the "evaluate from previous activations"
    /// rule in spec §2.4.
    pub output: f32,
    /// External input `I` for the tick in progress: what perception wrote here in step
    /// 2, waiting for step 3 to consume it (spec §2.4).
    ///
    /// Per neuron rather than one scratch buffer shared across agents, because those
    /// are two separate passes — every agent perceives before any agent thinks — so the
    /// value has to survive in between. `step` clears it as it consumes it, which is
    /// what stops a tick's input from being added twice.
    pub input: f32,
    pub bias: f32,
    /// `1 / tau`. Small tau reacts, large tau integrates (spec §3.2).
    pub inv_tau: f32,
    /// `1 / period`, in cycles per tick. Zero for everything but an oscillator.
    pub inv_period: f32,
    pub activation: Activation,
}

/// One connection, with both endpoints resolved to slots in the same brain.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Synapse {
    pub from: NeuronId,
    pub to: NeuronId,
    pub weight: f32,
}

impl Default for Synapse {
    /// Slot 0 at zero weight, **not** [`NeuronId::NULL`].
    ///
    /// The arena resets a block to `Default` when it hands it out, and an unwritten
    /// slot must be inert rather than dangerous: a NULL slot index is `u32::MAX` and
    /// would index out of range in the hot loop. A zero weight makes it a no-op.
    fn default() -> Self {
        Self {
            from: NeuronId::new(0),
            to: NeuronId::new(0),
            weight: 0.0,
        }
    }
}

/// The slot `id` occupies in a brain compiled from `genes`, or `None` if it names no
/// neuron there.
///
/// Neurons are the sorted leading run of a genome (spec §3.1), so a neuron's position
/// in that run *is* its slot — `compile` writes them in exactly that order.
#[inline]
pub fn slot_of(genes: &[Gene], id: InnovationId) -> Option<NeuronId> {
    genome::neuron_index(genes, id).map(NeuronId::from)
}

/// The connections that become synapses: enabled, with both endpoints resolving.
///
/// `synapse_count` and `compile` both walk this, so the block a caller allocates and
/// the number of synapses written into it cannot disagree.
fn compiled_connections(genes: &[Gene]) -> impl Iterator<Item = Synapse> + '_ {
    genes.iter().filter_map(move |gene| match gene {
        // A disabled connection stays in the genome so its innovation id survives
        // alignment, but it is not wiring (spec §3.1).
        Gene::Connection(c) if c.enabled => Some(Synapse {
            from: slot_of(genes, c.from)?,
            to: slot_of(genes, c.to)?,
            weight: c.weight,
        }),
        _ => None,
    })
}

/// How many synapses `genes` compiles to. Sizes the arena block before `compile` fills
/// it.
pub fn synapse_count(genes: &[Gene]) -> usize {
    compiled_connections(genes).count()
}

/// Writes the runnable form of `genes` into a freshly claimed pair of arena blocks.
///
/// Called on every birth and never inside a tick — the endpoint resolution here is a
/// binary search per connection, which is exactly the cost this compilation step
/// exists to pay once instead of sixty times a second.
///
/// Neurons start silent: `state` and `output` are zero, so a newborn drives nothing
/// until it has taken its first step.
pub fn compile(genes: &[Gene], neurons: &mut [Neuron], synapses: &mut [Synapse]) {
    debug_assert_eq!(
        neurons.len(),
        genome::neuron_count(genes),
        "brain block does not match the genome's neuron count"
    );
    debug_assert_eq!(
        synapses.len(),
        synapse_count(genes),
        "wiring block does not match the genome's connection count"
    );

    for (slot, gene) in neurons
        .iter_mut()
        .zip(genes.iter().filter_map(Gene::as_neuron))
    {
        *slot = Neuron {
            state: 0.0,
            output: 0.0,
            input: 0.0,
            bias: gene.bias,
            // `validate` rejects a non-positive tau, and an oscillator's period with
            // it, so neither reciprocal can be an infinity here.
            inv_tau: 1.0 / gene.tau,
            inv_period: if gene.activation == Activation::Oscillator {
                1.0 / gene.period
            } else {
                0.0
            },
            activation: gene.activation,
        };
    }

    for (slot, synapse) in synapses.iter_mut().zip(compiled_connections(genes)) {
        *slot = synapse;
    }
}

/// Advances every neuron one Euler step, consuming whatever perception left in
/// [`Neuron::input`] and clearing it for the next tick.
///
/// Synapses are summed in genome order. Float addition does not reassociate, so the
/// order is part of the result and has to be the same on every platform and every run
/// (spec §7.4).
pub fn step(neurons: &mut [Neuron], synapses: &[Synapse], dt: f32) {
    for synapse in synapses {
        // Read and write in separate statements: `neurons` cannot be borrowed shared
        // and unique at once, and the source's output is the *previous* step's either
        // way, so lifting it into a local changes nothing but the borrow.
        let source = neurons[synapse.from.index()].output;
        neurons[synapse.to.index()].input += synapse.weight * source;
    }

    for neuron in neurons.iter_mut() {
        // Reads the accumulated input and leaves 0.0 behind in one move, so a tick's
        // input cannot be counted twice and the next one starts from a clean slate.
        let input = core::mem::take(&mut neuron.input);
        if neuron.activation == Activation::Oscillator {
            // Free-running: it ignores `input` entirely and advances its phase instead
            // (spec §3.2). Wrapped every step, or the phase loses f32 precision after
            // a few hours of ticks and the period visibly drifts.
            neuron.state = math::wrap_pi(neuron.state + core::f32::consts::TAU * neuron.inv_period);
        } else {
            // dy/dt = (1/tau)(-y + Σ w·σ(y+b) + I), one Euler step (spec §3.2).
            neuron.state += euler_gain(dt, neuron.inv_tau) * (input - neuron.state);
        }
        neuron.output = activate(neuron.activation, neuron.state + neuron.bias);
    }
}

/// The Euler coefficient `dt / tau`, capped at 1.
///
/// Mutation may drive tau below the timestep — `mutate` floors it at 1e-3 against a dt
/// of 1/60 — and above 1 the update overshoots its target every step and the network
/// diverges to infinity within a few dozen ticks, taking every downstream neuron with
/// it. The cap is not a fudge: a neuron faster than the timestep can only be observed
/// tracking its input exactly, which is what a gain of 1 does.
#[inline]
fn euler_gain(dt: f32, inv_tau: f32) -> f32 {
    (dt * inv_tau).min(1.0)
}

/// What a neuron emits, given `state + bias`.
#[inline]
fn activate(kind: Activation, x: f32) -> f32 {
    match kind {
        Activation::Sigmoid => math::sigmoid(x),
        Activation::Tanh => math::tanh(x),
        // Ranged to [0, 1] to match the sigmoid neurons it feeds, so one weight scale
        // is right across the whole brain; a negative weight inverts it. `x` is the
        // phase plus the bias gene, which makes bias a phase offset here — the second
        // thing a free-running neuron can evolve, after its period.
        Activation::Oscillator => 0.5 + 0.5 * math::sin(x),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::founder::FounderPlan;
    use crate::genome::{ConnectionGene, Gene, NeuronGene, validate};
    use crate::ids::InnovationId;
    use crate::params::SimParams;
    use crate::rng::Rng;

    /// A genome of `neurons` neurons and the given connections, sorted and coherent.
    fn net(neurons: &[NeuronGene], connections: &[(u32, u32, f32)]) -> Vec<Gene> {
        let mut genes: Vec<Gene> = neurons.iter().copied().map(Gene::Neuron).collect();
        for (i, &(from, to, weight)) in connections.iter().enumerate() {
            genes.push(Gene::Connection(ConnectionGene {
                id: InnovationId::new(1_000 + i as u32),
                from: InnovationId::new(from),
                to: InnovationId::new(to),
                weight,
                enabled: true,
            }));
        }
        genes.sort_by_key(Gene::sort_key);
        assert_eq!(validate(&genes), Ok(()), "test fixture is incoherent");
        genes
    }

    fn neuron(id: u32, bias: f32, tau: f32) -> NeuronGene {
        NeuronGene {
            id: InnovationId::new(id),
            bias,
            tau,
            activation: Activation::Sigmoid,
            period: 0.0,
        }
    }

    fn oscillator(id: u32, period: f32) -> NeuronGene {
        NeuronGene {
            id: InnovationId::new(id),
            bias: 0.0,
            tau: 1.0,
            activation: Activation::Oscillator,
            period,
        }
    }

    /// Compiles `genes` into a brain and its wiring.
    fn build(genes: &[Gene]) -> (Vec<Neuron>, Vec<Synapse>) {
        let mut neurons = vec![Neuron::default(); genome::neuron_count(genes)];
        let mut synapses = vec![Synapse::default(); synapse_count(genes)];
        compile(genes, &mut neurons, &mut synapses);
        (neurons, synapses)
    }

    #[test]
    fn a_hand_built_neuron_produces_the_expected_numbers() {
        // One sigmoid neuron, no wiring: y' = (1/tau)(-y + I) integrated at dt.
        // dt/tau = 0.5, so y goes 0 → 0.5·2 = 1.0 → 1.0 + 0.5·(2.0 - 1.0) = 1.5.
        let genes = net(&[neuron(0, 0.0, 1.0)], &[]);
        let (mut neurons, synapses) = build(&genes);
        let dt = 0.5;

        neurons[0].input = 2.0;
        step(&mut neurons, &synapses, dt);
        assert!((neurons[0].state - 1.0).abs() < 1e-6, "{:?}", neurons[0]);
        assert!((neurons[0].output - math::sigmoid(1.0)).abs() < 1e-6);

        neurons[0].input = 2.0;
        step(&mut neurons, &synapses, dt);
        assert!((neurons[0].state - 1.5).abs() < 1e-6, "{:?}", neurons[0]);
        assert!((neurons[0].output - math::sigmoid(1.5)).abs() < 1e-6);
    }

    #[test]
    fn bias_shifts_the_output_without_shifting_the_state() {
        let genes = net(&[neuron(0, 0.75, 1.0)], &[]);
        let (mut neurons, synapses) = build(&genes);
        step(&mut neurons, &synapses, 1.0);
        assert_eq!(neurons[0].state, 0.0, "no input, so no potential");
        assert!((neurons[0].output - math::sigmoid(0.75)).abs() < 1e-6);
    }

    #[test]
    fn a_step_reads_the_previous_outputs_not_this_step_s() {
        // A → B with a heavy weight. B must see nothing on the first step, because A's
        // output is still the zero it was born with; only on the second does A's first
        // output reach it. Without that, behaviour would depend on slot order
        // (spec §2.4).
        let genes = net(&[neuron(0, 0.0, 1.0), neuron(1, 0.0, 1.0)], &[(0, 1, 4.0)]);
        let (mut neurons, synapses) = build(&genes);
        neurons[0].input = 1.0;
        step(&mut neurons, &synapses, 1.0);
        assert_eq!(neurons[1].state, 0.0, "B saw A's output in the same step");
        let a_output = neurons[0].output;
        step(&mut neurons, &synapses, 1.0);
        assert!(
            (neurons[1].state - 4.0 * a_output).abs() < 1e-6,
            "B did not receive A's previous output"
        );
    }

    #[test]
    fn tau_sets_how_fast_a_neuron_follows_its_input() {
        // The property the whole CTRNN rests on: the spread of tau across a brain is
        // what gives it a memory of any length (spec §3.2).
        let dt = 1.0 / 60.0;
        let genes = net(&[neuron(0, 0.0, dt), neuron(1, 0.0, 100.0 * dt)], &[]);
        let (mut neurons, synapses) = build(&genes);

        for _ in 0..10 {
            for n in neurons.iter_mut() {
                n.input = 1.0;
            }
            step(&mut neurons, &synapses, dt);
        }
        assert!(
            (neurons[0].state - 1.0).abs() < 1e-5,
            "tau = dt should track its input exactly: {}",
            neurons[0].state
        );
        assert!(
            neurons[1].state < 0.2,
            "tau = 100 dt should still be lagging: {}",
            neurons[1].state
        );
    }

    #[test]
    fn a_tau_below_the_timestep_does_not_diverge() {
        // Mutation floors tau at 1e-3, an order of magnitude under a 1/60 timestep. An
        // unclamped Euler gain of 16 makes the state alternate and grow without bound,
        // and the NaN that follows reaches every downstream neuron in one tick.
        let genes = net(&[neuron(0, 0.0, 1e-3)], &[]);
        let (mut neurons, synapses) = build(&genes);
        for _ in 0..1_000 {
            for n in neurons.iter_mut() {
                n.input = 1.0;
            }
            step(&mut neurons, &synapses, 1.0 / 60.0);
        }
        assert!(neurons[0].state.is_finite(), "{}", neurons[0].state);
        assert!(
            (neurons[0].state - 1.0).abs() < 1e-5,
            "{}",
            neurons[0].state
        );
    }

    #[test]
    fn an_oscillator_runs_at_its_period() {
        const PERIOD: usize = 40;
        let genes = net(&[oscillator(0, PERIOD as f32)], &[]);
        let (mut neurons, synapses) = build(&genes);

        let mut trace = Vec::new();
        for _ in 0..PERIOD * 4 {
            step(&mut neurons, &synapses, 1.0 / 60.0);
            trace.push(neurons[0].output);
        }

        for (t, &value) in trace.iter().enumerate().take(PERIOD * 3) {
            assert!(
                (value - trace[t + PERIOD]).abs() < 1e-4,
                "tick {t} does not repeat one period later: {value} vs {}",
                trace[t + PERIOD]
            );
        }
        let low = trace.iter().copied().fold(f32::INFINITY, f32::min);
        let high = trace.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(low < 0.02 && high > 0.98, "swing was only {low}..{high}");
    }

    #[test]
    fn an_oscillator_ignores_what_is_wired_into_it() {
        // "Free-running" is the whole reason oscillators are a useful scaffold: a
        // neuron whose rhythm can be shouted down is not a clock (spec §3.2).
        let genes = net(&[oscillator(0, 25.0), neuron(1, 0.0, 1.0)], &[(1, 0, 4.0)]);
        let (mut neurons, synapses) = build(&genes);
        let mut driven = Vec::new();
        for _ in 0..100 {
            neurons[1].input = 50.0;
            step(&mut neurons, &synapses, 1.0 / 60.0);
            driven.push(neurons[0].output);
        }

        let alone = net(&[oscillator(0, 25.0)], &[]);
        let (mut neurons, synapses) = build(&alone);
        for (t, expected) in driven.iter().enumerate() {
            step(&mut neurons, &synapses, 1.0 / 60.0);
            assert!(
                (neurons[0].output - expected).abs() < 1e-6,
                "input moved the oscillator at tick {t}"
            );
        }
    }

    #[test]
    fn compilation_resolves_endpoints_to_slots() {
        // Innovation ids are sparse and unordered relative to slots; binding by id is
        // the indirection that lets an organ be lost without breaking the brain.
        let genes = net(
            &[
                neuron(7, 0.0, 1.0),
                neuron(3, 0.0, 1.0),
                neuron(11, 0.0, 1.0),
            ],
            &[(11, 3, 1.0)],
        );
        let (_, synapses) = build(&genes);
        // Sorted by id: slot 0 is neuron 3, slot 1 is neuron 7, slot 2 is neuron 11.
        assert_eq!(synapses.len(), 1);
        assert_eq!(synapses[0].from, NeuronId::new(2));
        assert_eq!(synapses[0].to, NeuronId::new(0));
        assert_eq!(
            slot_of(&genes, InnovationId::new(7)),
            Some(NeuronId::new(1))
        );
        assert_eq!(slot_of(&genes, InnovationId::new(9)), None);
    }

    #[test]
    fn a_disabled_connection_is_not_wiring() {
        let mut genes = net(&[neuron(0, 0.0, 1.0), neuron(1, 0.0, 1.0)], &[(0, 1, 4.0)]);
        for gene in genes.iter_mut() {
            if let Gene::Connection(c) = gene {
                c.enabled = false;
            }
        }
        assert_eq!(
            synapse_count(&genes),
            0,
            "a disabled connection was compiled"
        );

        let (mut neurons, synapses) = build(&genes);
        for _ in 0..10 {
            neurons[0].input = 4.0;
            step(&mut neurons, &synapses, 1.0);
        }
        assert_eq!(
            neurons[1].state, 0.0,
            "a disabled connection carried signal"
        );
    }

    /// What a settled founder brain's wired neurons are doing, over `seeds` founders.
    ///
    /// Two numbers, because "saturated" has two faces. `pinned` is the fraction sitting
    /// against a rail of the sigmoid; `slope` is the mean of `σ·(1-σ)`, the activation's
    /// own derivative — how much a neuron's output would move if its input did. A brain
    /// can only be selected on through that derivative, so it is the quantity that
    /// matters and 0.25 is the most it can be.
    ///
    /// Only neurons with incoming connections are counted. A sensor-bound input neuron
    /// has no fan-in, so it holds `σ(bias)` whatever the weights do; it is the wired
    /// neurons that pin.
    fn settled_founders(params: &SimParams, seeds: u64) -> (f32, f32) {
        let mut next = 0u32;
        let plan = FounderPlan::new(params, || {
            next += 1;
            InnovationId::new(next - 1)
        });
        let mut genes = vec![Gene::default(); plan.len()];
        let (mut pinned, mut slope, mut counted) = (0u32, 0.0f32, 0u32);

        for seed in 0..seeds {
            plan.instantiate(&mut Rng::from_seed(seed), params, &mut genes);
            let (mut neurons, synapses) = build(&genes);
            let mut wired = vec![false; neurons.len()];
            for synapse in &synapses {
                wired[synapse.to.index()] = true;
            }
            // Long enough for the slowest tau in the default range to settle.
            for _ in 0..600 {
                step(&mut neurons, &synapses, params.world.dt);
            }
            for (n, &is_wired) in neurons.iter().zip(wired.iter()) {
                if !is_wired || n.activation == Activation::Oscillator {
                    continue;
                }
                assert!(n.output.is_finite(), "founder brain produced {}", n.output);
                counted += 1;
                slope += n.output * (1.0 - n.output);
                if !(0.02..=0.98).contains(&n.output) {
                    pinned += 1;
                }
            }
        }
        assert!(counted > 0, "no wired neurons to measure");
        (pinned as f32 / counted as f32, slope / counted as f32)
    }

    #[test]
    fn a_founder_brain_is_distributed_rather_than_pinned() {
        // M5's acceptance criterion. A saturated brain does not respond to its sensors,
        // and weight mutation moves it far too slowly to recover, so the population
        // looks alive and evolves nothing — which reads as a tuning failure rather than
        // an initialisation bug.
        //
        // Measured over 64 founders: 0% pinned, mean slope 0.217 of a possible 0.25.
        let (pinned, slope) = settled_founders(&SimParams::default(), 64);
        assert!(
            pinned < 0.02,
            "{:.0}% of a founder's wired neurons are against a rail",
            pinned * 100.0
        );
        assert!(slope > 0.15, "mean activation slope is only {slope:.3}");
    }

    #[test]
    fn without_fan_in_scaling_a_founder_saturates() {
        // The check above is only worth having if it can fail. Scaling the draw back up
        // by √fan-in undoes the divisor, which is what drawing from the full
        // ±`weight_limit` bound amounted to — and half the same brain pins at its rails
        // while its mean slope falls by a factor of three.
        let mut params = SimParams::default();
        let fan_in = 24.0; // sources feeding each sink in the default topology
        params.brain.weight_init_scale = params.mutation.weight_limit * math::sqrt(fan_in);
        let (pinned, slope) = settled_founders(&params, 64);
        assert!(
            pinned > 0.4,
            "only {:.0}% pinned; the acceptance check above may be vacuous",
            pinned * 100.0
        );
        assert!(slope < 0.10, "mean activation slope is still {slope:.3}");
    }

    #[test]
    fn compiling_the_same_genome_twice_gives_the_same_brain() {
        let params = SimParams::default();
        let mut next = 0u32;
        let plan = FounderPlan::new(&params, || {
            next += 1;
            InnovationId::new(next - 1)
        });
        let mut genes = vec![Gene::default(); plan.len()];
        plan.instantiate(&mut Rng::from_seed(4), &params, &mut genes);
        let (a_neurons, a_synapses) = build(&genes);
        let (b_neurons, b_synapses) = build(&genes);
        assert_eq!(a_neurons, b_neurons);
        assert_eq!(a_synapses, b_synapses);
    }

    #[test]
    fn a_compiled_brain_stays_small() {
        // Both arenas are allocated at `max_agents` and never grow, so every byte here
        // is multiplied by the pool: the default topology's 240 synapses are 19% of an
        // agent's whole footprint. See the memory note in `arena`.
        //
        // 28 rather than 24 since M6. `Neuron::input` is the one field bought
        // deliberately: perception and evaluation are separate passes over every agent,
        // so a tick's sensor input has to survive between them, and the alternative — a
        // second arena of one f32 per neuron — costs the same 0.6 MB while adding an
        // allocator, a handle, and a buffer whose clearing is the caller's problem.
        assert!(size_of::<Neuron>() <= 28, "{}", size_of::<Neuron>());
        assert!(size_of::<Synapse>() <= 12, "{}", size_of::<Synapse>());
    }

    #[test]
    fn an_unwritten_synapse_is_inert_rather_than_a_null_slot() {
        // The arena hands out blocks reset to Default, and a NULL slot index would be
        // an out-of-range panic in the hot loop rather than a quiet no-op.
        let genes = net(&[neuron(0, 0.0, 1.0)], &[]);
        let (mut neurons, _) = build(&genes);
        let synapses = [Synapse::default()];
        step(&mut neurons, &synapses, 1.0);
        assert_eq!(neurons[0].state, 0.0);
    }
}
