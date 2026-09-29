//! Deterministic project collectors (P2 task 3.4, spec EV-04): Cargo, Node,
//! and Java manifest markers produce project ownership claims only when the
//! build output sits next to its manifest in the same directory. A name match
//! alone never produces evidence, and nothing here grants deletion.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

use diskgraph_core::{
    AssertionKind, CollectorRun, DiskGraph, Entity, EntityKind, EvidenceRecord, Polarity, Relation,
    RelationEdge,
};

/// Rule set version; bumping it invalidates previously collected evidence.
pub const RULE_VERSION: u32 = 2;

pub const COLLECTOR_ID: &str = "build-project-markers";
pub const COLLECTOR_VERSION: u32 = 2;

/// The collected batch for one snapshot.
pub struct ProjectBatch {
    pub run: CollectorRun,
    pub entities: Vec<Entity>,
    pub evidence: Vec<EvidenceRecord>,
    pub edges: Vec<RelationEdge>,
}

/// One ecosystem rule: which manifest declares the project, which sibling
/// directory is build output, which tool rebuilds it, and whether the layout
/// alone proves the output is rebuildable.
struct Ecosystem {
    manifest: &'static str,
    output: &'static str,
    tool: &'static str,
    /// Whether the deterministic layout justifies a `rebuildable_by` edge.
    /// Node's `node_modules` depends on a lockfile this collector does not
    /// observe, so it is owned but not claimed rebuildable.
    proves_rebuildable: bool,
}

const ECOSYSTEMS: &[Ecosystem] = &[
    Ecosystem {
        manifest: "Cargo.toml",
        output: "target",
        tool: "cargo",
        proves_rebuildable: true,
    },
    Ecosystem {
        manifest: "package.json",
        output: "node_modules",
        tool: "npm",
        proves_rebuildable: false,
    },
    Ecosystem {
        // Maven and Gradle both keep build output in `target`; whichever
        // manifest sits beside it defines the project.
        manifest: "pom.xml",
        output: "target",
        tool: "maven",
        proves_rebuildable: true,
    },
    Ecosystem {
        manifest: "build.gradle",
        output: "build",
        tool: "gradle",
        proves_rebuildable: true,
    },
    Ecosystem {
        manifest: "build.gradle.kts",
        output: "build",
        tool: "gradle",
        proves_rebuildable: true,
    },
];

/// The ecosystem owning a manifest file name, if any.
fn ecosystem_for_manifest(manifest: &str) -> Option<&'static Ecosystem> {
    ECOSYSTEMS.iter().find(|rule| rule.manifest == manifest)
}

