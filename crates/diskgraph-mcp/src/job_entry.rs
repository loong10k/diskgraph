//! C02/C03 请求身份与持久任务接线；来源：原生 Rust MCP 适配器，不接收客户端 authority。
use crate::McpService;
use diskgraph_core::{BusinessError, ScopeId};
use diskgraph_engine::EngineError;
use serde_json::{Value, json};

impl McpService {
    /// 参数：scope 为请求范围断言，arguments 为已通过 schema 的业务字段，sync 选择 C03。
    /// 返回：真实持久 job handle；证据字段存在时固定绑定 revision/node，不退化成扫描。
    pub(super) fn index_tool(
        &self,
        scope: &Option<ScopeId>,
        arguments: &Value,
        sync: bool,
    ) -> Result<Value, EngineError> {
        let scope_id = self.require_scope(scope)?;
        let authority = self.context.job_authority()?;
        let authorizer = self.authorizer()?;
        let job = if sync && arguments.get("collector").is_some() {
            let revision = arguments
                .get("revision")
                .and_then(Value::as_str)
                .ok_or(BusinessError::InvalidArgument)?;
            let node = arguments
                .get("node_id")
                .and_then(Value::as_u64)
                .ok_or(BusinessError::InvalidArgument)?;
            match arguments.get("collector").and_then(Value::as_str) {
                Some("git") => self.engine.git_evidence_scope_with_authority(
                    &scope_id,
                    revision,
                    node,
                    &authority,
                    &authorizer,
                )?,
                Some("process") => self.engine.process_evidence_scope_with_authority(
                    &scope_id,
                    revision,
                    node,
                    &authority,
                    &authorizer,
                )?,
                _ => return Err(BusinessError::InvalidArgument.into()),
            }
        } else if sync {
            self.engine
                .sync_scope_with_authority(&scope_id, &authority, &authorizer)?
        } else {
            self.engine
                .index_scope_with_authority(&scope_id, &authority, &authorizer)?
        };
        Ok(json!({"job_id":job.job_id,"state":job.state,"poll_with":"diskgraph_status"}))
    }
}
