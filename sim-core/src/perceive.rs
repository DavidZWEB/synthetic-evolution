//! Perception: running an agent's sensor genes against the world and writing what they
//! return into its brain.
//!
//! **Agents do not read world state.** Nothing in `agents.rs` is a network input. Each
//! sensor is a narrow, local, lossy query — one number from one organ with a limited
//! range — and the world arrays are the substrate those queries run against, not the
//! input vector (spec §2.2c). Handing an agent the position array instead would make
//! every organism omniscient, and camouflage is worthless against something that reads
//! the array.
//!
//! Sensors bind to neurons **by innovation id**, which is what lets an agent lose an
//! eye and keep a coherent brain. Resolving that id is a binary search, so it happens
//! once at birth: [`compile`] writes a [`Sensor`] whose targets are already brain
//! slots, exactly as `brain::compile` does for connections.
//!
//! This is where the time goes. Perception is the only phase that touches the spatial
//! hash and will be 60–80% of tick cost once vision is real, which is why ray count is
//! metered as a metabolic cost — evolution pays for its own compute (spec §2.2c, §5.2).
//!
//! Deliberately not here: what the brain does with the numbers (`brain`), and anything
//! that changes the world. A sensor is read-only by construction — [`WorldView`] holds
//! shared references and nothing else.

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::brain::Neuron;
use crate::chemo::ChemoField;
use crate::corpses::Corpses;
use crate::genome::{GENE_PARAMS, Gene, Modality, SENSOR_CHANNELS};
use crate::ids::NeuronId;
use crate::math;
use crate::plants::Plants;
use crate::spatial::SpatialHash;

/// One sensor of a compiled agent: its modality, its parameters, and the brain slots
/// its channels write to.
///
/// Params are copied out of the gene rather than read through it, so perception never
/// touches the genome arena — 11 KB per agent that would otherwise be dragged through
/// cache once per tick for the sake of five floats.
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
pub struct Sensor {
    pub modality: Modality,
    /// `[azimuth, elevation, range, fov]` for a ray; `[channel, radius, _, _]` for a
    /// chemoreceptor. Elevation is clamped to 0 for all of V1 and has no mutation
    /// operator, but it occupies its slot so going 3D is an unclamping rather than a
    /// format break (spec §4.1, §9.1).
    pub params: [f32; GENE_PARAMS],
    /// One brain slot per channel. Only the first `modality.channels()` are meaningful;
    /// the rest are [`NeuronId::NULL`] and must not be written through.
    pub targets: [NeuronId; SENSOR_CHANNELS],
}

/// The world as a sensor may see it: shared references, and no way to change anything.
///
/// A bundle rather than eight parameters, and shared rather than mutable on purpose —
/// "sensors query the world, they are not fed world state" is a load-bearing invariant
/// (spec §2.2c), and a type that cannot mutate is a cheaper way to hold it than a rule
/// someone has to remember.
#[derive(Clone, Copy)]
pub struct WorldView<'a> {
    pub positions: &'a [Vec3],
    pub signatures: &'a [Vec3],
    pub sizes: &'a [f32],
    pub hash: &'a SpatialHash,
    pub field: &'a ChemoField,
    /// Plants are visible, not only smellable — see `PlantParams::signature` for why
    /// that was a decision rather than an omission.
    pub plants: &'a Plants,
    pub plant_radius: f32,
    pub plant_signature: Vec3,
    /// Corpses are visible in their own colour (spec §5.1).
    pub corpses: &'a Corpses,
    pub corpse_radius: f32,
    pub corpse_signature: Vec3,
}

/// The sensing agent's own state — what an interoceptor reads and what every
/// directional sensor is relative to.
#[derive(Clone, Copy, Debug)]
pub struct SelfView {
    /// Pool index, so an agent's own body does not blind its own eye.
    pub index: u32,
    pub position: Vec3,
    pub orientation: Quat,
    /// Energy in units of a full starting tank, not raw joules.
    ///
    /// Bounded like every other channel: a raw 100 would saturate the neuron it
    /// reaches and report the same "very full" whatever the agent's actual state. In
    /// tanks it sits near 1 at birth and 1.5 at the reproduction threshold, which is
    /// the band a sigmoid can still tell apart.
    pub energy_tanks: f32,
}

/// Compiles the sensor genes of `genes` into `out`, resolving every target to a slot.
///
/// Written in gene order, which is innovation-id order, so two agents of the same
/// lineage compile their organs into the same positions.
pub fn compile(genes: &[Gene], out: &mut [Sensor]) {
    debug_assert_eq!(
        out.len(),
        sensor_count(genes),
        "sensor block is the wrong size"
    );
    for (slot, sensor) in out.iter_mut().zip(compiled_sensors(genes)) {
        *slot = sensor;
    }
}