/// Walks the snapshot and emits deterministic project claims. Resource
/// entities are synthesized for every node an edge references, and repeated
/// claims for the same project are deduplicated so one batch inserts cleanly.
pub fn collect_projects(graph: &DiskGraph) -> ProjectBatch {
    // Directories that contain a manifest: the deterministic project anchor.
    // A Maven multi-module layout can declare several manifests in one
    // directory tree; each directory keeps its own ecosystem.
    let mut manifest_dirs: HashMap<u64, &'static Ecosystem> = HashMap::new();
    let mut manifest_nodes: HashMap<u64, u64> = HashMap::new();
    for node in &graph.nodes {
        if node.kind != diskgraph_core::NodeKind::File {
            continue;
        }
        if let Some(rule) = ecosystem_for_manifest(&node.name)
            && let Some(parent) = node.parent_id
        {
            manifest_dirs.entry(parent).or_insert(rule);
            manifest_nodes.entry(parent).or_insert(node.id);
        }
    }

    let run_id = format!("run-{}", graph.snapshot.id);
    let mut projects: HashMap<u64, String> = HashMap::new();
    let mut entity_map: HashMap<String, Entity> = HashMap::new();
    let mut evidence = Vec::new();
    let mut edges = Vec::new();

    // One project entity per manifest file.
    for node in &graph.nodes {
        let Some(parent) = node.parent_id else {
            continue;
        };
        let Some(rule) = manifest_dirs.get(&parent) else {
            continue;
        };
        if node.name == rule.manifest {
            let project_id = format!("project-{}", node.id);
            projects.insert(node.id, project_id.clone());
            entity_map.insert(
                project_id.clone(),
                Entity {
                    entity_id: project_id,
                    kind: EntityKind::Project,
                    identity: format!(
                        "{{\"manifest\":\"{}\",\"tool\":\"{}\",\"node_id\":{}}}",
                        rule.manifest, rule.tool, node.id
                    ),
                    display: format!("project at {}", node.name),
                    source_run_id: run_id.clone(),
                },
            );
        }
    }

    let mut resource_entities: HashMap<u64, String> = HashMap::new();

    for node in &graph.nodes {
        let Some(parent) = node.parent_id else {
            continue;
        };
        let Some(rule) = manifest_dirs.get(&parent) else {
            continue;
        };
        if node.name == rule.manifest {
            continue; // handled above
        }
        // The output directory must be the rule's own output name: a shared
        // or differently named `target` is never claimed by association.
        if node.name != rule.output || node.kind != diskgraph_core::NodeKind::Directory {
            continue;
        }
        let Some(manifest_node_id) = manifest_nodes.get(&parent).copied() else {
            continue;
        };
        let Some(project_id) = projects.get(&manifest_node_id).cloned() else {
            continue;
        };

        let tool = rule.tool;
        let recipe_id = format!("recipe-{tool}-{}", node.id);
        let observed_at = now_ms();
        let evidence_id = format!("ev-output-{tool}-{}", node.id);

        entity_map.insert(
            recipe_id.clone(),
            Entity {
                entity_id: recipe_id.clone(),
                kind: EntityKind::BuildRecipe,
                identity: format!("{{\"tool\":\"{tool}\",\"rule_version\":{RULE_VERSION}}}"),
                display: format!("{tool} build output"),
                source_run_id: run_id.clone(),
            },
        );
        evidence.push(EvidenceRecord {
            evidence_id: evidence_id.clone(),
            run_id: run_id.clone(),
            basis: format!(
                "{} sits in the same directory as {} (deterministic layout, rule v{RULE_VERSION})",
                node.name, rule.manifest
            ),
            observed_at_unix_ms: observed_at,
            expires_at_unix_ms: None,
            confidence: 90,
            input_fingerprint: fingerprint(&[node.id, manifest_node_id]),
        });

        let source_id = resource_entity(node.id, &run_id, &mut entity_map, &mut resource_entities);
        edges.push(RelationEdge {
            edge_id: format!("edge-own-{}", node.id),
            source_entity_id: source_id.clone(),
            relation: Relation::OwnedByProject,
            target_entity_id: project_id,
            assertion_kind: AssertionKind::Observed,
            evidence_refs: vec![(evidence_id.clone(), Polarity::Supports)],
        });
        // Rebuildability follows the rule: Node's `node_modules` depends on
        // lockfile state this collector does not observe, so it stays owned
        // but unclaimed.
        if rule.proves_rebuildable {
            edges.push(RelationEdge {
                edge_id: format!("edge-rebuild-{}", node.id),
                source_entity_id: source_id.clone(),
                relation: Relation::RebuildableBy,
                target_entity_id: recipe_id,
                assertion_kind: AssertionKind::Derived,
                evidence_refs: vec![(evidence_id, Polarity::Supports)],
            });
        }
    }

    let run = CollectorRun {
        run_id: run_id.clone(),
        snapshot_id: graph.snapshot.id.clone(),
        collector_id: COLLECTOR_ID.to_owned(),
        collector_version: COLLECTOR_VERSION,
        rule_version: RULE_VERSION,
        observed_at_unix_ms: now_ms(),
        coverage_complete: graph.snapshot.coverage.complete,
        errors: Vec::new(),
    };
    let mut entities: Vec<Entity> = entity_map.into_values().collect();
    entities.sort_by(|a, b| a.entity_id.cmp(&b.entity_id));
    ProjectBatch {
        run,
        entities,
        evidence,
        edges,
    }
}

/// Reuses (or lazily creates) the resource entity for one snapshot node.
fn resource_entity(
    node_id: u64,
    run_id: &str,
    entity_map: &mut HashMap<String, Entity>,
    known: &mut HashMap<u64, String>,
) -> String {
    if let Some(entity_id) = known.get(&node_id) {
        return entity_id.clone();
    }
    let entity_id = format!("resource-{node_id}");
    entity_map.insert(
        entity_id.clone(),
        Entity {
            entity_id: entity_id.clone(),
            kind: EntityKind::Resource,
            identity: format!("{{\"node_id\":{node_id}}}"),
            display: format!("node {node_id}"),
            source_run_id: run_id.to_owned(),
        },
    );
    known.insert(node_id, entity_id.clone());
    entity_id
}

fn fingerprint(parts: &[u64]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_millis()
        .try_into()
        .expect("timestamp beyond u64")
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
