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
fn ordinary_ancestor_rename_is_characterized_with_same_operation_controls() {
    let record = run(6);
    assert_eq!(record["ancestor_prelease_qualified"], true);
    if record["classification"] == "ancestor_rename_blocked_by_leaf_lease" {
        assert!(matches!(
            record["ancestor_rename_error"].as_u64(),
            Some(5 | 32)
        ));
        assert_eq!(record["route_unchanged_under_lease"], true);
        assert_eq!(record["loaded_a_under_lease"], true);
        assert_eq!(record["released_rename_succeeded"], true);
        assert_eq!(record["loaded_b_after_release"], true);
    } else {
        assert_eq!(record["classification"], "characterized_gap");
        assert_eq!(record["ancestor_rename_error"], 0);
        assert_eq!(record["released_rename_succeeded"], false);
    }
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

#[test]
fn resolved_kernel_route_loads_original_a_after_original_junction_loads_b() {
    let record = run(8);
    assert_eq!(
        record["classification"],
        "resolved_kernel_route_survived_junction"
    );
    assert_eq!(record["kernel_route_qualified"], true);
    assert_eq!(record["loaded_b_via_original_route"], true);
    assert_eq!(record["kernel_route_unchanged_after_rebind"], true);
    assert_eq!(record["loaded_a_via_kernel_route"], true);
    assert_eq!(record["same_file"], true);
    assert_eq!(record["route_changed"], false);
    assert_eq!(record["loaded_marker"], "A");
    assert_eq!(record["security_acceptance"], false);
}
