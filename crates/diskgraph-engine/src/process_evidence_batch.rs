//! 有界逐资源正向观察批次；来源：Rust D42 / EV-06，保留 Partial，永不生成无占用断言。
use crate::EngineError;
use crate::native_process::ProcessNativeSession;
use crate::process_native_error::native_error;
use diskgraph_core::{
    AssertionKind, BusinessError, CollectorBatch, CollectorRun, Entity, EntityKind, EvidenceRecord,
    Polarity, ProcessEvidenceJobInput, ProcessEvidenceSummary, ProcessObservationCoverage,
    Relation, RelationEdge,
};

/// 参数：持久输入/快照/run、真实原生摘要与未重置的会话；返回：安全批次或累计额度错误。
/// 分配前保守收费所有批次字符串/容器及指纹临时编码，最终序列化长度再计原结果额度。
pub(super) fn build(
    input: &ProcessEvidenceJobInput,
    snapshot: &str,
    run: &str,
    summary: &ProcessEvidenceSummary,
    session: &ProcessNativeSession<'_>,
) -> Result<CollectorBatch, EngineError> {
    let cap = usize::try_from(input.limits().max_result_bytes())
        .map_err(|_| BusinessError::BudgetExceeded)?;
    let basis_size = diskgraph_core::measure_json_bounded(summary, cap)
        .map_err(|_| BusinessError::InternalError)?
        .ok_or(BusinessError::BudgetExceeded)?;
    // 固定安全 identity 不含任意客户端文本。每 holder 4KiB 包含三个 ID、边及两份 JSON 临时值；
    // 输入编码至多 16KiB，指纹会重复规范化编码，额外 64KiB 先收费，不将其视为 RSS。
    let allocation = 65_536_usize
        .checked_add(snapshot.len())
        .and_then(|v| v.checked_add(run.len().checked_mul(16)?))
        .and_then(|v| v.checked_add(basis_size.checked_mul(4)?))
        .and_then(|v| v.checked_add(summary.processes().len().checked_mul(4096)?))
        .ok_or(BusinessError::BudgetExceeded)?;
    session
        .admit(0, 0, allocation as u64)
        .map_err(native_error)?;
    let resource = format!("{run}-resource");
    let evidence_id = format!("{run}-summary");
    let observed = summary.capture_window().1;
    let expiry = observed
        .checked_add(30_000)
        .ok_or(BusinessError::BudgetExceeded)?;
    let mut entities = Vec::new();
    entities
        .try_reserve_exact(summary.processes().len() + 1)
        .map_err(|_| BusinessError::ResourceExhausted)?;
    let mut edges = Vec::new();
    edges
        .try_reserve_exact(summary.processes().len())
        .map_err(|_| BusinessError::ResourceExhausted)?;
    entities.push(Entity {
        entity_id: resource.clone(),
        kind: EntityKind::Resource,
        identity: serde_json::json!({"node_id": input.node_id()}).to_string(),
        display: "indexed resource".into(),
        source_run_id: run.to_owned(),
    });
    for (index, startup) in summary.processes().iter().enumerate() {
        session.check().map_err(native_error)?;
        let process = format!("{run}-process-{index}");
        entities.push(Entity {
            entity_id: process.clone(),
            kind: EntityKind::Process,
            identity: serde_json::json!({"server_id":input.server_id(),"startup":startup})
                .to_string(),
            display: "observed process".into(),
            source_run_id: run.to_owned(),
        });
        edges.push(RelationEdge {
            edge_id: format!("{run}-usage-{index}"),
            source_entity_id: resource.clone(),
            relation: Relation::UsedByProcess,
            target_entity_id: process,
            assertion_kind: AssertionKind::Observed,
            evidence_refs: vec![(evidence_id.clone(), Polarity::Supports)],
        });
    }
    let batch = CollectorBatch {
        run: CollectorRun {
            run_id: run.to_owned(),
            snapshot_id: snapshot.to_owned(),
            collector_id: "process-native".into(),
            collector_version: 1,
            rule_version: 1,
            observed_at_unix_ms: observed,
            coverage_complete: summary.coverage()
                == ProcessObservationCoverage::VisibleMethodDomainComplete,
            errors: summary
                .codes()
                .iter()
                .map(|code| code.as_str().to_owned())
                .collect(),
        },
        entities,
        edges,
        evidence: vec![EvidenceRecord {
            evidence_id,
            run_id: run.to_owned(),
            basis: serde_json::to_string(summary).map_err(|_| BusinessError::InternalError)?,
            observed_at_unix_ms: observed,
            expires_at_unix_ms: Some(expiry),
            confidence: 100,
            input_fingerprint: summary.observation_fingerprint(input),
        }],
    };
    let bytes = diskgraph_core::measure_json_bounded(&batch, cap)
        .map_err(|_| BusinessError::InternalError)?
        .ok_or(BusinessError::BudgetExceeded)?;
    session.admit_result(bytes as u64).map_err(native_error)?;
    session.check().map_err(native_error)?;
    Ok(batch)
}
