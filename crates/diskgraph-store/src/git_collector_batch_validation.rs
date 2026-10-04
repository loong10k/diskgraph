//! 产品 Git 批次仅持久安全本地摘要；来源：原生 Rust EC-02 / EV-02。
use crate::{Result, StoreError};
use diskgraph_core::{
    AssertionKind, CollectorBatch, EntityKind, GitEvidenceJobInput, GitEvidenceSummary, Polarity,
    Relation,
};

/// 参数：固定原输入和由 Engine 构造的批次；返回：唯一安全关系和摘要或拒绝，不接收客户端批次。
pub(crate) fn validate(input: &GitEvidenceJobInput, batch: &CollectorBatch) -> Result<()> {
    let invalid = || StoreError::InvalidGraph("invalid Git collector batch".into());
    if batch.run.collector_id != "git-local"
        || batch.run.collector_version != 1
        || batch.run.rule_version != 1
        || !batch.run.coverage_complete
        || !batch.run.errors.is_empty()
        || batch.entities.len() != 2
        || batch.evidence.len() != 1
        || batch.edges.len() != 1
        || !valid_key(&batch.run.run_id)
    {
        return Err(invalid());
    }
    if diskgraph_core::measure_json_bounded(batch, 16384)?.is_none() {
        return Err(StoreError::BudgetExceeded);
    }
    let resource = batch
        .entities
        .iter()
        .find(|e| e.kind == EntityKind::Resource)
        .ok_or_else(invalid)?;
    let project = batch
        .entities
        .iter()
        .find(|e| e.kind == EntityKind::Project)
        .ok_or_else(invalid)?;
    let expected_resource = serde_json::json!({"node_id":input.node_id()});
    let expected_project = serde_json::json!({"node_id":input.node_id(),"collector":"git-local"});
    for entity in &batch.entities {
        if entity.source_run_id != batch.run.run_id
            || !valid_key(&entity.entity_id)
            || entity.display.len() > 256
        {
            return Err(invalid());
        }
    }
    if resource.entity_id == project.entity_id
        || serde_json::from_str::<serde_json::Value>(&resource.identity)? != expected_resource
        || serde_json::from_str::<serde_json::Value>(&project.identity)? != expected_project
    {
        return Err(invalid());
    }
    let evidence = &batch.evidence[0];
    let summary: GitEvidenceSummary = serde_json::from_str(&evidence.basis)?;
    let Some(expiry) = evidence.expires_at_unix_ms else {
        return Err(invalid());
    };
    if evidence.run_id != batch.run.run_id
        || !valid_key(&evidence.evidence_id)
        || evidence.confidence > 100
        || evidence.input_fingerprint != summary.observation_fingerprint(input)
        || summary.sampled_at_unix_ms() != evidence.observed_at_unix_ms
        || evidence.observed_at_unix_ms != batch.run.observed_at_unix_ms
        || expiry <= evidence.observed_at_unix_ms
        || expiry - evidence.observed_at_unix_ms > 300_000
    {
        return Err(invalid());
    }
    let edge = &batch.edges[0];
    if !valid_key(&edge.edge_id)
        || edge.source_entity_id != resource.entity_id
        || edge.target_entity_id != project.entity_id
        || edge.relation != Relation::OwnedByProject
        || edge.assertion_kind != AssertionKind::Observed
        || edge.evidence_refs != [(evidence.evidence_id.clone(), Polarity::Supports)]
    {
        return Err(invalid());
    }
    Ok(())
}

/// 参数：服务端生成的有限标识；返回：适合不可变记录的 ASCII 键。
pub(crate) fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
