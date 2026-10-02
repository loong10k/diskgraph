//! 共享 Engine 的 relation_access 职责；原调用与持锁顺序保持。

use crate::{Engine, EngineError, Explanation, ImpactResult, impact_bounded_with_neighbors};
use diskgraph_core::{Authorizer, PrincipalId, QueryBudget};
use diskgraph_store::CandidateSelection;

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
        let revision = graph.revision(revision_id)?;
        Ok(graph.all_edges(&revision.snapshot_id)?)
    }
}

impl Engine {
    /// 按 revision 实际归属授权解释实体及证据。
    /// 参数：revision_id/entity_id 指定对象；principal/authorizer 为请求身份。
    /// 返回：可选实体、关系、证据三元组或授权/读取失败。
    /// Explain one entity of a published revision: the entity, its edges, and
    /// their evidence records (C14, EV-02). Unknown entities stay unknown.
    pub fn explain_entity(
        &self,
        revision_id: &str,
        entity_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<Option<Explanation>, EngineError> {
        self.require_read_for_revision(revision_id, principal, authorizer)?;
        let graph = self.graph()?;
        let revision = graph.revision(revision_id)?;
        let Some(entity) = graph.entity(&revision.snapshot_id, entity_id)? else {
            return Ok(None);
        };
        let mut edges = graph.edges_from(&revision.snapshot_id, entity_id, None)?;
        edges.extend(graph.edges_to(&revision.snapshot_id, entity_id, None)?);
        let edge_ids: Vec<String> = edges.iter().map(|edge| edge.edge_id.clone()).collect();
        let evidence = graph.evidence_for_edges(&revision.snapshot_id, &edge_ids)?;
        Ok(Some((entity, edges, evidence)))
    }
}

impl Engine {
    /// 按 revision 实际范围授权读取指定方向关系。
    /// 参数：revision/entity、relation、outgoing 过滤关系；principal/authorizer 为身份。
    /// 返回：关系列表或授权/存储失败；旧列表接口不提供新分页预算。
    /// Typed relations of one entity with direction and optional filter (C13).
    pub fn related(
        &self,
        revision_id: &str,
        entity_id: &str,
        relation: Option<diskgraph_core::Relation>,
        outgoing: bool,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<Vec<diskgraph_core::RelationEdge>, EngineError> {
        self.require_read_for_revision(revision_id, principal, authorizer)?;
        let graph = self.graph()?;
        let revision = graph.revision(revision_id)?;
        if outgoing {
            Ok(graph.edges_from(&revision.snapshot_id, entity_id, relation)?)
        } else {
            Ok(graph.edges_to(&revision.snapshot_id, entity_id, relation)?)
        }
    }
}

impl Engine {
    /// 授权实际 revision 并读取一页关系。
    /// 参数：revision/entity、方向、after_edge_id/limit 为分页；principal/authorizer 为身份。
    /// 返回：关系页和更多标志或失败。
    /// Reads one bounded relation page after resolving the revision's real scope.
    #[allow(clippy::too_many_arguments)] // Mirrors the existing related() API plus a page cursor.
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
        let graph = self.revision_reader()?;
        self.authorize_revision_with_reader(&graph, None, revision_id, principal, authorizer)?;
        let revision = graph.revision(revision_id)?;
        if outgoing {
            Ok(graph.edges_from_page(&revision.snapshot_id, entity_id, after_edge_id, limit)?)
        } else {
            Ok(graph.edges_to_page(&revision.snapshot_id, entity_id, after_edge_id, limit)?)
        }
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
        let reader = self.revision_reader()?;
        self.authorize_revision_with_reader(&reader, None, revision_id, principal, authorizer)?;
        let snapshot_id = reader.revision(revision_id)?.snapshot_id;
        Ok(reader.candidate_selection(&snapshot_id, target_bytes, budget)?)
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
        let reader = self.revision_reader()?;
        self.authorize_revision_with_reader(&reader, None, revision_id, principal, authorizer)?;
        let snapshot_id = reader.revision(revision_id)?.snapshot_id;
        let mut interrupted = false;
        let mut answer = impact_bounded_with_neighbors::<EngineError, _>(
            entity_id,
            budget,
            |current, outgoing, limit| {
                let page = if outgoing {
                    reader.edges_from_page(&snapshot_id, current, None, limit as u64)
                } else {
                    reader.edges_to_page(&snapshot_id, current, None, limit as u64)
                };
                let (edges, more) = match page {
                    Ok(page) => page,
                    Err(error) if error.is_interrupted() => {
                        interrupted = true;
                        return Ok((Vec::new(), true));
                    }
                    Err(error) => return Err(error.into()),
                };
                Ok((
                    edges
                        .into_iter()
                        .map(|edge| {
                            let neighbour = if outgoing {
                                edge.target_entity_id
                            } else {
                                edge.source_entity_id
                            };
                            (neighbour, edge.relation)
                        })
                        .collect(),
                    more,
                ))
            },
        )?;
        if interrupted {
            answer.truncated = Some(diskgraph_core::TruncationReason::Deadline);
        }
        Ok(answer)
    }
}
