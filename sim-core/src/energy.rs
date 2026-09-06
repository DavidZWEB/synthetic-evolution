//! Compensated energy storage for the simulation's `f32` state arrays.
//!
//! Each owner keeps a visible `f32` value plus an `f64` residual. Operations update the
//! pair directly and report the delta actually represented, so `f32`-scale rounding
//! cannot make growth, costs, or transfers disappear, appear, or change owner. The
//! residual extends rather than abolishes finite precision (spec §5.1).

pub(crate) fn total(value: f32, residual: f64) -> f64 {
    value as f64 + residual
}

#[derive(Clone, Copy)]
pub(crate) struct Amount {
    pub major: f64,
    pub minor: f64,
}

impl Amount {
    pub(crate) fn approximate(self) -> f64 {
        self.major + self.minor
    }
}

pub(crate) fn at_least(value: f32, residual: f64, threshold: f32) -> bool {
    (value as f64 - threshold as f64) + residual >= 0.0
}

pub(crate) fn add_capped(value: &mut f32, residual: &mut f64, amount: f64, ceiling: f64) -> f64 {
    // A lowered live cap limits future growth; it does not confiscate stock that was
    // valid under the previous parameters. Removal must pass through a real sink.
    let room = (ceiling - *value as f64) - *residual;
    let added = if room <= 0.0 {
        0.0
    } else {
        amount.max(0.0).min(room)
    };
    add(value, residual, added).approximate()
}

#[cfg(test)]
pub(crate) fn take(value: &mut f32, residual: &mut f64, wanted: f64) -> f64 {
    take_amount(value, residual, wanted).approximate()
}

pub(crate) fn transfer(
    source: &mut f32,
    source_residual: &mut f64,
    destination: &mut f32,
    destination_residual: &mut f64,
    wanted: f64,
) -> f64 {
    let taken = take_amount(source, source_residual, wanted);
    add(destination, destination_residual, taken.major);
    add(destination, destination_residual, taken.minor);
    taken.approximate()
}

pub(crate) fn take_amount(value: &mut f32, residual: &mut f64, wanted: f64) -> Amount {
    let wanted = wanted.max(0.0);
    let visible = *value as f64;
    if wanted < visible {
        let change = add(value, residual, -wanted);
        return Amount {
            major: -change.major,
            minor: -change.minor,
        };
    }

    let residual_wanted = wanted - visible;
    let minor = residual_wanted.min(*residual);
    let remaining = *residual - minor;
    *value = 0.0;
    *residual = 0.0;
    add(value, residual, remaining);
    Amount {
        major: visible,
        minor,
    }
}

fn add(value: &mut f32, residual: &mut f64, amount: f64) -> Amount {
    let base = *value;
    let residual_before = *residual;
    let mut offset = *residual + amount;
    if offset == 0.0 {
        *residual = 0.0;
        return Amount {
            major: 0.0,
            minor: -residual_before,
        };
    }

    let approximate = base as f64 + offset;
    let mut next = floor_to_f32(approximate.max(0.0));
    if offset < 0.0 && next == base && base > 0.0 {
        next = next_down(base);
    }
    offset += base as f64 - next as f64;

    while offset < 0.0 && next > 0.0 {
        let lower = next_down(next);
        offset += next as f64 - lower as f64;
        next = lower;
    }
    while next < f32::MAX {
        let higher = next_up(next);
        if higher <= next {
            break;
        }
        let gap = higher as f64 - next as f64;
        if offset < gap {
            break;
        }
        offset -= gap;
        next = higher;
    }

    *value = next;
    *residual = offset.max(0.0);
    Amount {
        major: next as f64 - base as f64,
        minor: *residual - residual_before,
    }
}

fn floor_to_f32(value: f64) -> f32 {
    if value >= f32::MAX as f64 {
        return f32::MAX;
    }
    let rounded = value as f32;
    if rounded as f64 > value {
        next_down(rounded)
    } else {
        rounded
    }
}

fn next_down(value: f32) -> f32 {
    if value > 0.0 {
        f32::from_bits(value.to_bits() - 1)
    } else {
        value
    }
}

fn next_up(value: f32) -> f32 {
    if (0.0..f32::MAX).contains(&value) {
        f32::from_bits(value.to_bits() + 1)
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_preserves_ownership_and_total_across_float_scales() {
        for source in [0.123_456_7, 1.0, 60.0, 1_000_000.0] {
            for destination in [0.0, 1.0, 8_000.0, 1_000_000.0] {
                for wanted in [0.01f32, 0.7, 1.0, 100.0] {
                    let mut source_value = source;
                    let mut source_residual = 0.0;
                    let mut destination_value = destination;
                    let mut destination_residual = 0.0;
                    let before = source as f64 + destination as f64;

                    let moved = transfer(
                        &mut source_value,
                        &mut source_residual,
                        &mut destination_value,
                        &mut destination_residual,
                        wanted as f64,
                    );

                    assert_eq!(
                        total(source_value, source_residual)
                            + total(destination_value, destination_residual),
                        before
                    );
                    assert_eq!(moved, wanted.min(source) as f64);
                    assert_eq!(
                        total(destination_value, destination_residual),
                        destination as f64 + moved
                    );
                }
            }
        }
    }

    #[test]
    fn requests_below_the_visible_ulp_accumulate_for_the_same_owner() {
        let mut source = 1_000_000.0;
        let mut source_residual = 0.0;
        let mut destination = 0.0;
        let mut destination_residual = 0.0;

        for _ in 0..10 {
            transfer(
                &mut source,
                &mut source_residual,
                &mut destination,
                &mut destination_residual,
                0.01,
            );
        }

        assert!((total(source, source_residual) - 999_999.9).abs() < 1e-9);
        assert!((total(destination, destination_residual) - 0.1).abs() < 1e-9);
    }

    #[test]
    fn requests_below_the_f64_ulp_change_the_compensated_balance() {
        let mut value = 10_000_000_000.0;
        let mut residual = 0.0;
        let mut removed = 0.0;

        for _ in 0..10 {
            let taken = take(&mut value, &mut residual, 1e-7);
            assert!(taken > 0.0);
            removed += taken;
        }

        assert_eq!(value, 9_999_998_976.0);
        assert_eq!(residual, 1_024.0 - removed);
        assert!(!at_least(value, residual, 10_000_000_000.0));
    }

    #[test]
    fn draining_moves_a_sub_f64_ulp_residual() {
        let mut source = 10_000_000_000.0;
        let mut source_residual = 1e-7;
        let mut destination = 0.0;
        let mut destination_residual = 0.0;

        transfer(
            &mut source,
            &mut source_residual,
            &mut destination,
            &mut destination_residual,
            f64::MAX,
        );

        assert_eq!(source, 0.0);
        assert_eq!(source_residual, 0.0);
        assert_eq!(destination, 10_000_000_000.0);
        assert_eq!(destination_residual, 1e-7);
    }

    #[test]
    fn lowering_a_cap_does_not_remove_existing_stock() {
        let mut value = 60.0;
        let mut residual = 0.0;
        let added = add_capped(&mut value, &mut residual, 1.0, 30.0);
        assert_eq!(added, 0.0);
        assert_eq!(total(value, residual), 60.0);
    }
}
