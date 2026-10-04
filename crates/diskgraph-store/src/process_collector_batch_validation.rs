//! 逐资源正向占用批次和覆盖的严格安全合同；来源：原生 Rust D42 / EV-06。
use crate::{Result, StoreError};
use diskgraph_core::{
    AssertionKind, CollectorBatch, EntityKind, Polarity, ProcessEvidenceJobInput,
    ProcessEvidenceSummary, ProcessObservationCoverage, Relation,
};
/// 参数：实际固定输入与可信 Engine 批次；返回：逐资源完整启动身份匹配，否则整批拒绝。
/// 即使 complete empty 也不发布 Negative/无占用断言，更不删除旧正向证据。
pub(crate) fn validate(input: &ProcessEvidenceJobInput, batch: &CollectorBatch) -> Result<()> {
    let invalid = || StoreError::InvalidGraph("invalid process collector batch".into());
    if batch.run.collector_id != "process-native"
        || batch.run.collector_version != 1
        || batch.run.rule_version != 1
        || batch.evidence.len() != 1
        || !crate::git_collector_batch_validation::valid_key(&batch.run.run_id)
    {
        return Err(invalid());
    }
    let cap = usize::try_from(input.limits().max_result_bytes())
        .map_err(|_| StoreError::BudgetExceeded)?;
    if diskgraph_core::measure_json_bounded(batch, cap)?.is_none() {
        return Err(StoreError::BudgetExceeded);
    }
    let evidence = &batch.evidence[0];
    let summary: ProcessEvidenceSummary = serde_json::from_str(&evidence.basis)?;
    if summary.method() != input.method()
        || summary.processes().len() as u64 > input.limits().max_entries()
        || batch.entities.len() != summary.processes().len() + 1
        || batch.edges.len() != summary.processes().len()
    {
        return Err(invalid());
    }
    let complete = summary.coverage() == ProcessObservationCoverage::VisibleMethodDomainComplete;
    let codes: Vec<_> = summary.codes().iter().map(|c| c.as_str()).collect();
    if batch.run.coverage_complete != complete
        || batch
            .run
            .errors
            .iter()
            .map(String::as_str)
            .ne(codes.iter().copied())
    {
        return Err(invalid());
    }
    let resource = batch
        .entities
        .iter()
        .find(|e| e.kind == EntityKind::Resource)
        .ok_or_else(invalid)?;
    if resource.display != "indexed resource"
        || serde_json::from_str::<serde_json::Value>(&resource.identity)?
            != serde_json::json!({"node_id":input.node_id()})
    {
        return Err(invalid());
    }
    let expiry = evidence.expires_at_unix_ms.ok_or_else(invalid)?;
    if evidence.run_id != batch.run.run_id
        || !crate::git_collector_batch_validation::valid_key(&evidence.evidence_id)
        || evidence.confidence > 100
        || evidence.input_fingerprint != summary.observation_fingerprint(input)
        || evidence.observed_at_unix_ms != summary.capture_window().1
        || batch.run.observed_at_unix_ms != summary.capture_window().1
        || expiry <= evidence.observed_at_unix_ms
        || expiry - evidence.observed_at_unix_ms > 300000
    {
        return Err(invalid());
    }
    let mut entities = std::collections::HashSet::new();
    let mut processes = std::collections::HashMap::new();
    for entity in &batch.entities {
        if entity.source_run_id != batch.run.run_id
            || !crate::git_collector_batch_validation::valid_key(&entity.entity_id)
            || !entities.insert(&entity.entity_id)
        {
            return Err(invalid());
        }
        if entity.entity_id == resource.entity_id {
            continue;
        }
        if entity.kind != EntityKind::Process || entity.display != "observed process" {
            return Err(invalid());
        }
        let identity: crate::process_entity_identity::ProcessEntityIdentity =
            serde_json::from_str(&entity.identity)?;
        if identity.server_id != *input.server_id()
            || processes.insert(identity.startup, entity).is_some()
        {
            return Err(invalid());
        }
    }
    let mut edges = std::collections::HashMap::new();
    let mut keys = std::collections::HashSet::new();
    for edge in &batch.edges {
        if !crate::git_collector_batch_validation::valid_key(&edge.edge_id)
            || !keys.insert(&edge.edge_id)
            || edge.source_entity_id != resource.entity_id
            || edge.relation != Relation::UsedByProcess
            || edge.assertion_kind != AssertionKind::Observed
            || edge.evidence_refs != [(evidence.evidence_id.clone(), Polarity::Supports)]
            || edges.insert(&edge.target_entity_id, edge).is_some()
        {
            return Err(invalid());
        }
    }
    for startup in summary.processes() {
        let process = processes.get(startup).ok_or_else(invalid)?;
        if !edges.contains_key(&process.entity_id) {
            return Err(invalid());
        }
    }
    Ok(())
}
