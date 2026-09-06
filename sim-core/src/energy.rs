//! Compensated energy storage for the simulation's `f32` state arrays.
//!
//! Each owner keeps a visible `f32` value plus an `f64` residual. Operations act on
//! their sum and recanonicalize it, so sub-ULP growth, costs, and transfers remain
//! owned energy rather than disappearing, appearing, or blocking progress (spec §5.1).

pub(crate) fn total(value: f32, residual: f64) -> f64 {
    value as f64 + residual
}

pub(crate) fn set(value: &mut f32, residual: &mut f64, total: f64) {
    debug_assert!(total >= 0.0 && total.is_finite());
    let visible = floor_to_f32(total);
    *value = visible;
    *residual = total - visible as f64;
    debug_assert!(*residual >= 0.0);
}

pub(crate) fn add_capped(value: &mut f32, residual: &mut f64, amount: f64, ceiling: f64) -> f64 {
    let before = total(*value, *residual);
    // A lowered live cap limits future growth; it does not confiscate stock that was
    // valid under the previous parameters. Removal must pass through a real sink.
    let after = if before >= ceiling {
        before
    } else {
        (before + amount).min(ceiling)
    };
    set(value, residual, after);
    after - before
}

pub(crate) fn take(value: &mut f32, residual: &mut f64, wanted: f64) -> f64 {
    let before = total(*value, *residual);
    let taken = wanted.max(0.0).min(before);
    set(value, residual, before - taken);
    taken
}

pub(crate) fn transfer(
    source: &mut f32,
    source_residual: &mut f64,
    destination: &mut f32,
    destination_residual: &mut f64,
    wanted: f64,
) -> f64 {
    let taken = take(source, source_residual, wanted);
    let destination_total = total(*destination, *destination_residual);
    set(destination, destination_residual, destination_total + taken);
    taken
}

fn floor_to_f32(value: f64) -> f32 {
    if value >= f32::MAX as f64 {
        return f32::MAX;
    }
    let rounded = value as f32;
    if rounded as f64 > value {
        f32::from_bits(rounded.to_bits() - 1)
    } else {
        rounded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_preserves_ownership_and_total_across_float_scales() {
        for source in [0.123_456_7, 1.0, 60.0, 1_000_000.0] {
            for destination in [0.0, 1.0, 8_000.0, 1_000_000.0] {
                for wanted in [0.01, 0.7, 1.0, 100.0] {
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
                        wanted,
                    );

                    assert_eq!(
                        total(source_value, source_residual)
                            + total(destination_value, destination_residual),
                        before
                    );
                    assert_eq!(moved, wanted.min(source as f64));
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
    fn lowering_a_cap_does_not_remove_existing_stock() {
        let mut value = 60.0;
        let mut residual = 0.0;
        let added = add_capped(&mut value, &mut residual, 1.0, 30.0);
        assert_eq!(added, 0.0);
        assert_eq!(total(value, residual), 60.0);
    }
}