/// The sensor genes that compile to a working organ: every channel's target resolves.
fn compiled_sensors(genes: &[Gene]) -> impl Iterator<Item = Sensor> + '_ {
    genes.iter().filter_map(move |gene| {
        let Gene::Sensor(s) = gene else { return None };
        let mut targets = [NeuronId::NULL; SENSOR_CHANNELS];
        for (target, &id) in targets
            .iter_mut()
            .zip(s.targets.iter())
            .take(s.modality.channels())
        {
            *target = crate::brain::slot_of(genes, id)?;
        }
        Some(Sensor {
            modality: s.modality,
            params: s.params,
            targets,
        })
    })
}

/// How many sensors `genes` compiles to. Sizes the arena block before `compile` fills it.
pub fn sensor_count(genes: &[Gene]) -> usize {
    compiled_sensors(genes).count()
}

/// Runs every sensor of one agent and adds what they return to their target neurons.
///
/// Adds rather than assigns: two channels of a mutated genome may bind to one neuron,
/// and a neuron receiving two inputs summing them is what the CTRNN equation already
/// says happens (spec §3.2). `brain::step` clears the accumulator as it consumes it.
///
/// Step 2 of the tick. It runs for every agent before any brain steps, so no agent's
/// perception can see another's decision (spec §2.4).
pub fn perceive(sensors: &[Sensor], agent: &SelfView, world: &WorldView, neurons: &mut [Neuron]) {
    let mut channels = [0.0f32; SENSOR_CHANNELS];
    for sensor in sensors {
        let written = match sensor.modality {
            Modality::VisionRay => vision_ray(sensor, agent, world, &mut channels),
            Modality::Chemo => chemo(sensor, agent, world, &mut channels),
            // Spec §4.1 has the `which` param selecting energy, age, or health. Only
            // energy is worth sensing in Phase 1 — nothing reduces health before
            // predation lands, and age is a number an agent can do nothing about until
            // there is a life-history strategy to spend it on. The param slot stays,
            // and the branch on it arrives with the second thing to report.
            Modality::Interoception => {
                channels[0] = agent.energy_tanks;
                1
            }
        };
        debug_assert_eq!(written, sensor.modality.channels());
        for (&target, &value) in sensor.targets.iter().zip(channels.iter()).take(written) {
            // A channel whose target did not resolve is not compiled at all, so this is
            // belt and braces — but a NULL slot is `u32::MAX` and would index out of
            // range rather than fail quietly.
            debug_assert!(!target.is_null(), "compiled sensor kept a NULL target");
            if let Some(neuron) = neurons.get_mut(target.index()) {
                neuron.input += value;
            }
        }
    }
}

/// Casts one ray and reports what it hit: `[nearness, r, g, b]`.
///
/// Reports *nearness* — 1 at the eye, 0 at the limit of range — rather than raw
/// distance, so an empty field of view and an infinitely distant object read the same,
/// and so the value arrives already scaled to the range a sigmoid responds to. A raw
/// distance of 60 would saturate any neuron it reached.
fn vision_ray(
    sensor: &Sensor,
    agent: &SelfView,
    world: &WorldView,
    out: &mut [f32; SENSOR_CHANNELS],
) -> usize {
    let [azimuth, _elevation, range, fov] = sensor.params;
    out.fill(0.0);

    // Ray direction: the agent's facing, turned by the gene's azimuth. Elevation is
    // pinned to the plane for all of V1 (spec §4.1).
    let heading = math::yaw_of(agent.orientation) + math::reduce_angle(azimuth);
    let ray = Ray {
        origin: agent.position,
        direction: Vec3::new(math::cos(heading), math::sin(heading), 0.0),
        range,
        // `fov` is the full cone width, so a hit has to lie within half of it.
        cos_limit: math::cos((fov * 0.5).clamp(0.0, core::f32::consts::PI)),
    };

    // Every population, nearest wins. Agents, plants, and corpses live in separate
    // pools with separate grids, so an eye that queried only one would be blind to the
    // others — which for plants would mean paying for three organs that never see food.
    let mut seen: Option<Sighting> = None;
    cast(
        &ray,
        world.hash,
        world.positions,
        agent.index,
        |i| world.sizes[i],
        |i| world.signatures[i],
        &mut seen,
    );
    cast(
        &ray,
        world.plants.hash(),
        world.plants.position(),
        u32::MAX,
        |_| world.plant_radius,
        |_| world.plant_signature,
        &mut seen,
    );
    if let Some(hash) = world.corpses.hash() {
        cast(
            &ray,
            hash,
            world.corpses.position(),
            u32::MAX,
            |_| world.corpse_radius,
            |_| world.corpse_signature,
            &mut seen,
        );
    }

    if let Some(hit) = seen {
        out[0] = 1.0 - (hit.distance / range).clamp(0.0, 1.0);
        out[1] = hit.signature.x;
        out[2] = hit.signature.y;
        out[3] = hit.signature.z;
    }
    4
}

