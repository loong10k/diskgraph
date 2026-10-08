//! 快照页和历史查询适配；来源：原生 Rust MCP 服务，RT-10 机械职责拆分。
use crate::{McpService, snapshot_reply};
use diskgraph_core::{BusinessError, ScopeId};
use diskgraph_engine::EngineError;
use serde_json::{Value, json};

impl McpService {
    /// 按实际范围授权列出有界快照页。
    /// 参数：scope 为范围断言，arguments 为 limit/offset 字段。返回：快照页或错误。
    pub(crate) fn snapshots_tool(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
    ) -> Result<Value, EngineError> {
        let scope_id = self.require_scope(scope)?;
        let limit = arguments.get("limit").and_then(Value::as_u64).unwrap_or(20);
        let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(0);
        let snapshots = self.engine.list_snapshots(
            &scope_id,
            self.context.principal(),
            &self.authorizer()?,
            limit,
            offset,
        )?;
        Ok(json!({
            "snapshots": snapshots
                .iter()
                .map(|snapshot| json!({
                    "snapshot_id": snapshot.id,
                    "complete": snapshot.coverage.complete,
                }))
                .collect::<Vec<_>>(),
        }))
    }

    /// 沿原期限授权两侧历史版本并查询 changes 或 growth。
    /// 参数：catalog_id 为历史查询类型，arguments 为两侧版本，deadline 为原期限。返回：历史结果或错误。
    pub(crate) fn history_tool(
        &self,
        catalog_id: &str,
        arguments: &Value,
        deadline: std::time::Instant,
    ) -> Result<Value, EngineError> {
        let before = arguments.get("before").and_then(Value::as_str);
        let after = arguments.get("after").and_then(Value::as_str);
        let (Some(before), Some(after)) = (before, after) else {
            return Err(EngineError::Business(BusinessError::InvalidArgument));
        };
        let expected = arguments
            .get("scope")
            .and_then(Value::as_str)
            .map(|scope| {
                ScopeId::new(scope.to_owned())
                    .map_err(|_| EngineError::Business(BusinessError::InvalidArgument))
            })
            .transpose()?;
        self.engine.authorize_revision_until(
            expected.as_ref(),
            before,
            self.context.principal(),
            &self.authorizer_until(deadline)?,
            deadline,
        )?;
        self.engine.authorize_revision_until(
            expected.as_ref(),
            after,
            self.context.principal(),
            &self.authorizer_until(deadline)?,
            deadline,
        )?;
        if catalog_id == "C07" {
            let growth = self.engine.growth_between_until(
                before,
                after,
                std::path::Path::new(""),
                snapshot_reply::budget(),
                self.context.principal(),
                &self.authorizer_until(deadline)?,
                deadline,
            )?;
            return Ok(json!({
                "comparable": growth.is_some(),
                "delta_bytes": growth.map(|growth| growth.delta_bytes.to_string()),
            }));
        }
        self.engine.revision_changes_until(
            before,
            after,
            snapshot_reply::budget(),
            self.context.principal(),
            &self.authorizer_until(deadline)?,
            deadline,
        )
    }
}
