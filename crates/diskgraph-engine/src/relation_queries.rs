//! 关系和解释查询共用独立只读连接，限制实际解码、证据和响应成本。

use crate::{Engine, EngineError};
use diskgraph_core::{Authorizer, BusinessError, PrincipalId, QueryBudget, Relation};
use diskgraph_store::{SqliteSnapshotStore, StoreError};
use serde_json::{Value, json};
use std::collections::HashSet;

impl Engine {
    /// 按真实 revision 授权读取关系页；字节、边数及期限均适用，返回继续位置。
    #[allow(clippy::too_many_arguments)] // 保持 related 接口并增加分页位置和页大小。
    /// 参数：revision/entity 指定对象，relation/outgoing 指定过滤，after/limit 指定页；principal/authorizer 绑定请求。
    /// 返回：关系页 JSON 与继续/截断诊断，或授权、预算、存储错误。
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
        self.relation_answer(
            revision,
            entity,
            relation,
            Some(outgoing),
            after,
            limit,
            principal,
            authorizer,
        )
    }

    /// 有界解释一个实体，同时读取双向关系和相应证据；未知实体返回 not_found。
    #[allow(clippy::too_many_arguments)] // 解释上下文和继续位置不可互相替代。
    /// 参数：revision/entity 指定对象，after/limit 指定页，principal/authorizer 为请求身份。
    /// 返回：实体、关系及证据 JSON；未知对象、预算或授权失败返回错误。
    pub fn explain_bounded(
        &self,
        revision: &str,
        entity: &str,
        after: Option<&str>,
        limit: u64,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<Value, EngineError> {
        self.relation_answer(
            revision, entity, None, None, after, limit, principal, authorizer,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn relation_answer(
        &self,
        revision: &str,
        entity_id: &str,
        relation: Option<Relation>,
        direction: Option<bool>,
        after: Option<&str>,
        limit: u64,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<Value, EngineError> {
        let budget = QueryBudget::default();
        if limit == 0 {
            return Err(EngineError::Business(BusinessError::InvalidArgument));
        }
        let started = std::time::Instant::now();
        let reader = SqliteSnapshotStore::open_reader(&self.graph_path, budget.deadline_ms, None)?;
        self.authorize_revision_with_reader(&reader, None, revision, principal, authorizer)?;
        let snapshot = reader.revision(revision)?.snapshot_id;
        let entity = if direction.is_none() {
            Some(
                reader
                    .entity_bounded(
                        &snapshot,
                        entity_id,
                        budget.max_response_bytes.saturating_sub(2048),
                    )
                    .map_err(|error| match error {
                        StoreError::BudgetExceeded => {
                            EngineError::Business(BusinessError::BudgetExceeded)
                        }
                        other => other.into(),
                    })?
                    .ok_or(EngineError::Business(BusinessError::NotFound))?,
            )
        } else {
            None
        };
        let mut remaining = budget.max_response_bytes.saturating_sub(2048);
        if let Some(entity) = &entity {
            let cost = serde_json::to_vec(entity).map_err(StoreError::from)?.len();
            if cost > remaining {
                return Err(EngineError::Business(BusinessError::BudgetExceeded));
            }
            remaining -= cost;
        }
        let limit = limit.min(budget.max_edges as u64);
        let mut truncated = None;
        let candidates = match reader.edges_filtered_page(
            &snapshot, entity_id, direction, relation, after, limit, remaining,
        ) {
            Ok((page, more)) => {
                if more {
                    truncated = Some("edge_or_byte_limit");
                }
                page
            }
            Err(error) if error.is_interrupted() => {
                truncated = Some("deadline");
                Vec::new()
            }
            Err(StoreError::BudgetExceeded) => {
                return Err(EngineError::Business(BusinessError::BudgetExceeded));
            }
            Err(error) => return Err(error.into()),
        };
        let mut edges = Vec::new();
        let mut evidence = Vec::new();
        let mut seen = HashSet::new();
        let mut cursor_bytes = 0usize;
        for edge in candidates {
            if edges.len() as u64 >= limit {
                truncated = Some("edge_limit");
                break;
            }
            if started.elapsed().as_millis() >= budget.deadline_ms as u128 {
                truncated = Some("deadline");
                break;
            }
            let mut cost = serde_json::to_vec(&edge).map_err(StoreError::from)?.len();
            let mut records = Vec::new();
            let mut blocked = false;
            if direction.is_none() {
                let mut edge_seen = HashSet::new();
                for (id, _) in &edge.evidence_refs {
                    if seen.contains(id) || !edge_seen.insert(id) {
                        continue;
                    }
                    match reader.evidence_record_bounded(
                        &snapshot,
                        id,
                        remaining.saturating_sub(cost),
                    ) {
                        Ok(Some(record)) => {
                            cost = cost.saturating_add(
                                serde_json::to_vec(&record).map_err(StoreError::from)?.len(),
                            );
                            records.push(record);
                        }
                        Ok(None) => {
                            return Err(
                                StoreError::InvalidGraph(format!("missing evidence {id}")).into()
                            );
                        }
                        Err(StoreError::BudgetExceeded) => {
                            truncated = Some("response_byte_limit");
                            blocked = true;
                            break;
                        }
                        Err(error) if error.is_interrupted() => {
                            truncated = Some("deadline");
                            blocked = true;
                            break;
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
            }
            let next_cursor_bytes = serde_json::to_vec(&edge.edge_id)
                .map_err(StoreError::from)?
                .len();
            if cost.saturating_add(next_cursor_bytes) > remaining.saturating_add(cursor_bytes)
                || blocked
            {
                if truncated.is_none() {
                    truncated = Some("response_byte_limit");
                }
                if edges.is_empty() && truncated != Some("deadline") {
                    return Err(EngineError::Business(BusinessError::BudgetExceeded));
                }
                break;
            }
            remaining = remaining
                .saturating_add(cursor_bytes)
                .saturating_sub(cost)
                .saturating_sub(next_cursor_bytes);
            cursor_bytes = next_cursor_bytes;
            for record in records {
                seen.insert(record.evidence_id.clone());
                evidence.push(record);
            }
            edges.push(edge);
        }
        let next = edges
            .last()
            .filter(|_| truncated.is_some())
            .map(|edge| edge.edge_id.clone());
        let mut answer = json!({"edges":edges,"complete":truncated.is_none(),"truncated":truncated,"next_after_edge":next});
        if let Some(entity) = entity {
            answer["entity"] = json!(entity);
            answer["evidence"] = json!(evidence);
        }
        Ok(answer)
    }
}
