//! 按原规则收集构建项目证据。

use super::ProjectBatch;
use super::ecosystem::{Ecosystem, ecosystem_for_manifest};
use diskgraph_core::{
    AssertionKind, CollectorRun, DiskGraph, Entity, EntityKind, EvidenceRecord, Polarity, Relation,
    RelationEdge,
};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

/// Rule set version; bumping it invalidates previously collected evidence.
pub const RULE_VERSION: u32 = 2;
pub const COLLECTOR_ID: &str = "build-project-markers";
pub const COLLECTOR_VERSION: u32 = 2;
/// 扫描快照内同目录清单/产物布局并生成去重证据批次。
/// 参数：graph 为已观察的快照。
/// 返回：确定性 ProjectBatch，仅提出归属/重建证据，不授权删除。
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
