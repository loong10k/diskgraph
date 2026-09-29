//! Typed entities, relations, and evidence (P2 tasks 3.1–3.3, specs EV-01 to
//! EV-03). A relation is a claim with provenance — never a permission; a
//! missing or stale observation stays unknown instead of becoming "unused".

use serde::{Deserialize, Serialize};

/// Entity kinds; each relation type constrains its endpoint kinds (EV-01).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Resource,
    Application,
    Project,
    Process,
    BuildRecipe,
    ProtectionPolicy,
}

impl EntityKind {
    /// Stable wire name used in storage and envelopes.
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Resource => "resource",
            Self::Application => "application",
            Self::Project => "project",
            Self::Process => "process",
            Self::BuildRecipe => "build_recipe",
            Self::ProtectionPolicy => "protection_policy",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "resource" => Self::Resource,
            "application" => Self::Application,
            "project" => Self::Project,
            "process" => Self::Process,
            "build_recipe" => Self::BuildRecipe,
            "protection_policy" => Self::ProtectionPolicy,
            _ => return None,
        })
    }
}

/// Typed relations with fixed direction: source → target (EV-01).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// Resource → Resource; only from same-snapshot parent enumeration.
    Contains,
    /// Resource → Project; a manifest declares the project root.
    Declares,
    /// Resource → Project; supports nested projects and multiple owners.
    OwnedByProject,
    /// Resource → Application; precise metadata is separate from heuristics.
    OwnedByApplication,
    /// Resource → Process; observation coverage only, never a full dependency claim.
    UsedByProcess,
    /// Resource → BuildRecipe; carries rule version and tool preconditions.
    RebuildableBy,
    /// Resource → ProtectionPolicy; authoritative sources only.
    ProtectedBy,
    /// Resource → Resource; content-verified conclusion, not deletion license.
    SameContentAs,
}

impl Relation {
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Contains => "contains",
            Self::Declares => "declares",
            Self::OwnedByProject => "owned_by_project",
            Self::OwnedByApplication => "owned_by_application",
            Self::UsedByProcess => "used_by_process",
            Self::RebuildableBy => "rebuildable_by",
            Self::ProtectedBy => "protected_by",
            Self::SameContentAs => "same_content_as",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "contains" => Self::Contains,
            "declares" => Self::Declares,
            "owned_by_project" => Self::OwnedByProject,
            "owned_by_application" => Self::OwnedByApplication,
            "used_by_process" => Self::UsedByProcess,
            "rebuildable_by" => Self::RebuildableBy,
            "protected_by" => Self::ProtectedBy,
            "same_content_as" => Self::SameContentAs,
            _ => return None,
        })
    }

    /// The legal endpoint kinds (source, target) for this relation.
    pub fn endpoints(self) -> (&'static [EntityKind], &'static [EntityKind]) {
        const RESOURCE: &[EntityKind] = &[EntityKind::Resource];
        match self {
            Self::Contains => (RESOURCE, RESOURCE),
            Self::Declares => (RESOURCE, &[EntityKind::Project]),
            Self::OwnedByProject => (RESOURCE, &[EntityKind::Project]),
            Self::OwnedByApplication => (RESOURCE, &[EntityKind::Application]),
            Self::UsedByProcess => (RESOURCE, &[EntityKind::Process]),
            Self::RebuildableBy => (RESOURCE, &[EntityKind::BuildRecipe]),
            Self::ProtectedBy => (RESOURCE, &[EntityKind::ProtectionPolicy]),
            Self::SameContentAs => (RESOURCE, RESOURCE),
        }
    }

    /// Whether a source/target pair is legal for this relation.
    pub fn admits(self, source: EntityKind, target: EntityKind) -> bool {
        let (sources, targets) = self.endpoints();
        sources.contains(&source) && targets.contains(&target)
    }
}

/// How an edge came to exist; heuristic edges can never upgrade themselves.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssertionKind {
    Observed,
    Derived,
    Heuristic,
    UserPolicy,
}

/// Evidence polarity on one edge: supports or contradicts the claim.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    Supports,
    Contradicts,
}

/// Provenance for one collected batch (EV-02): who, against what, how covered.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CollectorRun {
    pub run_id: String,
    pub snapshot_id: String,
    pub collector_id: String,
    pub collector_version: u32,
    pub rule_version: u32,
    pub observed_at_unix_ms: u64,
    pub coverage_complete: bool,
    pub errors: Vec<String>,
}

/// One evidence item backing or contradicting edges; freshness is judged from
/// its own observation time plus the input fingerprint (EV-03).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub evidence_id: String,
    pub run_id: String,
    pub basis: String,
    pub observed_at_unix_ms: u64,
    /// Absence means "valid until invalidated", not "forever".
    pub expires_at_unix_ms: Option<u64>,
    /// Method-local confidence grade, 0-100; never a probability, never permission.
    pub confidence: u8,
    /// Fingerprint of what was observed; a change invalidates dependent edges.
    pub input_fingerprint: String,
}

