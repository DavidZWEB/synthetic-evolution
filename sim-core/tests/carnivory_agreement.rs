//! Native half of the carnivory mechanisms' cross-target check (research only).

#[path = "common/scenario.rs"]
mod scenario;

#[path = "common/carnivory_case.rs"]
mod carnivory_case;

#[test]
fn carnivory_mechanisms_hash_to_the_recorded_values() {
    let hashes = carnivory_case::carnivory_hashes();
    println!("carnivory hashes: {hashes:#018x?}");
    assert_eq!(hashes, carnivory_case::GOLDEN);
}
