//! 实体关系、解释、影响与复核候选适配；来源：原生 Rust MCP 服务，RT-10 机械职责拆分。
use crate::McpService;
use diskgraph_core::{BusinessError, QueryBudget, Relation, ScopeId};
use diskgraph_engine::EngineError;
use serde_json::{Value, json};

impl McpService {
    /// 沿共享期限查询指定实体的单向关系页。
    /// 参数：arguments 为实体、关系及游标字段，deadline 为原期限。返回：有界关系页或错误。
    #[cfg(test)]
    pub(crate) fn related_tool(
        &self,
        arguments: &Value,
        deadline: std::time::Instant,
    ) -> Result<Value, EngineError> {
        self.related_tool_with_finish(arguments, deadline, |_, _| Ok(()))
    }

    /// 在原关系请求内准备实际响应，保持编码期间的撤权见证。
    /// 参数：原业务参数与期限；finish 可重复准备，但不得输出。返回：终检成功的结果或错误。
    pub(crate) fn related_tool_with_finish(
        &self,
        arguments: &Value,
        deadline: std::time::Instant,
        mut finish: impl FnMut(&Value, bool) -> Result<(), EngineError>,
    ) -> Result<Value, EngineError> {
        let (revision, entity) = self.revision_and_entity(arguments)?;
        let relation = arguments
            .get("relation")
            .and_then(Value::as_str)
            .map(|name| {
                Relation::parse(name).ok_or(EngineError::Business(BusinessError::InvalidArgument))
            })
            .transpose()?;
        let outgoing = arguments
            .get("direction")
            .and_then(Value::as_str)
            .map(|direction| direction == "outgoing")
            .unwrap_or(true);
        self.engine.related_bounded_with_finish_until(
            &revision,
            &entity,
            relation,
            outgoing,
            arguments.get("after_edge").and_then(Value::as_str),
            arguments
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(QueryBudget::default().max_edges as u64),
            QueryBudget::default(),
            self.context.principal(),
            &self.authorizer_until(deadline)?,
            deadline,
            |data, expired| finish(data, expired),
        )
    }

    /// 沿共享期限读取实体证据及双向关系解释。
    /// 参数：arguments 为实体和游标字段，deadline 为原期限。返回：有界解释结果或错误。
    #[cfg(test)]
    pub(crate) fn explain_tool(
        &self,
        arguments: &Value,
        deadline: std::time::Instant,
    ) -> Result<Value, EngineError> {
        self.explain_tool_with_finish(arguments, deadline, |_, _| Ok(()))
    }

    /// 在原关系请求内准备实际响应，保持编码期间的撤权见证。
    /// 参数：原业务参数与期限；finish 可重复准备，但不得输出。返回：终检成功的结果或错误。
    pub(crate) fn explain_tool_with_finish(
        &self,
        arguments: &Value,
        deadline: std::time::Instant,
        mut finish: impl FnMut(&Value, bool) -> Result<(), EngineError>,
    ) -> Result<Value, EngineError> {
        let (revision, entity) = self.revision_and_entity(arguments)?;
        self.engine.explain_bounded_with_finish_until(
            &revision,
            &entity,
            arguments.get("after_edge").and_then(Value::as_str),
            arguments
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(QueryBudget::default().max_edges as u64),
            QueryBudget::default(),
            self.context.principal(),
            &self.authorizer_until(deadline)?,
            deadline,
            |data, expired| finish(data, expired),
        )
    }

    /// 读取实体关系影响，仅返回只读评估，不授予执行权限。
    /// 参数：arguments 为版本及实体，deadline 为原期限。返回：影响条目与完整性或错误。
    #[cfg(test)]
    pub(crate) fn impact_tool(
        &self,
        arguments: &Value,
        deadline: std::time::Instant,
    ) -> Result<Value, EngineError> {
        self.impact_tool_with_finish(arguments, deadline, |_, _| Ok(()))
    }