/// One typed claim with its supporting and contradicting evidence (EV-02).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RelationEdge {
    pub edge_id: String,
    pub source_entity_id: String,
    pub relation: Relation,
    pub target_entity_id: String,
    pub assertion_kind: AssertionKind,
    /// (evidence_id, polarity) pairs; conflicts coexist and stay explainable.
    pub evidence_refs: Vec<(String, Polarity)>,
}

/// An entity observed within one snapshot scope (EV-01).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub entity_id: String,
    pub kind: EntityKind,
    /// Structured identity payload (JSON); shape depends on the kind.
    pub identity: String,
    pub display: String,
    pub source_run_id: String,
}

/// Result of endpoint validation before any edge is stored (EV-01).
#[derive(Debug)]
pub struct EdgeValidationError {
    pub edge_id: String,
    pub relation: Relation,
    pub reason: &'static str,
}

/// Validates every edge's endpoints against its relation contract and the
/// known entity kinds; unknown entities are rejected outright.
pub fn validate_edges(
    edges: &[RelationEdge],
    entities: &[(String, EntityKind)],
) -> Result<(), EdgeValidationError> {
    let kinds: std::collections::HashMap<&str, EntityKind> = entities
        .iter()
        .map(|(id, kind)| (id.as_str(), *kind))
        .collect();
    for edge in edges {
        let Some(&source) = kinds.get(edge.source_entity_id.as_str()) else {
            return Err(EdgeValidationError {
                edge_id: edge.edge_id.clone(),
                relation: edge.relation,
                reason: "unknown source entity",
            });
        };
        let Some(&target) = kinds.get(edge.target_entity_id.as_str()) else {
            return Err(EdgeValidationError {
                edge_id: edge.edge_id.clone(),
                relation: edge.relation,
                reason: "unknown target entity",
            });
        };
        if !edge.relation.admits(source, target) {
            return Err(EdgeValidationError {
                edge_id: edge.edge_id.clone(),
                relation: edge.relation,
                reason: "endpoint kinds violate the relation contract",
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(id: &str, source: &str, relation: Relation, target: &str) -> RelationEdge {
        RelationEdge {
            edge_id: id.into(),
            source_entity_id: source.into(),
            relation,
            target_entity_id: target.into(),
            assertion_kind: AssertionKind::Observed,
            evidence_refs: vec![],
        }
    }

    #[test]
    fn relation_endpoints_reject_kind_mismatches() {
        let entities = vec![
            ("res-1".to_owned(), EntityKind::Resource),
            ("proj-1".to_owned(), EntityKind::Project),
            ("proc-1".to_owned(), EntityKind::Process),
        ];
        assert!(
            validate_edges(
                &[edge("e1", "res-1", Relation::OwnedByProject, "proj-1")],
                &entities,
            )
            .is_ok()
        );
        // A process is not a project: the owned_by_project edge is refused.
        assert!(matches!(
            validate_edges(
                &[edge("e2", "res-1", Relation::OwnedByProject, "proc-1")],
                &entities
            ),
            Err(EdgeValidationError {
                reason: "endpoint kinds violate the relation contract",
                ..
            })
        ));
        // Edges to entities that do not exist in this batch are refused.
        assert!(matches!(
            validate_edges(
                &[edge("e3", "res-1", Relation::Contains, "ghost")],
                &entities
            ),
            Err(EdgeValidationError {
                reason: "unknown target entity",
                ..
            })
        ));
    }

    #[test]
    fn wire_names_round_trip_through_parse() {
        for relation in [
            Relation::Contains,
            Relation::Declares,
            Relation::OwnedByProject,
            Relation::OwnedByApplication,
            Relation::UsedByProcess,
            Relation::RebuildableBy,
            Relation::ProtectedBy,
            Relation::SameContentAs,
        ] {
            assert_eq!(Relation::parse(relation.wire_name()), Some(relation));
        }
        for kind in [
            EntityKind::Resource,
            EntityKind::Application,
            EntityKind::Project,
            EntityKind::Process,
            EntityKind::BuildRecipe,
            EntityKind::ProtectionPolicy,
        ] {
            assert_eq!(EntityKind::parse(kind.wire_name()), Some(kind));
        }
        assert_eq!(Relation::parse("not-a-relation"), None);
    }

    #[test]
    fn evidence_records_keep_supporting_and_contradicting_refs_apart() {
        let edge = RelationEdge {
            edge_id: "e1".into(),
            source_entity_id: "res-1".into(),
            relation: Relation::OwnedByProject,
            target_entity_id: "proj-1".into(),
            assertion_kind: AssertionKind::Heuristic,
            evidence_refs: vec![
                ("ev-1".into(), Polarity::Supports),
                ("ev-2".into(), Polarity::Contradicts),
            ],
        };
        assert_eq!(edge.evidence_refs.len(), 2);
        assert_eq!(edge.evidence_refs[0].1, Polarity::Supports);
        assert_eq!(edge.evidence_refs[1].1, Polarity::Contradicts);
    }
}
