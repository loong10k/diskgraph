//! 范围管理和旧扫描状态适配；来源：原生 Rust MCP 服务，RT-10 机械职责拆分。
use crate::{McpService, protocol};
use diskgraph_core::BusinessError;
use diskgraph_engine::EngineError;
use serde_json::{Value, json};

impl McpService {
    /// 处理服务端范围管理的已支持动作。
    /// 参数：arguments 为已通过 schema 的动作字段。返回：范围列表或明确 unsupported 错误。
    pub(crate) fn scope_tool(
        &self,
        arguments: &Value,
        deadline: std::time::Instant,
    ) -> Result<Value, EngineError> {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("list");
        match action {
            "list" => {
                let scopes = self.engine.list_scopes_until(
                    self.context.principal(),
                    &self.authorizer_until(deadline)?,
                    deadline,
                )?;
                Ok(json!({
                    "scopes": scopes
                        .iter()
                        .map(|scope| json!({
                            "scope_id": scope.scope_id.as_str(),
                            "root_display": scope.root.display(),
                            "revoked": scope.revoked,
                        }))
                        .collect::<Vec<_>>(),
                }))
            }
            // Scope add/remove arrive with the CLI's C28/C01 wiring in P3's
            // install step; MCP exposes listing today.
            _ => Err(EngineError::Business(BusinessError::Unsupported)),
        }
    }

    /// 读取服务状态或按实际任务范围授权的原扫描状态。
    /// 参数：arguments 为可选 job_id 字段。返回：原状态 wire 形状或错误。
    pub(crate) fn status_tool(
        &self,
        arguments: &Value,
        deadline: std::time::Instant,
    ) -> Result<Value, EngineError> {
        let Some(job_id) = arguments.get("job_id").and_then(Value::as_str) else {
            // With no job ID the status is service-level.
            return Ok(json!({
                "server_id": self.engine.server_id_until(deadline)?.as_str(),
                "profile": self.profile.wire_name(),
                "tools": protocol::tools_for(self.profile).len(),
                // Capability diagnostics: the legacy adapter ships in this
                // build but serves only when explicitly enabled (P4-5.7).
                "legacy_sse": self.legacy_sse,
            }));
        };
        let record = self.engine.job_status_authorized_until(
            job_id,
            self.context.principal(),
            &self.authorizer_until(deadline)?,
            deadline,
        )?;
        let result = json!({
            "job_id": record.job_id,
            "scope_id": record.scope_id.as_str(),
            "state": format!("{:?}", record.state).to_ascii_lowercase(),
        });
        Ok(result)
    }
}
