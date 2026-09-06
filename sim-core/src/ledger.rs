//! The energy ledger: every joule that entered the world, and every joule that left.
//!
//! Spec §5.1 makes the whole economy rest on one claim — energy enters at a fixed rate
//! and leaves only through metabolic dissipation. That claim is worth nothing unless it
//! is *measured*, because the ways it breaks are all silent: a birth that hands the
//! offspring a full tank without charging the parent, a plant that regrows what it was
//! eaten, a rounding error that rounds up. Each one makes every strategy viable and
//! makes selection stop being real, and none of them fails a functional test.
//!
//! So the two flows are counted at the moment they happen and compared against the
//! stock actually present. What must hold, always:
//!
//! ```text
//! stock_now  ==  stock_at_start + input - dissipated
//! ```
//!
//! Transfers do not appear here at all, and that is the point: compensated energy
//! stores preserve ownership while moving stock, so a transfer neither enters nor
//! leaves the economy.
//!
//! Deliberately not here: what charges or credits anything. This module counts.

use serde::{Deserialize, Serialize};

use crate::energy::Amount;

/// Running totals for one world.
///
/// `f64` accumulators over `f32` quantities, deliberately. A 10k-tick run at the
/// default population dissipates on the order of tens of millions of joules, and `f32`
/// carries about seven significant digits — long before the run ends, adding a single
/// agent's 0.4 to the total would round to no change at all and the ledger would
/// silently stop counting. The simulation stays `f32` for determinism; the measurement
/// of it does not have to.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EnergyLedger {
    input: f64,
    dissipated: f64,
    initial_stock: f64,
}

impl EnergyLedger {
    /// Starts a ledger against the energy already present.
    pub fn opening(stock: f64) -> Self {
        Self {
            input: 0.0,
            dissipated: 0.0,
            initial_stock: stock,
        }
    }

    /// Energy that entered the world. Only plants absorbing the input rate may call
    /// this (spec §5.1).
    #[inline]
    pub fn record_input(&mut self, amount: f64) {
        debug_assert!(amount >= 0.0, "negative input: {amount}");
        self.input += amount;
    }

    /// Energy that left the world, through metabolism or any other sink.
    #[inline]
    pub fn record_dissipated(&mut self, amount: f64) {
        debug_assert!(amount >= 0.0, "negative dissipation: {amount}");
        self.dissipated += amount;
    }

    #[inline]
    pub(crate) fn record_dissipated_amount(&mut self, amount: Amount) {
        let total = amount.approximate();
        debug_assert!(total >= 0.0, "negative dissipation: {total}");
        self.dissipated += total;
    }

    #[inline]
    pub fn input(&self) -> f64 {
        self.input
    }

    #[inline]
    pub fn dissipated(&self) -> f64 {
        self.dissipated
    }

    /// What the stock should be if nothing leaked.
    #[inline]
    pub fn expected_stock(&self) -> f64 {
        self.initial_stock + self.input - self.dissipated
    }

    /// How far the world's actual energy has drifted from what the ledger accounts for.
    ///
    /// Positive means energy appeared from nowhere; negative means it vanished without
    /// being dissipated. Either is a broken economy, and the sign says which mistake to
    /// look for.
    #[inline]
    pub fn drift(&self, stock: f64) -> f64 {
        stock - self.expected_stock()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_untouched_world_has_not_drifted() {
        let ledger = EnergyLedger::opening(1_000.0);
        assert_eq!(ledger.drift(1_000.0), 0.0);
        assert_eq!(ledger.expected_stock(), 1_000.0);
    }

    #[test]
    fn input_and_dissipation_move_the_expected_stock() {
        let mut ledger = EnergyLedger::opening(100.0);
        ledger.record_input(50.0);
        ledger.record_dissipated(30.0);
        assert_eq!(ledger.expected_stock(), 120.0);
        assert_eq!(ledger.drift(120.0), 0.0);
    }

    #[test]
    fn drift_names_the_direction_of_the_mistake() {
        let mut ledger = EnergyLedger::opening(100.0);
        ledger.record_input(10.0);
        // Energy conjured: more in the world than the ledger accounts for.
        assert!(ledger.drift(150.0) > 0.0);
        // Energy vanished without passing through a sink.
        assert!(ledger.drift(50.0) < 0.0);
    }

    #[test]
    fn a_transfer_is_invisible_to_the_ledger() {
        // Moving energy between a plant and an agent, or a parent and a child, changes
        // nothing about how much the world holds. That is exactly why a transfer
        // written wrongly shows up as drift without anyone predicting the mistake.
        let mut ledger = EnergyLedger::opening(100.0);
        ledger.record_input(20.0);
        let stock = 120.0;
        assert_eq!(ledger.drift(stock), 0.0);
        // Ten joules move from one holder to another; the total is unchanged.
        let (a, b) = (stock - 10.0, 10.0);
        assert_eq!(ledger.drift(a + b), 0.0);
    }

    #[test]
    fn small_charges_still_register_against_a_large_total() {
        // The reason the accumulators are f64. A 10k-tick run dissipates tens of
        // millions of joules, and in f32 a single agent's fraction would round to no
        // change at all — the ledger would agree with itself while counting nothing.
        let mut ledger = EnergyLedger::opening(0.0);
        for _ in 0..20_000_000 {
            ledger.record_dissipated(0.4);
        }
        let total = ledger.dissipated();
        ledger.record_dissipated(0.4);
        assert!(
            ledger.dissipated() > total,
            "a small charge vanished into a large total"
        );
        assert!(
            (total - 8_000_000.0).abs() < 1.0,
            "accumulated {total}, expected 8000000"
        );
    }
}
