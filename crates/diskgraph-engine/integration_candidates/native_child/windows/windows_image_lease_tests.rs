#![cfg(windows)]

#[path = "windows_image_lease_probe.rs"]
mod windows_image_lease_probe;

use serde_json::Value;
use windows_image_lease_probe::WindowsImageLeaseProbe;

fn run(case_id: u32) -> Value {
    let (status, record) = WindowsImageLeaseProbe::run(case_id);
    assert!(
        status.success(),
        "native qualification or capability blocked: {record}"
    );
    assert_eq!(record["case"], case_id);
    assert_eq!(record["win32_error"], 0);
    assert_eq!(record["cleanup_error"], 0);
    assert_eq!(record["launches"], record["actual_waits"]);
    assert_eq!(record["launches"], record["empty_jobs"]);
    assert!(record["launches"].as_u64().is_some_and(|count| count >= 2));
    record
}

#[test]
fn real_same_source_a_b_images_and_unleased_operations_are_qualified() {
    let record = run(1);
    assert_eq!(record["classification"], "fixture_qualified");
}

#[test]
fn leaf_read_lease_denies_write_delete_and_replacement_until_actual_a_load() {
    let record = run(2);
    assert_eq!(record["classification"], "leaf_write_delete_blocked");
    assert_eq!(record["same_file"], true);
    assert_eq!(record["write_open_error"], 32);
    assert_eq!(record["delete_open_error"], 32);
    assert_eq!(record["loaded_marker"], "A");
}

#[test]
fn a_live_original_writer_prevents_reopen_and_release_has_an_independent_positive() {
    let record = run(3);
    assert_eq!(record["classification"], "live_writer_rejected");
    assert_eq!(record["lease_error"], 32);
    assert_eq!(record["writer_closed"], true);
    assert_eq!(record["same_file"], true);
    assert_eq!(record["loaded_marker"], "A");
}

fn require_mapping_stability(record: &Value) {
    assert_eq!(record["writer_closed"], true);
    assert_eq!(record["mapping_handle_closed"], true);
    assert_eq!(
        record["view_changed"], false,
        "mutable mapped bytes are always blocking"
    );
    let rejected = record["classification"] == "mapped_writer_admission_rejected";
    let protected = record["classification"] == "existing_view_write_blocked";
    assert!(
        rejected || protected,
        "no proven writable-view admission or store protection: {record}"
    );
    if protected {
        assert_eq!(record["exception_code"], 0xc000_0005u32);
        assert_eq!(record["loaded_marker"], "A");
    }
}

#[test]
fn retained_writable_view_cannot_mutate_after_a_successful_reopen_lease() {
    require_mapping_stability(&run(4));
}

#[test]
fn adding_sec_image_cannot_hide_mutability_of_a_retained_writable_view() {
    require_mapping_stability(&run(5));
}

#[test]
fn leaf_only_lease_ancestor_rebinding_is_characterized_as_a_gap() {
    let record = run(6);
    assert_eq!(record["classification"], "characterized_gap");
    assert_eq!(record["same_file"], true);
    assert_eq!(record["route_changed"], true);
    assert_eq!(record["loaded_marker"], "B");
    assert_eq!(record["security_acceptance"], false);
}

#[test]
fn leaf_only_lease_junction_rebinding_is_characterized_as_a_gap() {
    let record = run(7);
    assert_eq!(record["classification"], "characterized_gap");
    assert_eq!(record["same_file"], true);
    assert_eq!(record["route_changed"], true);
    assert_eq!(record["loaded_marker"], "B");
    assert_eq!(record["security_acceptance"], false);
}
