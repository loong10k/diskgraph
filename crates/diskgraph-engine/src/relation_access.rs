//! 共享 Engine 的 relation_access 职责；原调用与持锁顺序保持。

use crate::{Engine, EngineError, Explanation, ImpactResult};
use diskgraph_core::{
    Authorizer, PrincipalId, QueryBudget, TruncationReason, measure_json_bounded, query_deadline,
};
use diskgraph_store::CandidateSelection;
use diskgraph_store::StoreError;
use std::time::Instant;

/// 旧关系接口的固定兼容期限，不随新请求的默认 QueryBudget 变化。
const LEGACY_RELATION_DEADLINE_MS: u64 = 1000;

impl Engine {
    /// 可信内部读取 revision 的全部关系。
    /// 参数：revision_id 为标识；调用方须先授权。
    /// 返回：全部关系或存储失败。
    /// Every typed edge of one published revision, for relation-shaped
    /// traversals (impact and friends).
    pub fn all_edges(
        &self,
        revision_id: &str,
    ) -> Result<Vec<diskgraph_core::RelationEdge>, EngineError> {
        let graph = self.graph()?;
        let evidence_reader = graph.revision_evidence(revision_id)?;
        evidence_reader.require_confirmed_membership()?;
        Ok(evidence_reader.all_edges()?)
    }
}

