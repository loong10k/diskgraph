//! Git 方法局部摘要到严格 collector batch 的转换；来源：原生 Rust EV-02 / EC-02。
//! fingerprint 绑定方法和安全结果观察及持久请求，不声称是源正文或整个工作树内容摘要；过期后保留 unknown。

use crate::EngineError;
use crate::live_evidence::GitSample;
use diskgraph_core::{
    AssertionKind, BusinessError, CollectorBatch, CollectorRun, Entity, EntityKind, EvidenceRecord,
    GitEvidenceJobInput, GitEvidenceSummary, Polarity, Relation, RelationEdge,
};

/// 参数：input 是原持久请求，snapshot/run 为发布身份，sample 为完成源终验与私有清理的真实样本。
/// 返回：仅包含安全计数和固定 coverage 的完整 batch；原 HEAD、路径及工具诊断永不持久化。
pub(super) fn build(
    input: &GitEvidenceJobInput,
    snapshot: &str,
    run: &str,
    sample: &GitSample,
) -> Result<CollectorBatch, EngineError> {
    let summary = GitEvidenceSummary::new(
        sample.dirty_count,
        sample.stash_count,
        sample.ahead_of_upstream,
        sample.behind_upstream,
        sample.sampled_at_unix_ms,
    )
    .map_err(|_| BusinessError::Unsupported)?;
    let resource = format!("{run}-resource");
    let project = format!("{run}-project");
    let evidence_id = format!("{run}-summary");
    let expiry = sample
        .sampled_at_unix_ms
        .checked_add(30_000)
        .ok_or(BusinessError::Unsupported)?;
    let basis = serde_json::to_string(&summary).map_err(|_| BusinessError::InternalError)?;
    Ok(CollectorBatch {
        run: CollectorRun {
            run_id: run.to_owned(),
            snapshot_id: snapshot.to_owned(),
            collector_id: "git-local".into(),
            collector_version: 1,
            rule_version: 1,
            observed_at_unix_ms: sample.sampled_at_unix_ms,
            coverage_complete: true,
            errors: Vec::new(),
        },
        entities: vec![
            Entity {
                entity_id: resource.clone(),
                kind: EntityKind::Resource,
                identity: serde_json::json!({"node_id":input.node_id()}).to_string(),
                display: format!("Indexed directory {}", input.node_id()),
                source_run_id: run.to_owned(),
            },
            Entity {
                entity_id: project.clone(),
                kind: EntityKind::Project,
                identity: serde_json::json!({"node_id":input.node_id(),"collector":"git-local"})
                    .to_string(),
                display: format!("Local Git project {}", input.node_id()),
                source_run_id: run.to_owned(),
            },
        ],
        evidence: vec![EvidenceRecord {
            evidence_id: evidence_id.clone(),
            run_id: run.to_owned(),
            basis,
            observed_at_unix_ms: sample.sampled_at_unix_ms,
            expires_at_unix_ms: Some(expiry),
            confidence: 100,
            input_fingerprint: summary.observation_fingerprint(input),
        }],
        edges: vec![RelationEdge {
            edge_id: format!("{run}-ownership"),
            source_entity_id: resource,
            relation: Relation::OwnedByProject,
            target_entity_id: project,
            assertion_kind: AssertionKind::Observed,
            evidence_refs: vec![(evidence_id, Polarity::Supports)],
        }],
    })
}