/// One ray, in world space.
struct Ray {
    origin: Vec3,
    direction: Vec3,
    range: f32,
    cos_limit: f32,
}

/// The nearest thing the ray has met so far.
struct Sighting {
    distance: f32,
    signature: Vec3,
}

/// Scans one population and keeps `best` pointed at the nearest hit found anywhere.
///
/// Generic over how a population reports its radius and colour rather than taking two
/// more slices, because plants share one of each between them while agents carry their
/// own. Monomorphised, so the tick still costs no indirect call (spec §7.5).
fn cast(
    ray: &Ray,
    hash: &SpatialHash,
    positions: &[Vec3],
    skip: u32,
    radius_of: impl Fn(usize) -> f32,
    signature_of: impl Fn(usize) -> Vec3,
    best: &mut Option<Sighting>,
) {
    hash.for_each_within(positions, ray.origin, ray.range, |index, offset, d2| {
        // An agent's own body sits at distance zero and would be a permanent hit,
        // blinding the eye to everything else.
        if index == skip {
            return;
        }
        let distance = math::sqrt(d2);
        if best.as_ref().is_some_and(|hit| distance >= hit.distance) {
            return;
        }
        let along = offset.dot(ray.direction);
        if along <= 0.0 {
            return; // behind the eye
        }
        // A body has width: it counts as hit if it falls inside the cone *or* the ray
        // passes within its radius, which is what stops a narrow eye threading between
        // two neighbours.
        let within_cone = along >= distance * ray.cos_limit;
        let within_radius = (offset - ray.direction * along).length() <= radius_of(index as usize);
        if within_cone || within_radius {
            *best = Some(Sighting {
                distance,
                signature: signature_of(index as usize),
            });
        }
    });
}

