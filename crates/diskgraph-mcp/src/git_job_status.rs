//! Git C04 的实际任务响应边界；来源：原生 Rust CMD-04 / EC-02，共用 Engine 授权投影。
use crate::McpService;
use diskgraph_core::{
    BusinessError, Envelope, RevisionId, ScopeId, ServerId, measure_json_bounded,
};
use diskgraph_engine::EngineError;
use serde_json::Value;
use std::time::Instant;

impl McpService {
    /// 参数：已完成工具/profile/schema 校验的业务字段与原请求期限。
    /// 返回：Git 专用 envelope，或 None 继续原扫描/服务状态；不以默认 scope 或 latest 补齐身份。
    pub(super) fn git_status_envelope(
        &self,
        arguments: &Value,
        deadline: Instant,
    ) -> Result<Option<Value>, EngineError> {
        let Some(job_id) = arguments.get("job_id").and_then(Value::as_str) else {
            return Ok(None);
        };
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let Some(data) = self.engine.git_job_status_details(
            job_id,
            self.context.principal(),
            &self.authorizer()?,
        )?
        else {
            return Ok(None);
        };
        // 字段均来自同一有界、初末已授权投影；终检后不再查询 server、默认 scope 或 latest。
        let scope =
            ScopeId::new(field(&data, "scope_id")?).map_err(|_| BusinessError::InternalError)?;
        if arguments
            .get("scope")
            .and_then(Value::as_str)
            .is_some_and(|hint| hint != scope.as_str())
        {
            return Err(BusinessError::PermissionDenied.into());
        }
        let server =
            ServerId::new(field(&data, "server_id")?).map_err(|_| BusinessError::InternalError)?;
        let revision = data
            .get("revision_id")
            .map(|value| {
                let id = value.as_str().ok_or(BusinessError::InternalError)?;
                RevisionId::new(id).map_err(|_| BusinessError::InternalError)
            })
            .transpose()?;
        let envelope = Envelope::ok(data)
            .with_ids(Some(server), Some(scope), revision)
            .into_json();
        if measure_json_bounded(&envelope, 16 * 1024)
            .map_err(|_| BusinessError::InternalError)?
            .is_none()
            || Instant::now() >= deadline
        {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(Some(envelope))
    }
}

fn field<'a>(data: &'a Value, name: &str) -> Result<&'a str, BusinessError> {
    data.get(name)
        .and_then(Value::as_str)
        .ok_or(BusinessError::InternalError)
}
