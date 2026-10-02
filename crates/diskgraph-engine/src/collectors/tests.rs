use super::*;
use diskgraph_core::{DiskGraph, EntityKind, Relation};
use diskgraph_core::{
    DiskNode, DiskSnapshot, NodeKind, ResourceLocator, ScanCoverage, ScanSettings,
};

fn node(id: u64, parent: Option<u64>, name: &str, kind: NodeKind) -> DiskNode {
    DiskNode {
        id,
        parent_id: parent,
        locator: ResourceLocator::NativePath(format!("/tmp/{name}")),
        name: name.to_owned(),
        kind,
        subtree_bytes: 10,
        size_known: true,
        direct_bytes: 10,
        files: 1,
        directories: 0,
        modified_unix_seconds: None,
        file_identity: None,
        category_hint: None,
        reclaim_hint: None,
        read_error: false,
    }
}

fn graph(nodes: Vec<DiskNode>) -> DiskGraph {
    DiskGraph {
        snapshot: DiskSnapshot {
            id: "snap-1".into(),
            root: ResourceLocator::NativePath("/tmp".into()),
            volume_id: Some("v".into()),
            captured_at_unix_ms: 1,
            settings: ScanSettings {
                apparent_size: false,
                follow_links: false,
                include_hidden: true,
                one_filesystem: true,
                max_depth: None,
                dedup_hardlinks: true,
            },
            coverage: ScanCoverage {
                complete: true,
                unreadable_nodes: 0,
                depth_limited: false,
            },
        },
        nodes,
        evidence: vec![],
    }
}

#[test]
fn cargo_target_next_to_manifest_produces_ownership_and_rebuildable() {
    let graph = graph(vec![
        node(1, None, "proj", NodeKind::Directory),
        node(2, Some(1), "Cargo.toml", NodeKind::File),
        node(3, Some(1), "target", NodeKind::Directory),
    ]);
    let batch = collect_projects(&graph);
    let kinds: Vec<_> = batch.edges.iter().map(|edge| edge.relation).collect();
    assert!(kinds.contains(&Relation::OwnedByProject));
    assert!(kinds.contains(&Relation::RebuildableBy));
    assert!(batch.evidence.iter().all(|record| record.confidence <= 100));
}

#[test]
fn a_target_directory_without_a_sibling_manifest_stays_unclaimed() {
    let graph = graph(vec![
        node(1, None, "plain", NodeKind::Directory),
        node(2, Some(1), "target", NodeKind::Directory),
    ]);
    let batch = collect_projects(&graph);
    assert!(batch.edges.is_empty());
    assert!(batch.evidence.is_empty());
}

#[test]
fn node_modules_is_owned_but_not_marked_rebuildable_in_rule_v1() {
    let graph = graph(vec![
        node(1, None, "webapp", NodeKind::Directory),
        node(2, Some(1), "package.json", NodeKind::File),
        node(3, Some(1), "node_modules", NodeKind::Directory),
    ]);
    let batch = collect_projects(&graph);
    let kinds: Vec<_> = batch.edges.iter().map(|edge| edge.relation).collect();
    assert!(kinds.contains(&Relation::OwnedByProject));
    assert!(!kinds.contains(&Relation::RebuildableBy));
}

#[test]
fn batches_pass_endpoint_validation_including_resource_entities() {
    let graph = graph(vec![
        node(1, None, "proj", NodeKind::Directory),
        node(2, Some(1), "Cargo.toml", NodeKind::File),
        node(3, Some(1), "target", NodeKind::Directory),
    ]);
    let batch = collect_projects(&graph);
    let entity_ids: Vec<(String, EntityKind)> = batch
        .entities
        .iter()
        .map(|entity| (entity.entity_id.clone(), entity.kind))
        .collect();
    // Every edge endpoint must exist inside the same batch.
    diskgraph_core::validate_edges(&batch.edges, &entity_ids).unwrap();
    // Both edges cite the same evidence record; it must exist.
    for edge in &batch.edges {
        for (evidence_id, _) in &edge.evidence_refs {
            assert!(
                batch
                    .evidence
                    .iter()
                    .any(|record| &record.evidence_id == evidence_id),
                "dangling evidence {evidence_id}"
            );
        }
    }
}

