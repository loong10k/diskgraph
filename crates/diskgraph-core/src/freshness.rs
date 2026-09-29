//! Evidence freshness and invalidation (P2 task 3.3, spec EV-03). Freshness is
//! a four-state judgement, never a boolean: an expired process observation
//! means "cannot prove current use", never "unused", and a temporarily
//! unreadable protection policy never un-protects a resource.

use std::collections::HashMap;

use crate::EvidenceRecord;

/// How usable an evidence item is right now.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Freshness {
    /// Observed within its validity and against the same inputs.
    Fresh,
    /// Past its validity window; still historical fact, not current truth.
    Stale,
    /// Its inputs changed, so the claim no longer describes this input.
    Invalidated,
    /// Never established, or the current state could not be observed.
    Unknown,
}

impl Freshness {
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Invalidated => "invalidated",
            Self::Unknown => "unknown",
        }
    }
}

/// The inputs an evidence item was recorded against.
pub trait FingerprintSource {
    /// Current fingerprint of whatever the evidence depended on.
    fn current_fingerprint(&self, evidence_id: &str) -> Option<String>;
}

/// A fingerprint source backed by a plain map (tests and simple adapters).
#[derive(Clone, Debug, Default)]
pub struct FingerprintMap {
    entries: HashMap<String, String>,
}

impl FingerprintMap {
    pub fn set(&mut self, evidence_id: impl Into<String>, fingerprint: impl Into<String>) {
        self.entries.insert(evidence_id.into(), fingerprint.into());
    }
}

impl FingerprintSource for FingerprintMap {
    fn current_fingerprint(&self, evidence_id: &str) -> Option<String> {
        self.entries.get(evidence_id).cloned()
    }
}

/// Classifies one evidence record at `now_unix_ms`.
///
/// A derived claim cannot outlive its inputs, so a caller passes the current
/// fingerprints of every upstream item; missing input data yields `Unknown`
/// rather than an optimistic `Fresh`.
pub fn classify(
    record: &EvidenceRecord,
    now_unix_ms: u64,
    source: &dyn FingerprintSource,
) -> Freshness {
    let Some(current) = source.current_fingerprint(&record.evidence_id) else {
        return Freshness::Unknown;
    };
    if current != record.input_fingerprint {
        return Freshness::Invalidated;
    }
    match record.expires_at_unix_ms {
        Some(expiry) if now_unix_ms >= expiry => Freshness::Stale,
        _ => Freshness::Fresh,
    }
}