impl Engine {
    /// 按 revision 实际归属授权解释实体及证据。
    /// 参数：revision_id/entity_id 指定对象；principal/authorizer 为请求身份。
    /// 返回：可选实体、关系、证据三元组或授权/读取失败。
    /// Explain one entity of a published revision: the entity, its edges, and
    /// their evidence records (C14, EV-02). Unknown entities stay unknown.
    /// 旧签名保留完整结果结构；默认一秒读取期限及实时终检，不提供新页字节/节点额度。
    /// 新外部请求应使用 `explain_bounded_until`，以传递完整请求预算和原期限。
    #[deprecated(note = "use explain_bounded_until for external budgeted requests")]
    pub fn explain_entity(
        &self,
        revision_id: &str,
        entity_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<Option<Explanation>, EngineError> {
        self.with_authorized_revision_reader(
            revision_id,
            principal,
            authorizer,
            LEGACY_RELATION_DEADLINE_MS,
            |graph, _, _| {
                let evidence_reader = graph.revision_evidence(revision_id)?;
                evidence_reader.require_confirmed_membership()?;
                let Some(entity) = evidence_reader.entity(entity_id)? else {
                    return Ok(None);
                };
                let mut edges = evidence_reader.edges_from(entity_id, None)?;
                edges.extend(evidence_reader.edges_to(entity_id, None)?);
                let edge_ids: Vec<String> = edges.iter().map(|edge| edge.edge_id.clone()).collect();
                let evidence = evidence_reader.evidence_for_edges(&edge_ids)?;
                Ok(Some((entity, edges, evidence)))
            },
        )
    }
}

impl Engine {
    /// 按 revision 实际范围授权读取指定方向关系。
    /// 参数：revision/entity、relation、outgoing 过滤关系；principal/authorizer 为身份。
    /// 返回：关系列表或授权/存储失败；旧列表接口不提供新分页预算。
    /// Typed relations of one entity with direction and optional filter (C13).
    /// 旧签名保留完整结果结构；默认一秒读取期限及实时终检，不提供新页字节/节点额度。
    /// 新外部请求应使用 `related_bounded_until`，以传递完整请求预算和原期限。
    #[deprecated(note = "use related_bounded_until for external budgeted requests")]
    pub fn related(
        &self,
        revision_id: &str,
        entity_id: &str,
        relation: Option<diskgraph_core::Relation>,
        outgoing: bool,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<Vec<diskgraph_core::RelationEdge>, EngineError> {
        self.with_authorized_revision_reader(
            revision_id,
            principal,
            authorizer,
            LEGACY_RELATION_DEADLINE_MS,
            |graph, _, _| {
                let evidence_reader = graph.revision_evidence(revision_id)?;
                evidence_reader.require_confirmed_membership()?;
                if outgoing {
                    Ok(evidence_reader.edges_from(entity_id, relation)?)
                } else {
                    Ok(evidence_reader.edges_to(entity_id, relation)?)
                }
            },
        )
    }
}

impl Engine {
    /// 授权实际 revision 并读取一页关系。
    /// 参数：revision/entity、方向、after_edge_id/limit 为分页；principal/authorizer 为身份。
    /// 返回：关系页和更多标志或失败。
    /// Reads one bounded relation page after resolving the revision's real scope.
    #[allow(clippy::too_many_arguments)] // Mirrors the existing related() API plus a page cursor.
    /// 旧签名保留完整结果结构；默认一秒读取期限及实时终检，不提供新页字节/节点额度。
    /// 新外部请求应使用 `related_bounded_until`，以传递完整请求预算和原期限。
    #[deprecated(note = "use related_bounded_until for external budgeted requests")]
    pub fn related_page(
        &self,
        revision_id: &str,
        entity_id: &str,
        outgoing: bool,
        after_edge_id: Option<&str>,
        limit: u64,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<(Vec<diskgraph_core::RelationEdge>, bool), EngineError> {
        self.with_authorized_revision_reader(
            revision_id,
            principal,
            authorizer,
            LEGACY_RELATION_DEADLINE_MS,
            |graph, _, _| {
                let evidence_reader = graph.revision_evidence(revision_id)?;
                evidence_reader.require_confirmed_membership()?;
                if outgoing {
                    Ok(evidence_reader.edges_from_page(entity_id, after_edge_id, limit)?)
                } else {
                    Ok(evidence_reader.edges_to_page(entity_id, after_edge_id, limit)?)
                }
            },
        )
    }
}

impl Engine {
    /// 授权实际 revision 并通过窄读选择有界候选。
    /// 参数：revision_id、target_bytes、budget 与请求身份为约束。
    /// 返回：候选选择及预算诊断或失败。
    /// 按 revision 的持久归属授权，再通过请求专用只读连接选择有界候选。
    pub fn review_candidates(
        &self,
        revision_id: &str,
        target_bytes: u64,
        budget: QueryBudget,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<CandidateSelection, EngineError> {
        self.review_candidates_until(
            revision_id,
            target_bytes,
            budget,
            principal,
            authorizer,
            query_deadline(budget)?,
        )
    }

    /// 在最外层共享期限上选择候选并完成末段实时授权。
    /// 参数：原查询参数与 deadline；预算从最初准备起计入，typed 期限没有 1000ms 新限制。
    /// 返回：完整候选前缀/缺口或拒权/真实格式错误，最小诊断也放不下则预算失败。
    pub fn review_candidates_until(
        &self,
        revision_id: &str,
        target_bytes: u64,
        budget: QueryBudget,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Instant,
    ) -> Result<CandidateSelection, EngineError> {
        budget.validated()?;
        let answer = self.with_relation_reader_until(
            revision_id,
            principal,
            authorizer,
            deadline,
            budget,
            None,
            |_reader, evidence, reads| match evidence {
                Some(evidence) => {
                    Ok(evidence.candidate_selection_with_budget(target_bytes, budget, reads)?)
                }
                // 目标未观测时不得调用快照读取；保留既有安全诊断与完整缺口。
                None => Ok(CandidateSelection {
                    candidates: Vec::new(),
                    selected_bytes: 0,
                    remaining_bytes: target_bytes,
                    coverage_complete: false,
                    coverage_observed: false,
                    complete: false,
                    truncated: Some(TruncationReason::Deadline),
                }),
            },
            |answer, expired| {
                if expired {
                    answer.complete = false;
                    answer.truncated = Some(TruncationReason::Deadline);
                }
                if measure_json_bounded(answer, budget.max_response_bytes)
                    .map_err(StoreError::from)?
                    .is_none()
                {
                    return Err(diskgraph_core::BusinessError::BudgetExceeded.into());
                }
                Ok(())
            },
        )?;
        Ok(answer)
    }
}

impl Engine {
    /// 在一个已授权 reader 上执行有界影响查询。
    /// 参数：revision/entity、budget 与 principal/authorizer 为上下文。
    /// 返回：影响条目和截断原因或授权/读取失败。
    /// 一次影响查询共享一个授权结果与只读连接，避免每个实体重新打开数据库。
    pub fn revision_impact(
        &self,
        revision_id: &str,
        entity_id: &str,
        budget: QueryBudget,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<ImpactResult, EngineError> {
        self.revision_impact_until(
            revision_id,
            entity_id,
            budget,
            principal,
            authorizer,
            query_deadline(budget)?,
        )
    }

    /// 在整次请求期限与原始字段账本上进行影响遍历。
    /// 参数：原查询参数和 deadline；入/出方向、重复及不传播边按实际解码累计。
    /// 返回：有界条目/截断，末段撤权拒绝全部数据；不授予操作权限。
    pub fn revision_impact_until(
        &self,
        revision_id: &str,
        entity_id: &str,
        budget: QueryBudget,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Instant,
    ) -> Result<ImpactResult, EngineError> {
        budget.validated()?;
        let answer = self.with_relation_reader_until(
            revision_id,
            principal,
            authorizer,
            deadline,
            budget,
            None,
            |_store, reader, reads| {
                let Some(reader) = reader else {
                    return Ok(ImpactResult {
                        entries: Vec::new(),
                        truncated: Some(TruncationReason::Deadline),
                    });
                };
                reader.require_confirmed_membership()?;
                crate::queries::impact_with_budget::<EngineError, _>(
                    entity_id,
                    budget,
                    reads,
                    |current, outgoing, limit, reads| {
                        let (edges, more) = match reader.edges_text_with_budget_page(
                            current,
                            Some(outgoing),
                            None,
                            None,
                            u64::try_from(limit).map_err(|_| StoreError::IntegerOverflow)?,
                            reads,
                        ) {
                            Ok(page) => page,
                            Err(StoreError::BudgetExceeded) => return Ok((Vec::new(), true)),
                            Err(error) if error.is_interrupted() => {
                                reads.stop(TruncationReason::Deadline);
                                return Ok((Vec::new(), true));
                            }
                            Err(error) => return Err(error.into()),
                        };
                        Ok((
                            edges
                                .into_iter()
                                .map(|edge| {
                                    (
                                        if outgoing {
                                            edge.target_entity_id
                                        } else {
                                            edge.source_entity_id
                                        },
                                        edge.relation,
                                    )
                                })
                                .collect(),
                            more,
                        ))
                    },
                )
            },
            |answer, expired| {
                if expired {
                    answer.truncated = Some(TruncationReason::Deadline);
                }
                if measure_json_bounded(answer, budget.max_response_bytes)
                    .map_err(StoreError::from)?
                    .is_none()
                {
                    return Err(diskgraph_core::BusinessError::BudgetExceeded.into());
                }
                Ok(())
            },
        )?;
        Ok(answer)
    }
}