    /// 在原关系请求内准备实际响应，保持编码期间的撤权见证。
    /// 参数：原业务参数与期限；finish 可重复准备，但不得输出。返回：终检成功的结果或错误。
    pub(crate) fn impact_tool_with_finish(
        &self,
        arguments: &Value,
        deadline: std::time::Instant,
        mut finish: impl FnMut(&Value, bool) -> Result<(), EngineError>,
    ) -> Result<Value, EngineError> {
        let (revision, entity) = self.revision_and_entity(arguments)?;
        let authorizer = self.authorizer_until(deadline)?;
        let mut prepared = None;
        self.engine.revision_impact_with_finish_until(
            &revision,
            &entity,
            QueryBudget::default(),
            self.context.principal(),
            &authorizer,
            deadline,
            |answer, expired| {
                let data = json!({
                    "entries": answer.entries
                        .iter()
                        .map(|entry| json!({
                            "entity_id": entry.entity_id,
                            "relation": entry.relation.wire_name(),
                            "depth": entry.depth,
                        }))
                        .collect::<Vec<_>>(),
                    "complete": answer.truncated.is_none(),
                    "truncated": answer.truncated.map(|reason| reason.wire_name()),
                    // A read-only impact answer never authorizes a mutation.
                    "grants_execution": false,
                });
                finish(&data, expired)?;
                prepared = Some(data);
                Ok(())
            },
        )?;
        prepared.ok_or_else(|| BusinessError::BudgetExceeded.into())
    }

    /// 按实际版本及共享期限执行仅供复核的候选选择。
    /// 参数：scope 为范围断言，arguments 为目标字节字段，deadline 为原期限。返回：候选、覆盖率、缺口与截断信息或错误。
    #[cfg(test)]
    pub(crate) fn candidates_tool(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
        deadline: std::time::Instant,
    ) -> Result<Value, EngineError> {
        self.candidates_tool_with_finish(scope, arguments, deadline, |_, _| Ok(()))
    }

    /// 在原关系请求内准备实际响应，保持编码期间的撤权见证。
    /// 参数：原业务参数与期限；finish 可重复准备，但不得输出。返回：终检成功的结果或错误。
    pub(crate) fn candidates_tool_with_finish(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
        deadline: std::time::Instant,
        mut finish: impl FnMut(&Value, bool) -> Result<(), EngineError>,
    ) -> Result<Value, EngineError> {
        let revision = self.require_revision_until(scope, arguments, deadline)?;
        let target = arguments
            .get("target_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let authorizer = self.authorizer_until(deadline)?;
        let mut prepared = None;
        self.engine.review_candidates_with_finish_until(
            &revision,
            target,
            QueryBudget::default(),
            self.context.principal(),
            &authorizer,
            deadline,
            |answer, expired| {
                let data = json!({
                    "candidates": answer.candidates
                        .iter()
                        .map(|(node, evidence)| json!({"node": node, "evidence": evidence}))
                        .collect::<Vec<_>>(),
                    "review_only": true,
                    "coverage_complete": answer.coverage_complete,
                    "coverage_observed": answer.coverage_observed,
                    "complete": answer.complete,
                    "truncated": answer.truncated.map(|reason| reason.wire_name()),
                    "selected_bytes": answer.selected_bytes.to_string(),
                    "remaining_bytes": answer.remaining_bytes.to_string(),
                });
                finish(&data, expired)?;
                prepared = Some(data);
                Ok(())
            },
        )?;
        prepared.ok_or_else(|| BusinessError::BudgetExceeded.into())
    }

    /// 提取必须显式提供的关系查询版本和实体。
    /// 参数：arguments 为已校验业务字段。返回：版本及实体字符串或参数错误。
    fn revision_and_entity(&self, arguments: &Value) -> Result<(String, String), EngineError> {
        let revision = arguments
            .get("revision")
            .and_then(Value::as_str)
            .ok_or(EngineError::Business(BusinessError::InvalidArgument))?
            .to_owned();
        let entity = arguments
            .get("entity")
            .and_then(Value::as_str)
            .ok_or(EngineError::Business(BusinessError::InvalidArgument))?
            .to_owned();
        Ok((revision, entity))
    }
}
