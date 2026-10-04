//! 关系/解释读取共享原始字段账本与绝对期限，结果有限计量后实时复核授权。
use crate::{Engine, EngineError};
use diskgraph_core::{
    Authorizer, BusinessError, PrincipalId, QueryBudget, QueryReadBudget, Relation,
    TruncationReason, measure_json_bounded, query_deadline,
};
use diskgraph_store::{RevisionEvidenceReader, StoreError};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::time::Instant;

impl Engine {
    /// 按真实 revision 授权读取默认预算关系页。
    /// 参数：原 revision/entity/过滤/分页及请求身份。返回：关系前缀、继续位置与截断或失败。
    #[allow(clippy::too_many_arguments)]
    pub fn related_bounded(
        &self,
        revision: &str,
        entity: &str,
        relation: Option<Relation>,
        outgoing: bool,
        after: Option<&str>,
        limit: u64,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<Value, EngineError> {
        let budget = QueryBudget::default();
        self.related_bounded_until(
            revision,
            entity,
            relation,
            outgoing,
            after,
            limit,
            budget,
            principal,
            authorizer,
            query_deadline(budget)?,
        )
    }
    /// 默认预算解释实体；未知对象保持 not_found。
    /// 参数：原 revision/entity/分页及请求身份。返回：实体与完整关系/必需证据前缀，或失败。
    #[allow(clippy::too_many_arguments)]
    pub fn explain_bounded(
        &self,
        revision: &str,
        entity: &str,
        after: Option<&str>,
        limit: u64,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<Value, EngineError> {
        let budget = QueryBudget::default();
        self.explain_bounded_until(
            revision,
            entity,
            after,
            limit,
            budget,
            principal,
            authorizer,
            query_deadline(budget)?,
        )
    }
    /// 关系页采用最外层期限及累计读取额度。
    /// 参数：原参数、budget 和 deadline 为一次请求，实际归属用于授权。
    /// 返回：有界 JSON 前缀与诊断，末段撤权拒绝完整/部分数据。
    #[allow(clippy::too_many_arguments)]
    pub fn related_bounded_until(
        &self,
        revision: &str,
        entity: &str,
        relation: Option<Relation>,
        outgoing: bool,
        after: Option<&str>,
        limit: u64,
        budget: QueryBudget,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Instant,
    ) -> Result<Value, EngineError> {
        self.relation_answer_until(
            revision,
            entity,
            relation,
            Some(outgoing),
            after,
            limit,
            budget,
            principal,
            authorizer,
            deadline,
        )
    }
    /// 解释页共用最外层期限；实体、关系及实际证据读取逐项累计。
    /// 参数：原参数、budget 和 deadline；原默认包装签名不改变。
    /// 返回：有界 JSON、准确继续位置与诊断或真实授权/格式失败。
    #[allow(clippy::too_many_arguments)]
    pub fn explain_bounded_until(
        &self,
        revision: &str,
        entity: &str,
        after: Option<&str>,
        limit: u64,
        budget: QueryBudget,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Instant,
    ) -> Result<Value, EngineError> {
        self.relation_answer_until(
            revision, entity, None, None, after, limit, budget, principal, authorizer, deadline,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn relation_answer_until(
        &self,
        revision: &str,
        entity: &str,
        relation: Option<Relation>,
        direction: Option<bool>,
        after: Option<&str>,
        limit: u64,
        budget: QueryBudget,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Instant,
    ) -> Result<Value, EngineError> {
        budget.validated()?;
        if limit == 0 {
            return Err(BusinessError::InvalidArgument.into());
        }
        let answer = self.with_relation_reader_until(
            revision,
            principal,
            authorizer,
            deadline,
            budget,
            None,
            |_store, reader, reads| {
                let Some(reader) = reader else {
                    let mut answer = json!({"edges":[],"complete":false,"truncated":"deadline","next_after_edge":null});
                    if direction.is_none() {
                        answer["entity"] = Value::Null;
                        answer["evidence"] = json!([]);
                    }
                    return Ok(answer);
                };
                reader.require_confirmed_membership()?;
                relation_data(
                    reader, entity, relation, direction, after, limit, budget, reads,
                )
            },
            |answer, expired| {
                if expired {
                    answer["complete"] = json!(false);
                    answer["truncated"] = json!("deadline");
                }
                if measure_json_bounded(answer, budget.max_response_bytes)
                    .map_err(StoreError::from)?
                    .is_none()
                {
                    return Err(BusinessError::BudgetExceeded.into());
                }
                Ok(())
            },
        )?;
        Ok(answer)
    }
}

#[allow(clippy::too_many_arguments)]
fn relation_data(
    reader: &RevisionEvidenceReader<'_>,
    entity_id: &str,
    relation: Option<Relation>,
    direction: Option<bool>,
    after: Option<&str>,
    limit: u64,
    budget: QueryBudget,
    reads: &mut QueryReadBudget,
) -> Result<Value, EngineError> {
    let deadline = reads.deadline();
    let mut entity = None;
    let mut reason = None;
    if direction.is_none() && reads.check() {
        match reader.entity_with_budget(entity_id, reads) {
            Ok(Some(value)) => entity = Some(value),
            Ok(None) => return Err(BusinessError::NotFound.into()),
            Err(StoreError::BudgetExceeded)
                if reads.stopped() != Some(TruncationReason::Deadline) =>
            {
                return Err(BusinessError::BudgetExceeded.into());
            }
            Err(StoreError::BudgetExceeded) => reason = reads.stopped(),
            Err(error) if error.is_interrupted() => reason = Some(TruncationReason::Deadline),
            Err(error) => return Err(error.into()),
        }
    }
    let cap = budget.max_response_bytes.saturating_sub(2048);
    let mut encoded = match &entity {
        Some(entity) => measure_json_bounded(entity, cap)
            .map_err(StoreError::from)?
            .ok_or(BusinessError::BudgetExceeded)?,
        None => 0,
    };
    let limit =
        limit.min(u64::try_from(budget.max_edges).map_err(|_| StoreError::IntegerOverflow)?);
    let candidates = if reads.check() && reason.is_none() {
        match reader.edges_with_budget_page(entity_id, direction, relation, after, limit, reads) {
            Ok((page, more)) => {
                if more {
                    reason = Some(reads.stopped().unwrap_or(TruncationReason::EdgeLimit));
                }
                page
            }
            Err(StoreError::BudgetExceeded) => {
                reason = Some(reads.stopped().unwrap_or(TruncationReason::ByteLimit));
                Vec::new()
            }
            Err(error) if error.is_interrupted() => {
                reason = Some(TruncationReason::Deadline);
                Vec::new()
            }
            Err(error) => return Err(error.into()),
        }
    } else {
        Vec::new()
    };
    let mut edges = Vec::new();
    let mut evidence = Vec::new();
    let mut seen = HashSet::new();
    let mut cursor_bytes = 0usize;
    for edge in candidates {
        if Instant::now() >= deadline {
            reason = Some(TruncationReason::Deadline);
            break;
        }
        let Some(mut cost) =
            measure_json_bounded(&edge, cap.saturating_sub(encoded)).map_err(StoreError::from)?
        else {
            reason = Some(TruncationReason::ByteLimit);
            break;
        };
        let mut records = Vec::new();
        let mut edge_seen = HashSet::new();
        let mut blocked = false;
        if direction.is_none() {
            for (id, _) in &edge.evidence_refs {
                if seen.contains(id) || !edge_seen.insert(id) {
                    continue;
                }
                match reader.evidence_record_with_budget(id, reads) {
                    Ok(Some(record)) => {
                        let Some(bytes) = measure_json_bounded(
                            &record,
                            cap.saturating_sub(encoded).saturating_sub(cost),
                        )
                        .map_err(StoreError::from)?
                        else {
                            reason = Some(TruncationReason::ByteLimit);
                            blocked = true;
                            break;
                        };
                        cost = cost
                            .checked_add(bytes)
                            .and_then(|n| n.checked_add(1))
                            .ok_or(BusinessError::BudgetExceeded)?;
                        records.push(record);
                    }
                    Ok(None) => {
                        return Err(
                            StoreError::InvalidGraph(format!("missing evidence {id}")).into()
                        );
                    }
                    Err(StoreError::BudgetExceeded) => {
                        reason = Some(reads.stopped().unwrap_or(TruncationReason::ByteLimit));
                        blocked = true;
                        break;
                    }
                    Err(error) if error.is_interrupted() => {
                        reason = Some(TruncationReason::Deadline);
                        blocked = true;
                        break;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
        if blocked {
            break;
        }
        let Some(next_cursor) = measure_json_bounded(&edge.edge_id, cap.saturating_sub(encoded))
            .map_err(StoreError::from)?
        else {
            reason = Some(TruncationReason::ByteLimit);
            break;
        };
        let Some(total) = encoded
            .checked_sub(cursor_bytes)
            .and_then(|n| n.checked_add(cost))
            .and_then(|n| n.checked_add(next_cursor))
            .and_then(|n| n.checked_add(1))
            .filter(|n| *n <= cap)
        else {
            reason = Some(TruncationReason::ByteLimit);
            break;
        };
        if Instant::now() >= deadline {
            reason = Some(TruncationReason::Deadline);
            break;
        }
        encoded = total;
        cursor_bytes = next_cursor;
        for record in records {
            seen.insert(record.evidence_id.clone());
            evidence.push(record);
        }
        edges.push(edge);
    }
    if Instant::now() >= deadline {
        reason = Some(TruncationReason::Deadline);
    }
    let next = edges
        .last()
        .filter(|_| reason.is_some())
        .map(|edge| edge.edge_id.clone());
    let mut answer = json!({"edges":edges,"complete":reason.is_none(),"truncated":reason.map(TruncationReason::wire_name),"next_after_edge":next});
    if direction.is_none() {
        answer["entity"] = json!(entity);
        answer["evidence"] = json!(evidence);
    }
    if measure_json_bounded(&answer, budget.max_response_bytes)
        .map_err(StoreError::from)?
        .is_none()
    {
        return Err(BusinessError::BudgetExceeded.into());
    }
    Ok(answer)
}
