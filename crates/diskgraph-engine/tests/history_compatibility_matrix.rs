//! D35 / Q-04 上层矩阵：真实公开发布/授权查询，除 native 命名用例外均为合成旧元数据。
#[path = "history_compatibility/fixture.rs"]
mod fixture;

use diskgraph_core::{DifferentReason, Evidence, NodeKind, ResourceLocator, ScanSettings, Verdict};
use diskgraph_store::{SqliteSnapshotStore, StoreError};
use fixture::HistoryFixture;
use std::path::Path;

fn assert_incompatible(f: &HistoryFixture, expected: &str) {
    assert!(f.growth("before", "after", "item").is_none());
    assert!(
        f.engine
            .growth_between("before", "after", Path::new("item"))
            .unwrap()
            .is_none()
    );
    for changes in [
        f.changes("before", "after"),
        f.engine.revision_changes("before", "after").unwrap(),
    ] {
        assert_eq!(changes["incompatible"], expected, "{changes}");
        assert_eq!(changes["added"], 0);
        assert_eq!(changes["removed"], 0);
        assert_eq!(changes["size_changed"], 0);
        assert_eq!(changes["complete"], true);
        assert_eq!(changes["summary_is_partial"], false);
    }
}

#[test]
fn imported_same_owner_root_and_volume_keep_signed_growth_and_change_counts() {
    for (bytes, delta, count) in [(100, 0, 0), (200, 100, 1), (0, -100, 1)] {
        let f = HistoryFixture::new();
        let before = f.graph("before", 1, 100);
        let mut after = f.graph("after", 2, bytes);
        after.nodes[1].file_identity.as_mut().unwrap().file_id = 99;
        f.publish(&before).unwrap();
        f.publish(&after).unwrap();
        assert_eq!(
            f.growth("before", "after", "item").unwrap().delta_bytes,
            delta
        );
        assert_eq!(
            f.engine
                .growth_between("before", "after", Path::new("item"))
                .unwrap()
                .unwrap()
                .delta_bytes,
            delta
        );
        let changes = f.changes("before", "after");
        assert!(changes["incompatible"].is_null());
        assert_eq!(changes["size_changed"], count);
        assert_eq!(changes["added"], 0);
        assert_eq!(changes["removed"], 0);
    }
}

#[test]
fn imported_volume_or_provider_domain_changes_are_not_silently_compared() {
    for (volume, expected) in [
        (Some("other-import-volume"), "different_volume"),
        (Some("provider:other-account"), "different_volume"),
        (None, "unknown_volume"),
    ] {
        for bad_left in [false, true] {
            let f = HistoryFixture::new();
            let mut before = f.graph("before", 1, 100);
            let mut after = f.graph("after", 2, 200);
            let target = if bad_left { &mut before } else { &mut after };
            target.snapshot.volume_id = volume.map(str::to_owned);
            f.publish(&before).unwrap();
            f.publish(&after).unwrap();
            assert_incompatible(&f, expected);
        }
    }
}

