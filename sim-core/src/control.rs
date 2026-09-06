//! Experiment modes that change heredity without changing the world's parameters.
//!
//! The random-brain control is a separate world with the same seed and [`SimParams`].
//! Its offspring receive freshly randomized neural scalars instead of inheriting a
//! mutated parent brain, so behavior that also appears there is not evidence of
//! cumulative neural evolution (spec §7.8, §10).

/// How neural scalars are assigned to offspring.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum BrainInheritance {
    /// Inherit the parent's genome and apply the configured mutation operators.
    #[default]
    Evolving = 0,
    /// Keep non-neural genes but redraw weights, biases, taus, and oscillator periods.
    RandomizedAtBirth = 1,
}