/// The freshness of one claim after combining its supporting evidence: any
/// contradiction forces `Unknown`, the weakest support sets the ceiling, and
/// an edge with no evidence is `Unknown` by definition.
pub fn edge_freshness(
    edge: &crate::RelationEdge,
    records: &HashMap<String, &EvidenceRecord>,
    now_unix_ms: u64,
    source: &dyn FingerprintSource,
) -> Freshness {
    if edge.evidence_refs.is_empty() {
        return Freshness::Unknown;
    }
    let mut contradicted = false;
    let mut worst = Freshness::Fresh;
    let mut saw_unknown = false;
    for (evidence_id, polarity) in &edge.evidence_refs {
        if *polarity == crate::Polarity::Contradicts {
            contradicted = true;
            continue;
        }
        match records.get(evidence_id) {
            None => saw_unknown = true,
            Some(record) => {
                let state = classify(record, now_unix_ms, source);
                match state {
                    Freshness::Fresh => {}
                    Freshness::Stale => worst = Freshness::Stale,
                    Freshness::Invalidated => worst = Freshness::Invalidated,
                    Freshness::Unknown => saw_unknown = true,
                }
            }
        }
    }
    if contradicted {
        return Freshness::Unknown;
    }
    if saw_unknown && worst == Freshness::Fresh {
        return Freshness::Unknown;
    }
    worst
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Polarity, Relation, RelationEdge};

    fn record(id: &str, expiry: Option<u64>, fingerprint: &str) -> EvidenceRecord {
        EvidenceRecord {
            evidence_id: id.into(),
            run_id: "run-1".into(),
            basis: "test".into(),
            observed_at_unix_ms: 0,
            expires_at_unix_ms: expiry,
            confidence: 80,
            input_fingerprint: fingerprint.into(),
        }
    }

    fn edge(refs: Vec<(&str, Polarity)>) -> RelationEdge {
        RelationEdge {
            edge_id: "edge-1".into(),
            source_entity_id: "res".into(),
            relation: Relation::UsedByProcess,
            target_entity_id: "proc".into(),
            assertion_kind: crate::AssertionKind::Observed,
            evidence_refs: refs
                .into_iter()
                .map(|(id, polarity)| (id.to_owned(), polarity))
                .collect(),
        }
    }

    fn map(pairs: &[(&str, &str)]) -> FingerprintMap {
        let mut map = FingerprintMap::default();
        for (id, fingerprint) in pairs {
            map.set(*id, *fingerprint);
        }
        map
    }

    #[test]
    fn unchanged_inputs_stay_fresh_until_their_expiry() {
        let item = record("ev-1", Some(1_000), "fp-1");
        let source = map(&[("ev-1", "fp-1")]);
        assert_eq!(classify(&item, 500, &source), Freshness::Fresh);
        assert_eq!(classify(&item, 1_000, &source), Freshness::Stale);
        assert_eq!(classify(&item, 9_999, &source), Freshness::Stale);
    }

    #[test]
    fn changed_inputs_invalidate_even_inside_the_validity_window() {
        let item = record("ev-1", Some(10_000), "fp-1");
        let source = map(&[("ev-1", "fp-2")]);
        assert_eq!(classify(&item, 500, &source), Freshness::Invalidated);
    }

    #[test]
    fn unobservable_current_state_is_unknown_not_fresh() {
        let item = record("ev-1", None, "fp-1");
        // No fingerprint source knows this evidence: the process may have exited.
        assert_eq!(
            classify(&item, 500, &FingerprintMap::default()),
            Freshness::Unknown
        );
    }

    #[test]
    fn contradicting_evidence_keeps_the_claim_unknown() {
        let supporting = record("ev-1", Some(10_000), "fp-1");
        let contradicting = record("ev-2", Some(10_000), "fp-1");
        let mut records = HashMap::new();
        records.insert("ev-1".to_owned(), &supporting);
        records.insert("ev-2".to_owned(), &contradicting);
        let source = map(&[("ev-1", "fp-1"), ("ev-2", "fp-1")]);
        let contested = edge(vec![
            ("ev-1", Polarity::Supports),
            ("ev-2", Polarity::Contradicts),
        ]);
        assert_eq!(
            edge_freshness(&contested, &records, 500, &source),
            Freshness::Unknown
        );
    }

    #[test]
    fn the_weakest_support_sets_the_ceiling_for_a_claim() {
        let fresh = record("ev-1", Some(10_000), "fp-1");
        let expired = record("ev-2", Some(100), "fp-1");
        let mut records = HashMap::new();
        records.insert("ev-1".to_owned(), &fresh);
        records.insert("ev-2".to_owned(), &expired);
        let source = map(&[("ev-1", "fp-1"), ("ev-2", "fp-1")]);
        let claim = edge(vec![
            ("ev-1", Polarity::Supports),
            ("ev-2", Polarity::Supports),
        ]);
        assert_eq!(
            edge_freshness(&claim, &records, 500, &source),
            Freshness::Stale
        );
    }

    #[test]
    fn an_edge_without_evidence_is_unknown_by_definition() {
        let records = HashMap::new();
        let source = FingerprintMap::default();
        assert_eq!(
            edge_freshness(&edge(vec![]), &records, 0, &source),
            Freshness::Unknown
        );
    }
}