/// Samples the pheromone field: `[strength, uphill ahead, uphill left]`.
///
/// The direction is a **unit vector in the agent's own frame** — `+x` is straight
/// ahead, `+y` is to its left — and that is the whole difference between a sensor an
/// organism can act on and a fact about the world it has no way to use. A world-frame
/// vector would need to be combined with the agent's own heading to mean anything, and
/// Phase 1 has no proprioceptor to combine it with; it would also be a global truth
/// handed to a local organ, which is the omniscience spec §2.2c exists to prevent.
/// Spec §4.1 asks for "gradient **direction**", and a direction is what this returns.
///
/// Both scalars are bounded, for the reason `vision_ray` reports nearness rather than
/// distance: a raw gradient is a spatial derivative of order 0.01 and would never move
/// a neuron, while a raw concentration accumulates without limit and would saturate one
/// permanently. Strength saturates as `c / (1 + c)`, which is also how a real
/// chemoreceptor behaves — it reports *some / lots*, not an absolute count.
fn chemo(
    sensor: &Sensor,
    agent: &SelfView,
    world: &WorldView,
    out: &mut [f32; SENSOR_CHANNELS],
) -> usize {
    let channel = sensor.params[0].max(0.0) as usize;
    let radius = sensor.params[1];
    let concentration = world.field.sample(channel, agent.position);
    let gradient = world.field.gradient(channel, agent.position, radius);

    // Rotating the world-frame gradient into the agent's frame, so `+x` is ahead.
    // Through the quaternion rather than a sin/cos pair: it costs no transcendentals
    // and cannot disagree with `math::forward` about which way the agent is pointing.
    let local = agent.orientation.inverse() * gradient;
    let uphill = local.normalize_or_zero();

    out[0] = concentration / (1.0 + concentration);
    out[1] = uphill.x;
    out[2] = uphill.y;
    3
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brain;
    use crate::genome::{Activation, GenomeError, NeuronGene, SensorGene, validate};
    use crate::ids::InnovationId;
    use crate::params::{ChemoParams, SimParams};
    use crate::pool::SlotPool;

    /// A genome of `n` neurons plus one sensor bound to the first `channels()` of them.
    fn one_sensor(modality: Modality, params: [f32; GENE_PARAMS]) -> Vec<Gene> {
        let n = SENSOR_CHANNELS;
        let mut genes: Vec<Gene> = (0..n)
            .map(|i| {
                Gene::Neuron(NeuronGene {
                    id: InnovationId::new(i as u32),
                    bias: 0.0,
                    tau: 1.0,
                    activation: Activation::Sigmoid,
                    period: 0.0,
                })
            })
            .collect();
        let mut targets = [InnovationId::NULL; SENSOR_CHANNELS];
        for (t, i) in targets.iter_mut().zip(0..modality.channels()) {
            *t = InnovationId::new(i as u32);
        }
        genes.push(Gene::Sensor(SensorGene {
            id: InnovationId::new(100),
            modality,
            params,
            targets,
        }));
        genes.sort_by_key(Gene::sort_key);
        assert_eq!(validate(&genes), Ok(()), "fixture is incoherent");
        genes
    }

    fn compile_sensors(genes: &[Gene]) -> Vec<Sensor> {
        let mut out = vec![Sensor::default(); sensor_count(genes)];
        compile(genes, &mut out);
        out
    }

    fn neurons_for(genes: &[Gene]) -> Vec<Neuron> {
        let mut neurons = vec![Neuron::default(); crate::genome::neuron_count(genes)];
        let mut synapses = vec![brain::Synapse::default(); brain::synapse_count(genes)];
        brain::compile(genes, &mut neurons, &mut synapses);
        neurons
    }

    /// A world holding `positions`, with matching signatures and sizes.
    struct Fixture {
        positions: Vec<Vec3>,
        signatures: Vec<Vec3>,
        sizes: Vec<f32>,
        hash: SpatialHash,
        field: ChemoField,
        pool: SlotPool,
        plants: Plants,
        plant_radius: f32,
        plant_signature: Vec3,
        corpses: Corpses,
    }

    impl Fixture {
        fn new(positions: Vec<Vec3>) -> Self {
            let params = SimParams::default();
            let n = positions.len() as u32;
            let mut pool = SlotPool::with_capacity(n.max(1));
            for _ in 0..n {
                pool.alloc().expect("capacity");
            }
            let mut hash = SpatialHash::new(
                params.world.size,
                params.sensing.max_sense_radius(),
                n.max(1),
            );
            let mut grid_cell = vec![0u32; positions.len()];
            hash.rebuild(&positions, pool.alive_flags(), &mut grid_cell);
            // No plants unless a test asks for them, so a fixture measuring what an
            // eye makes of *agents* is not quietly seeing scenery as well.
            let empty = SimParams {
                plants: crate::params::PlantParams {
                    max_plants: 0,
                    ..params.plants.clone()
                },
                ..params.clone()
            };
            Self {
                signatures: vec![Vec3::new(0.25, 0.5, 0.75); positions.len()],
                sizes: vec![params.body.size; positions.len()],
                field: ChemoField::new(&ChemoParams::default(), params.world.size),
                plants: Plants::new(&empty, &mut crate::rng::Rng::from_seed(1)),
                plant_radius: params.plants.radius,
                plant_signature: Vec3::from(params.plants.signature),
                corpses: Corpses::new(&params),
                positions,
                hash,
                pool,
            }
        }

        fn view(&self) -> WorldView<'_> {
            WorldView {
                positions: &self.positions,
                signatures: &self.signatures,
                sizes: &self.sizes,
                hash: &self.hash,
                field: &self.field,
                plants: &self.plants,
                plant_radius: self.plant_radius,
                plant_signature: self.plant_signature,
                corpses: &self.corpses,
                corpse_radius: SimParams::default().corpses.radius,
                corpse_signature: Vec3::from(SimParams::default().corpses.signature),
            }
        }

        fn agent(&self, index: u32, yaw: f32) -> SelfView {
            SelfView {
                index,
                position: self.positions[index as usize],
                orientation: math::yaw_quat(yaw),
                energy_tanks: 1.0,
            }
        }
    }

    #[test]
    fn compilation_resolves_every_channel_to_a_slot() {
        let genes = one_sensor(Modality::VisionRay, [0.0, 0.0, 60.0, 0.5]);
        let sensors = compile_sensors(&genes);
        assert_eq!(sensors.len(), 1);
        for slot in 0..Modality::VisionRay.channels() {
            assert_eq!(sensors[0].targets[slot], NeuronId::new(slot as u32));
        }
    }

    #[test]
    fn unused_channels_stay_null_and_are_never_written() {
        // Interoception writes one value. If the three spare target slots were treated
        // as real, a narrow sensor would scribble into whatever neuron sorts first.
        let genes = one_sensor(Modality::Interoception, [0.0; GENE_PARAMS]);
        let sensors = compile_sensors(&genes);
        assert!(sensors[0].targets[1..].iter().all(|t| t.is_null()));

        let fixture = Fixture::new(vec![Vec3::new(100.0, 100.0, 0.0)]);
        let mut neurons = neurons_for(&genes);
        perceive(
            &sensors,
            &fixture.agent(0, 0.0),
            &fixture.view(),
            &mut neurons,
        );
        assert_eq!(neurons[0].input, 1.0, "energy should reach channel 0");
        assert!(
            neurons[1..].iter().all(|n| n.input == 0.0),
            "a spare channel wrote into a neuron"
        );
    }

    #[test]
    fn an_eye_sees_what_is_in_front_and_not_what_is_behind() {
        let genes = one_sensor(Modality::VisionRay, [0.0, 0.0, 60.0, 0.5]);
        let sensors = compile_sensors(&genes);
        // Observer at the origin-ish, target 20 units east.
        let fixture = Fixture::new(vec![
            Vec3::new(500.0, 500.0, 0.0),
            Vec3::new(520.0, 500.0, 0.0),
        ]);

        let mut looking_east = neurons_for(&genes);
        perceive(
            &sensors,
            &fixture.agent(0, 0.0),
            &fixture.view(),
            &mut looking_east,
        );
        assert!(looking_east[0].input > 0.0, "saw nothing to the east");
        assert!((looking_east[1].input - 0.25).abs() < 1e-6, "signature r");
        assert!((looking_east[2].input - 0.5).abs() < 1e-6, "signature g");
        assert!((looking_east[3].input - 0.75).abs() < 1e-6, "signature b");

        let mut looking_west = neurons_for(&genes);
        perceive(
            &sensors,
            &fixture.agent(0, core::f32::consts::PI),
            &fixture.view(),
            &mut looking_west,
        );
        assert_eq!(
            looking_west[0].input, 0.0,
            "saw through the back of its head"
        );
    }

    #[test]
    fn nearness_falls_off_with_distance() {
        // Reported as nearness, not distance: an empty view and a very distant object
        // must read the same, and a raw distance would saturate any neuron it reached.
        let genes = one_sensor(Modality::VisionRay, [0.0, 0.0, 60.0, 0.5]);
        let sensors = compile_sensors(&genes);
        let near = Fixture::new(vec![
            Vec3::new(500.0, 500.0, 0.0),
            Vec3::new(510.0, 500.0, 0.0),
        ]);
        let far = Fixture::new(vec![
            Vec3::new(500.0, 500.0, 0.0),
            Vec3::new(550.0, 500.0, 0.0),
        ]);

        let read = |f: &Fixture| {
            let mut neurons = neurons_for(&genes);
            perceive(&sensors, &f.agent(0, 0.0), &f.view(), &mut neurons);
            neurons[0].input
        };
        let (n, d) = (read(&near), read(&far));
        assert!(n > d, "nearness did not fall off: {n} vs {d}");
        assert!((0.0..=1.0).contains(&n) && (0.0..=1.0).contains(&d));
    }

    #[test]
    fn nothing_beyond_range_is_visible() {
        let genes = one_sensor(Modality::VisionRay, [0.0, 0.0, 30.0, 0.5]);
        let sensors = compile_sensors(&genes);
        let fixture = Fixture::new(vec![
            Vec3::new(500.0, 500.0, 0.0),
            Vec3::new(560.0, 500.0, 0.0),
        ]);
        let mut neurons = neurons_for(&genes);
        perceive(
            &sensors,
            &fixture.agent(0, 0.0),
            &fixture.view(),
            &mut neurons,
        );
        assert_eq!(neurons[0].input, 0.0, "saw past its own range");
    }

    #[test]
    fn an_eye_reports_the_nearest_hit_not_the_first_found() {
        // Grid order is not distance order. Reporting whichever neighbour the cell walk
        // reached first would make what an agent sees depend on the hash's layout.
        let genes = one_sensor(Modality::VisionRay, [0.0, 0.0, 60.0, 0.5]);
        let sensors = compile_sensors(&genes);
        let mut fixture = Fixture::new(vec![
            Vec3::new(500.0, 500.0, 0.0),
            Vec3::new(540.0, 500.0, 0.0),
            Vec3::new(510.0, 500.0, 0.0),
        ]);
        fixture.signatures[1] = Vec3::new(1.0, 0.0, 0.0);
        fixture.signatures[2] = Vec3::new(0.0, 1.0, 0.0);

        let mut neurons = neurons_for(&genes);
        perceive(
            &sensors,
            &fixture.agent(0, 0.0),
            &fixture.view(),
            &mut neurons,
        );
        assert_eq!(neurons[1].input, 0.0, "reported the far agent's red");
        assert_eq!(neurons[2].input, 1.0, "should have reported the near green");
    }

    #[test]
    fn an_agent_cannot_see_itself() {
        // Its own body sits at distance zero, which would be a permanent hit at
        // nearness 1 and would blind the eye to everything else.
        let genes = one_sensor(Modality::VisionRay, [0.0, 0.0, 60.0, 0.5]);
        let sensors = compile_sensors(&genes);
        let fixture = Fixture::new(vec![Vec3::new(500.0, 500.0, 0.0)]);
        let mut neurons = neurons_for(&genes);
        perceive(
            &sensors,
            &fixture.agent(0, 0.0),
            &fixture.view(),
            &mut neurons,
        );
        assert_eq!(neurons[0].input, 0.0, "the agent saw itself");
    }

    #[test]
    fn vision_crosses_the_wrap_seam() {
        // The world is a torus. An agent near one edge must see a neighbour near the
        // other, or the seam is a wall that only exists for eyes (spec §2.3).
        let genes = one_sensor(Modality::VisionRay, [0.0, 0.0, 60.0, 0.5]);
        let sensors = compile_sensors(&genes);
        let fixture = Fixture::new(vec![
            Vec3::new(990.0, 500.0, 0.0),
            Vec3::new(10.0, 500.0, 0.0),
        ]);
        let mut neurons = neurons_for(&genes);
        perceive(
            &sensors,
            &fixture.agent(0, 0.0),
            &fixture.view(),
            &mut neurons,
        );
        assert!(neurons[0].input > 0.0, "did not see across the seam");
    }

    /// A fixture whose plants sit exactly where asked, rather than scattered at random.
    fn with_plants(agent_at: Vec3, plants_at: &[Vec3]) -> Fixture {
        let mut fixture = Fixture::new(vec![agent_at]);
        let mut params = SimParams::default();
        params.plants.max_plants = plants_at.len() as u32;
        let mut grown = Plants::new(&params, &mut crate::rng::Rng::from_seed(1));
        grown.place_for_test(plants_at);
        fixture.plants = grown;
        fixture
    }

    #[test]
    fn an_eye_sees_plants_and_not_only_agents() {
        // The M7 decision, made explicit. If an eye queried only the agent grid, three
        // quarters of the sensor budget would buy nothing for the whole of Phase 1 and
        // no operator exists to shed it — see `PlantParams::signature`.
        let genes = one_sensor(Modality::VisionRay, [0.0, 0.0, 60.0, 0.5]);
        let sensors = compile_sensors(&genes);
        let fixture = with_plants(
            Vec3::new(500.0, 500.0, 0.0),
            &[Vec3::new(525.0, 500.0, 0.0)],
        );
        let mut neurons = neurons_for(&genes);
        perceive(
            &sensors,
            &fixture.agent(0, 0.0),
            &fixture.view(),
            &mut neurons,
        );

        assert!(neurons[0].input > 0.0, "the eye did not see the plant");
        let green = Vec3::from(SimParams::default().plants.signature);
        assert!((neurons[1].input - green.x).abs() < 1e-6, "plant red");
        assert!((neurons[2].input - green.y).abs() < 1e-6, "plant green");
        assert!((neurons[3].input - green.z).abs() < 1e-6, "plant blue");
    }

    #[test]
    fn an_eye_sees_a_corpse_in_its_own_colour() {
        let genes = one_sensor(Modality::VisionRay, [0.0, 0.0, 60.0, 0.5]);
        let sensors = compile_sensors(&genes);
        let mut fixture = Fixture::new(vec![Vec3::new(500.0, 500.0, 0.0)]);
        let params = SimParams::default();
        let (mut energy, mut reserve) = (100.0f32, 0.0f64);
        fixture.corpses.leave(
            Vec3::new(525.0, 500.0, 0.0),
            &mut energy,
            &mut reserve,
            &params.corpses,
        );
        fixture.corpses.settle();
        let mut neurons = neurons_for(&genes);
        perceive(
            &sensors,
            &fixture.agent(0, 0.0),
            &fixture.view(),
            &mut neurons,
        );

        assert!(neurons[0].input > 0.0, "the eye did not see the corpse");
        let colour = Vec3::from(params.corpses.signature);
        assert!((neurons[1].input - colour.x).abs() < 1e-6);
        assert!((neurons[2].input - colour.y).abs() < 1e-6);
        assert!((neurons[3].input - colour.z).abs() < 1e-6);
    }

    #[test]
    fn the_nearest_hit_wins_across_both_populations() {
        // Agents and plants live in separate grids. Taking the nearest within each and
        // then whichever was queried last would make what an agent sees depend on the
        // order the two queries happen to run in.
        let genes = one_sensor(Modality::VisionRay, [0.0, 0.0, 60.0, 0.5]);
        let sensors = compile_sensors(&genes);

        // Plant nearer than the other agent: the plant should win.
        let mut near_plant = Fixture::new(vec![
            Vec3::new(500.0, 500.0, 0.0),
            Vec3::new(540.0, 500.0, 0.0),
        ]);
        let mut params = SimParams::default();
        params.plants.max_plants = 1;
        let mut grown = Plants::new(&params, &mut crate::rng::Rng::from_seed(1));
        grown.place_for_test(&[Vec3::new(515.0, 500.0, 0.0)]);
        near_plant.plants = grown;

        let mut neurons = neurons_for(&genes);
        perceive(
            &sensors,
            &near_plant.agent(0, 0.0),
            &near_plant.view(),
            &mut neurons,
        );
        let green = Vec3::from(params.plants.signature);
        assert!(
            (neurons[2].input - green.y).abs() < 1e-6,
            "reported the farther agent instead of the nearer plant"
        );

        // Agent nearer than the plant: the agent should win.
        let mut near_agent = Fixture::new(vec![
            Vec3::new(500.0, 500.0, 0.0),
            Vec3::new(510.0, 500.0, 0.0),
        ]);
        let mut grown = Plants::new(&params, &mut crate::rng::Rng::from_seed(1));
        grown.place_for_test(&[Vec3::new(540.0, 500.0, 0.0)]);
        near_agent.plants = grown;

        let mut neurons = neurons_for(&genes);
        perceive(
            &sensors,
            &near_agent.agent(0, 0.0),
            &near_agent.view(),
            &mut neurons,
        );
        assert!(
            (neurons[2].input - 0.5).abs() < 1e-6,
            "reported the farther plant instead of the nearer agent"
        );
    }

    #[test]
    fn a_nose_reads_concentration_and_the_way_uphill() {
        // The field is diffused first, because that is the only state a nose ever meets
        // in a running world: a raw deposit sits in one cell and is invisible from the
        // next one over.
        let genes = one_sensor(Modality::Chemo, [0.0, 40.0, 0.0, 0.0]);
        let sensors = compile_sensors(&genes);
        let mut fixture = Fixture::new(vec![Vec3::new(400.0, 500.0, 0.0)]);
        let params = ChemoParams {
            decay: vec![1.0],
            diffuse: 0.5,
            ..ChemoParams::default()
        };
        fixture
            .field
            .deposit(0, Vec3::new(460.0, 500.0, 0.0), 5_000.0);
        for _ in 0..200 {
            fixture.field.update(&params);
        }

        let mut neurons = neurons_for(&genes);
        let agent = fixture.agent(0, 0.0);
        perceive(&sensors, &agent, &fixture.view(), &mut neurons);
        let raw = fixture.field.sample(0, agent.position);
        assert!(raw > 0.0, "the fixture laid down nothing to smell");
        assert!(
            (neurons[0].input - raw / (1.0 + raw)).abs() < 1e-6,
            "strength should saturate rather than report a raw concentration"
        );
        assert!(
            neurons[0].input < 1.0,
            "strength must stay bounded: {}",
            neurons[0].input
        );
        assert!(
            neurons[1].input > 0.0,
            "uphill should read straight ahead toward the source: {}",
            neurons[1].input
        );
        // Not zero, and it should not be: the gradient is a difference of two grid
        // cells, so when the sample points straddle a cell boundary unevenly they sit
        // at slightly different distances from the peak. The quantisation is real and
        // an evolved nose has to cope with it; what matters is that it is small beside
        // the direction that actually points at food.
        assert!(
            neurons[1].input > neurons[2].input.abs() * 3.0,
            "source is due east, but the gradient reads {} east / {} north",
            neurons[1].input,
            neurons[2].input
        );
    }

    #[test]
    fn an_imported_eye_turns_with_its_body_whatever_its_azimuth() {
        // An import may carry any finite azimuth. At 2^26 a quarter turn of yaw added to
        // it rounded away, so the eye kept looking one way however its agent turned.
        use core::f32::consts::FRAC_PI_2;
        let azimuth = 67_108_864.0f32;
        let aim = math::reduce_angle(azimuth);
        let genes = one_sensor(Modality::VisionRay, [azimuth, 0.0, 60.0, 0.5]);
        let sensors = compile_sensors(&genes);
        let sees = |yaw: f32, target: f32| {
            let heading = target + aim;
            let fixture = Fixture::new(vec![
                Vec3::new(500.0, 500.0, 0.0),
                Vec3::new(
                    500.0 + 20.0 * math::cos(heading),
                    500.0 + 20.0 * math::sin(heading),
                    0.0,
                ),
            ]);
            let mut neurons = neurons_for(&genes);
            perceive(
                &sensors,
                &fixture.agent(0, yaw),
                &fixture.view(),
                &mut neurons,
            );
            neurons[0].input > 0.0
        };
        assert!(sees(0.0, 0.0));
        assert!(!sees(FRAC_PI_2, 0.0), "the eye did not turn with its body");
        assert!(sees(FRAC_PI_2, FRAC_PI_2));
    }

    #[test]
    fn an_interoceptor_reads_the_agents_own_energy() {
        let genes = one_sensor(Modality::Interoception, [0.0; GENE_PARAMS]);
        let sensors = compile_sensors(&genes);
        let fixture = Fixture::new(vec![Vec3::new(500.0, 500.0, 0.0)]);
        let mut agent = fixture.agent(0, 0.0);
        agent.energy_tanks = 0.42;
        let mut neurons = neurons_for(&genes);
        perceive(&sensors, &agent, &fixture.view(), &mut neurons);
        assert_eq!(neurons[0].input, 0.42);
    }

    #[test]
    fn perception_adds_rather_than_overwrites() {
        // Two channels of a mutated genome may bind to one neuron, and summing is what
        // the CTRNN equation already says a neuron does with two inputs (spec §3.2).
        let genes = one_sensor(Modality::Interoception, [0.0; GENE_PARAMS]);
        let sensors = compile_sensors(&genes);
        let fixture = Fixture::new(vec![Vec3::new(500.0, 500.0, 0.0)]);
        let mut neurons = neurons_for(&genes);
        neurons[0].input = 5.0;
        perceive(
            &sensors,
            &fixture.agent(0, 0.0),
            &fixture.view(),
            &mut neurons,
        );
        assert_eq!(neurons[0].input, 6.0);
    }

    #[test]
    fn a_sensor_whose_target_vanished_is_not_compiled() {
        // Phase 2 can delete a neuron. The organ that pointed at it must drop out
        // rather than resolve to whatever now sits in that slot.
        let mut genes = one_sensor(Modality::Interoception, [0.0; GENE_PARAMS]);
        for gene in genes.iter_mut() {
            if let Gene::Sensor(s) = gene {
                s.targets[0] = InnovationId::new(999);
            }
        }
        assert_eq!(validate(&genes), Err(GenomeError::DanglingReference));
        assert_eq!(sensor_count(&genes), 0, "a dangling sensor was compiled");
    }

    #[test]
    fn perception_is_deterministic_across_hash_rebuilds() {
        // Neighbours arrive in grid order, and the grid is rebuilt every tick. If a
        // reading depended on that order it would be stable within a run and different
        // across a resume, which is the worst kind of determinism bug (spec §7.4).
        let genes = one_sensor(Modality::VisionRay, [0.0, 0.0, 60.0, 1.5]);
        let sensors = compile_sensors(&genes);
        let mut fixture = Fixture::new(vec![
            Vec3::new(500.0, 500.0, 0.0),
            Vec3::new(515.0, 505.0, 0.0),
            Vec3::new(512.0, 495.0, 0.0),
            Vec3::new(530.0, 500.0, 0.0),
        ]);
        let read = |f: &Fixture| {
            let mut neurons = neurons_for(&genes);
            perceive(&sensors, &f.agent(0, 0.0), &f.view(), &mut neurons);
            neurons.iter().map(|n| n.input).collect::<Vec<_>>()
        };
        let first = read(&fixture);
        let mut grid_cell = vec![0u32; fixture.positions.len()];
        for _ in 0..5 {
            fixture.hash.rebuild(
                &fixture.positions,
                fixture.pool.alive_flags(),
                &mut grid_cell,
            );
            assert_eq!(read(&fixture), first);
        }
    }

    #[test]
    fn an_unfocused_eye_still_only_sees_forward() {
        // fov is the full cone width, so half of it is the limit on either side. Getting
        // that factor wrong doubles or halves every field of view in the world.
        let narrow = compile_sensors(&one_sensor(Modality::VisionRay, [0.0, 0.0, 60.0, 0.2]));
        let wide = compile_sensors(&one_sensor(Modality::VisionRay, [0.0, 0.0, 60.0, 3.0]));
        let genes = one_sensor(Modality::VisionRay, [0.0, 0.0, 60.0, 0.2]);
        // A target off to one side, well outside a body radius of the ray line.
        let fixture = Fixture::new(vec![
            Vec3::new(500.0, 500.0, 0.0),
            Vec3::new(530.0, 525.0, 0.0),
        ]);
        let read = |sensors: &[Sensor]| {
            let mut neurons = neurons_for(&genes);
            perceive(
                sensors,
                &fixture.agent(0, 0.0),
                &fixture.view(),
                &mut neurons,
            );
            neurons[0].input
        };
        assert_eq!(
            read(&narrow),
            0.0,
            "a narrow eye saw something 40° off-axis"
        );
        assert!(read(&wide) > 0.0, "a wide eye missed it");
    }
}