#[test]
fn repeated_outputs_of_one_project_deduplicate_entities() {
    let graph = graph(vec![
        node(1, None, "proj", NodeKind::Directory),
        node(2, Some(1), "Cargo.toml", NodeKind::File),
        node(3, Some(1), "target", NodeKind::Directory),
        node(4, Some(1), "target2", NodeKind::Directory),
    ]);
    let batch = collect_projects(&graph);
    let ids: std::collections::HashSet<_> = batch
        .entities
        .iter()
        .map(|entity| &entity.entity_id)
        .collect();
    assert_eq!(ids.len(), batch.entities.len(), "entity ids must be unique");
}

#[test]
fn maven_and_gradle_targets_follow_their_own_manifest() {
    // Maven: pom.xml + target/
    let maven = graph(vec![
        node(1, None, "svc", NodeKind::Directory),
        node(2, Some(1), "pom.xml", NodeKind::File),
        node(3, Some(1), "target", NodeKind::Directory),
    ]);
    let batch = collect_projects(&maven);
    let kinds: Vec<_> = batch.edges.iter().map(|edge| edge.relation).collect();
    assert!(kinds.contains(&Relation::OwnedByProject));
    assert!(kinds.contains(&Relation::RebuildableBy));
    assert!(
        batch
            .entities
            .iter()
            .any(|entity| entity.identity.contains("\"tool\":\"maven\""))
    );

    // Gradle: build.gradle + build/
    let gradle = graph(vec![
        node(1, None, "app", NodeKind::Directory),
        node(2, Some(1), "build.gradle", NodeKind::File),
        node(3, Some(1), "build", NodeKind::Directory),
    ]);
    let batch = collect_projects(&gradle);
    let kinds: Vec<_> = batch.edges.iter().map(|edge| edge.relation).collect();
    assert!(kinds.contains(&Relation::RebuildableBy));
    assert!(
        batch
            .entities
            .iter()
            .any(|entity| entity.identity.contains("\"tool\":\"gradle\""))
    );
}

#[test]
fn a_maven_target_is_not_claimed_by_a_neighbouring_cargo_project() {
    // Two sibling projects: one Rust, one Java. The Java `target` must be
    // attributed to the Java project only.
    let graph = graph(vec![
        node(1, None, "workspace", NodeKind::Directory),
        node(2, Some(1), "rusty", NodeKind::Directory),
        node(3, Some(2), "Cargo.toml", NodeKind::File),
        node(4, Some(2), "target", NodeKind::Directory),
        node(5, Some(1), "java", NodeKind::Directory),
        node(6, Some(5), "pom.xml", NodeKind::File),
        node(7, Some(5), "target", NodeKind::Directory),
    ]);
    let batch = collect_projects(&graph);
    let owner_of = |node_id: u64| {
        batch
            .edges
            .iter()
            .find(|edge| {
                edge.relation == Relation::OwnedByProject
                    && edge.edge_id == format!("edge-own-{node_id}")
            })
            .map(|edge| edge.target_entity_id.clone())
    };
    let rust_target = owner_of(4).expect("rust target owned");
    let java_target = owner_of(7).expect("java target owned");
    assert_ne!(
        rust_target, java_target,
        "each target maps to its own project"
    );
}

#[test]
fn a_target_directory_without_any_manifest_stays_unclaimed_even_with_java_rules() {
    let graph = graph(vec![
        node(1, None, "orphan", NodeKind::Directory),
        node(2, Some(1), "target", NodeKind::Directory),
    ]);
    let batch = collect_projects(&graph);
    assert!(batch.edges.is_empty());
    assert!(batch.evidence.is_empty());
}

#[test]
fn a_directory_named_like_an_output_but_differently_scoped_is_not_claimed() {
    // `target` here belongs to a different (manifest-less) subdirectory;
    // the manifest sits one level up, so the pair is not siblings.
    let graph = graph(vec![
        node(1, None, "proj", NodeKind::Directory),
        node(2, Some(1), "pom.xml", NodeKind::File),
        node(3, Some(1), "module", NodeKind::Directory),
        node(4, Some(3), "target", NodeKind::Directory),
    ]);
    let batch = collect_projects(&graph);
    // Only the manifest is anchored; the nested target is not a sibling.
    assert!(!batch.edges.iter().any(|edge| edge.edge_id == "edge-own-4"));
}