#[test]
fn every_recorded_scan_setting_difference_refuses_growth_and_changes() {
    // 为同一设置名及其变换使用明确的夹具类型，不改变逐项双侧验收。
    type SettingChange = (&'static str, fn(&mut ScanSettings));
    let variants: [SettingChange; 6] = [
        ("apparent_size", |s| s.apparent_size = false),
        ("follow_links", |s| s.follow_links = true),
        ("include_hidden", |s| s.include_hidden = false),
        ("one_filesystem", |s| s.one_filesystem = false),
        ("max_depth", |s| s.max_depth = Some(2)),
        ("dedup_hardlinks", |s| s.dedup_hardlinks = true),
    ];
    for (name, change) in variants {
        for bad_left in [false, true] {
            let f = HistoryFixture::new();
            let mut before = f.graph("before", 1, 100);
            let mut after = f.graph("after", 2, 200);
            change(if bad_left {
                &mut before.snapshot.settings
            } else {
                &mut after.snapshot.settings
            });
            assert_ne!(before.snapshot.settings, after.snapshot.settings, "{name}");
            f.publish(&before).unwrap();
            f.publish(&after).unwrap();
            assert_incompatible(&f, "different_settings");
        }
    }
}

#[test]
fn partial_unreadable_and_depth_coverage_never_claim_removed_paths() {
    for mode in 0..3 {
        for bad_left in [false, true] {
            let f = HistoryFixture::new();
            let mut before = f.graph("before", 1, 100);
            let mut after = f.graph("after", 2, 0);
            after.nodes.truncate(1);
            after.nodes[0].files = 0;
            let coverage = if bad_left {
                &mut before.snapshot.coverage
            } else {
                &mut after.snapshot.coverage
            };
            coverage.complete = false;
            coverage.unreadable_nodes = u64::from(mode == 1);
            coverage.depth_limited = mode == 2;
            f.publish(&before).unwrap();
            f.publish(&after).unwrap();
            assert_incompatible(&f, "incomplete_coverage");
        }
    }
}

#[test]
fn contradictory_complete_coverage_is_refused_by_public_publication() {
    for unreadable in [false, true] {
        let f = HistoryFixture::new();
        f.publish(&f.graph("before", 1, 100)).unwrap();
        let mut invalid = f.graph("after", 2, 200);
        invalid.snapshot.coverage.unreadable_nodes = u64::from(unreadable);
        invalid.snapshot.coverage.depth_limited = !unreadable;
        assert!(matches!(
            f.publish(&invalid),
            Err(StoreError::InvalidGraph(_))
        ));
        let reader = SqliteSnapshotStore::open_reader(
            &f.directory.path().join("data/diskgraph.sqlite"),
            30_000,
            None,
        )
        .unwrap();
        assert!(reader.revision("after").is_err());
        assert_eq!(
            f.engine.latest_revision(&f.scope).unwrap().as_deref(),
            Some("before")
        );
    }
}

#[test]
fn reverse_observation_order_has_an_explicit_incompatibility() {
    let f = HistoryFixture::new();
    f.publish(&f.graph("before", 2, 100)).unwrap();
    f.publish(&f.graph("after", 1, 200)).unwrap();
    assert_incompatible(&f, "out_of_order");
}

#[test]
fn node_quality_and_type_replacement_matrix_keeps_unknown_bytes_null() {
    for bad_left in [false, true] {
        for read_error in [false, true] {
            let f = HistoryFixture::new();
            let mut before = f.graph("before", 1, 100);
            let mut after = f.graph("after", 2, 200);
            let node = if bad_left {
                &mut before.nodes[1]
            } else {
                &mut after.nodes[1]
            };
            if read_error {
                node.read_error = true;
            } else {
                node.size_known = false;
            }
            f.publish(&before).unwrap();
            f.publish(&after).unwrap();
            assert!(f.growth("before", "after", "item").is_none());
            assert_eq!(f.changes("before", "after")["size_changed"], 0);
            let report = f.compare("before", "after");
            assert_eq!(report.summary.unknown, 1);
            let row = &report.rows[0];
            assert_eq!(
                row.verdict,
                Verdict::Different {
                    reason: DifferentReason::UnknownSize
                }
            );
            assert_eq!(row.left_bytes, if bad_left { None } else { Some(100) });
            assert_eq!(row.right_bytes, if bad_left { Some(200) } else { None });
            let wire = report.to_json(None);
            assert!(
                wire["rows"][0][if bad_left {
                    "left_bytes"
                } else {
                    "right_bytes"
                }]
                .is_null()
            );
        }
    }
    for kind in [NodeKind::Directory, NodeKind::Symlink, NodeKind::Other] {
        let f = HistoryFixture::new();
        let before = f.graph("before", 1, 100);
        let mut after = f.graph("after", 2, 200);
        after.nodes[1].kind = kind;
        f.publish(&before).unwrap();
        f.publish(&after).unwrap();
        assert!(f.growth("before", "after", "item").is_none());
        assert_eq!(f.changes("before", "after")["size_changed"], 0);
        assert_eq!(
            f.compare("before", "after").rows[0].verdict,
            Verdict::Different {
                reason: DifferentReason::Path
            }
        );
    }
}

#[test]
fn same_identity_renamed_path_is_one_addition_and_one_removal_not_growth() {
    let f = HistoryFixture::new();
    let before = f.graph("before", 1, 100);
    let mut after = f.graph("after", 2, 100);
    after.nodes[1].name = "renamed".into();
    after.nodes[1].locator =
        ResourceLocator::NativePath(f.root.join("renamed").to_str().unwrap().into());
    assert_eq!(before.nodes[1].file_identity, after.nodes[1].file_identity);
    f.publish(&before).unwrap();
    f.publish(&after).unwrap();
    for path in ["item", "renamed", "absent"] {
        assert!(f.growth("before", "after", path).is_none());
    }
    let changes = f.changes("before", "after");
    assert_eq!(changes["added"], 1);
    assert_eq!(changes["removed"], 1);
    assert_eq!(changes["size_changed"], 0);
    let report = f.compare("before", "after");
    assert_eq!(report.rows.len(), 2);
    assert!(
        report
            .rows
            .iter()
            .any(|row| row.path == "item" && row.verdict == Verdict::LeftOnly)
    );
    assert!(
        report
            .rows
            .iter()
            .any(|row| row.path == "renamed" && row.verdict == Verdict::RightOnly)
    );
}

#[test]
fn native_scan_growth_and_rename_follow_real_host_paths_without_identity_inference() {
    let f = HistoryFixture::new();
    std::fs::write(f.root.join("item"), vec![1u8; 100]).unwrap();
    let before = f.native_scan();
    std::fs::write(f.root.join("item"), vec![2u8; 200]).unwrap();
    let grown = f.native_scan();
    assert_eq!(f.growth(&before, &grown, "item").unwrap().delta_bytes, 100);
    std::fs::rename(f.root.join("item"), f.root.join("renamed")).unwrap();
    let renamed = f.native_scan();
    assert!(f.growth(&grown, &renamed, "item").is_none());
    assert!(f.growth(&grown, &renamed, "renamed").is_none());
    let changes = f.changes(&grown, &renamed);
    assert_eq!(changes["added"], 1);
    assert_eq!(changes["removed"], 1);
    assert_eq!(changes["size_changed"], 0);
    let graph = f.engine.load_revision(&renamed).unwrap();
    assert!(graph.snapshot.coverage.complete);
    assert!(graph.snapshot.volume_id.is_some());
}

#[test]
fn generic_metadata_comparison_keeps_distinct_known_identities_legal() {
    let f = HistoryFixture::new();
    let before = f.graph("before", 1, 100);
    let mut after = f.graph("after", 2, 100);
    after.nodes[1].file_identity.as_mut().unwrap().file_id = 99;
    f.publish(&before).unwrap();
    f.publish(&after).unwrap();
    let report = f.compare("before", "after");
    assert_eq!(
        report.rows[0].verdict,
        Verdict::Same {
            evidence: Evidence::Metadata
        }
    );
    assert_eq!(report.rows[0].digests, None);
    assert_eq!(
        report.to_json(None)["rows"][0]["verdict"]["evidence"],
        "metadata"
    );
}
