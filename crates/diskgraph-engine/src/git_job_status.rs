//! Git 持久状态的共同授权投影；来源：原生 Rust C03 / EC-02，CLI/MCP 共用。
use crate::{Engine, EngineError};
use diskgraph_core::{
    Authorizer, BusinessError, Permission, PrincipalId, QueryBudget, QueryReadBudget, ScopeId,
    measure_json_bounded,
};
use diskgraph_store::{ControlStore, JobKind, JobState, SqliteSnapshotStore, StoreError};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

impl Engine {
    /// 读取 Git 任务固定状态；旧扫描任务返回 None，由适配器保持原响应。
    /// 参数：job_id 是持久任务，principal/authorizer 是本次实际请求身份。
    /// 返回：有界安全状态或拒绝；初末均检查实际 scope，完成结果另需 MetadataRead。
    /// 同步授权为协作式检查，不能宣称硬实时；原 1 秒期限不因末检重新开始。
    pub fn git_job_status_details(
        &self,
        job_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
    ) -> Result<Option<Value>, EngineError> {
        self.git_job_status_details_until(
            job_id,
            principal,
            authorizer,
            Instant::now() + Duration::from_secs(1),
        )
    }

    /// 读取状态并继承整次请求期限，分类探测和授权末检不续期。
    /// 来源：DiskGraph 原生 Rust 持久任务投影。
    /// 参数：job_id 为实际任务；principal/authorizer 为本次身份；deadline 为原始期限。
    /// 返回：有界授权投影、非本类型的 None，或期限/权限错误。
    pub fn git_job_status_details_until(
        &self,
        job_id: &str,
        principal: &PrincipalId,
        authorizer: &dyn Authorizer,
        deadline: Instant,
    ) -> Result<Option<Value>, EngineError> {
        if job_id.len() > 128 {
            return Err(BusinessError::InvalidArgument.into());
        }
        let control = self.control_until(deadline)?;
        let (job, failure, server) = control.with_read_deadline(deadline, |control| {
            let job = control.job(job_id)?;
            if job.kind != JobKind::GitEvidence {
                return Ok::<_, EngineError>((job, None, None));
            }
            status_permissions(control, authorizer, principal, &job.scope_id, job.state)?;
            let failure = control.git_job_failure(job_id)?;
            Ok((job, failure, Some(control.existing_server_id()?)))
        })?;
        drop(control);
        if job.kind != JobKind::GitEvidence {
            return Ok(None);
        }
        let server = server.ok_or(BusinessError::InternalError)?;
        let mut result = json!({"job_id":job.job_id,"server_id":server.as_str(),"scope_id":job.scope_id.as_str(),"state":job.state});
        if job.state == JobState::Completed {
            let reader = SqliteSnapshotStore::open_reader_until(&self.graph_path, deadline, None)?;
            let receipt = reader
                .job_publication_receipt(job_id)?
                .ok_or(BusinessError::Conflict)?;
            if receipt.server_id() != &server || receipt.scope_id() != &job.scope_id {
                return Err(BusinessError::PermissionDenied.into());
            }
            let mut reads = QueryReadBudget::new(
                QueryBudget {
                    max_response_bytes: 16 * 1024,
                    ..QueryBudget::default()
                },
                deadline,
            )?;
            // 回执保留已提交历史；存在的结果还须与原 snapshot 和真实归属完全一致。
            let available =
                match reader.revision_target_with_budget(receipt.revision_id(), &mut reads) {
                    Ok((snapshot, Some((server, scope))))
                        if snapshot == receipt.snapshot_id()
                            && server == receipt.server_id().as_str()
                            && scope == receipt.scope_id().as_str() =>
                    {
                        true
                    }
                    Ok(_) => return Err(BusinessError::PermissionDenied.into()),
                    Err(StoreError::RevisionNotFound(_)) => false,
                    Err(error) => return Err(error.into()),
                };
            result["result_available"] = json!(available);
            result["revision"] = json!(receipt.revision_id());
            result["revision_id"] = json!(receipt.revision_id());
            result["run_id"] = json!(receipt.run_id());
        }
        if let Some(failure) = failure {
            result["failure"] =
                serde_json::to_value(failure).map_err(|_| BusinessError::InternalError)?;
        }
        if measure_json_bounded(&result, 16 * 1024)
            .map_err(|_| BusinessError::InternalError)?
            .is_none()
        {
            return Err(BusinessError::BudgetExceeded.into());
        }
        let control = self.control_until(deadline)?;
        control.with_read_deadline(deadline, |control| {
            status_permissions(control, authorizer, principal, &job.scope_id, job.state)
        })?;
        if Instant::now() >= deadline {
            return Err(BusinessError::BudgetExceeded.into());
        }
        Ok(Some(result))
    }
}

/// 参数：原控制库、身份与实际任务 scope/state；返回：能力上限与当前授权交集。
/// 所有能力回调后再纯读全部 grant，后一个回调不能使前一个权限复检失效。
fn status_permissions(
    control: &ControlStore,
    authorizer: &dyn Authorizer,
    principal: &PrincipalId,
    scope: &ScopeId,
    state: JobState,
) -> Result<(), EngineError> {
    let permissions: &[Permission] = if state == JobState::Completed {
        &[Permission::OperationView, Permission::MetadataRead]
    } else {
        &[Permission::OperationView]
    };
    if control.scope(scope)?.revoked {
        return Err(BusinessError::PermissionDenied.into());
    }
    for permission in permissions {
        Engine::require_with_control(control, authorizer, principal, permission, scope)?;
    }
    for permission in permissions {
        if control.live_permission(principal, permission, scope)? == Some(false) {
            return Err(BusinessError::PermissionDenied.into());
        }
    }
    Ok(())
}
