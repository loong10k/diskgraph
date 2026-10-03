//! 有界正文读取。

use super::inspection_clock::now_ms;
use super::{InspectionRequest, InspectionStop, PlaceholderProbe, ReadOutcome};
use crate::{Engine, EngineError};
use diskgraph_core::BusinessError;
use std::io::Read;

impl Engine {
    /// 在内容授权与原生句柄门禁后读取有限范围。
    /// 参数：request 为精确范围/预算/取消，probe 仅诊断，authorizer 为请求能力。
    /// 返回：读取字节及稳定/截断诊断，或授权/平台/I/O 失败。
    /// Reads at most `max_bytes` from `offset` of one authorized object.
    /// Every gate runs before the first byte moves: authorization, scope
    /// containment, plain-file check, placeholder policy (CT-01, CT-02).
    pub fn read_bounded(
        &self,
        request: &InspectionRequest<'_>,
        probe: &dyn PlaceholderProbe,
        authorizer: &dyn diskgraph_core::Authorizer,
    ) -> Result<ReadOutcome, EngineError> {
        let deadline = std::time::Instant::now()
            .checked_add(std::time::Duration::from_secs(30))
            .ok_or(EngineError::Business(BusinessError::InvalidArgument))?;
        self.read_bounded_until(request, probe, authorizer, deadline)
    }

    /// 在同一绝对期限内读取正文并执行末段授权、取消及版本复检。
    /// 参数：request/probe/authorizer 沿用旧范围与能力，deadline 在首次准备前建立。
    /// 返回：实际读取字节与中止原因；撤权拒绝返回正文，同步 I/O 与锁等待为协作期限。
    /// 来源：DiskGraph CT-01/02 与 D24 内容终态契约。
    pub fn read_bounded_until(
        &self,
        request: &InspectionRequest<'_>,
        probe: &dyn PlaceholderProbe,
        authorizer: &dyn diskgraph_core::Authorizer,
        deadline: std::time::Instant,
    ) -> Result<ReadOutcome, EngineError> {
        let _hydration = crate::scoped_content::ScopedContent::hydration_guard()?;
        self.require(
            authorizer,
            request.principal,
            &diskgraph_core::Permission::ContentRead,
            request.scope_id,
        )?;
        let record = self.scope(request.scope_id)?;
        if record.revoked {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        let root = record
            .root
            .to_native_path()
            .map_err(|_| EngineError::Business(BusinessError::Unsupported))?;
        if std::time::Instant::now() >= deadline {
            return Err(EngineError::Business(BusinessError::Timeout));
        }
        let mut prepared = crate::scoped_content::ScopedContent::open(&root, request.path, probe)?;
        if prepared.file.is_none() {
            self.require_read_terminal(request, authorizer)?;
            return Ok(ReadOutcome {
                requested_path: request.path.to_path_buf(),
                offset: request.offset,
                bytes: Vec::new(),
                file_len: prepared.metadata.len(),
                truncated: false,
                stopped: Some(if std::time::Instant::now() >= deadline {
                    InspectionStop::Deadline
                } else if request
                    .cancel
                    .is_some_and(|cancel| cancel.load(std::sync::atomic::Ordering::SeqCst))
                {
                    InspectionStop::Cancelled
                } else {
                    InspectionStop::Placeholder
                }),
                observed_at_unix_ms: now_ms(),
            });
        }
        // 数据句柄覆盖后续版本核验；Windows 的 prepared 还持有父目录与属性租约。
        // Unix 沿用文件句柄及路径身份检查，不在本增量承诺持续祖先目录租约。
        let mut file = prepared
            .file
            .take()
            .ok_or(EngineError::Business(BusinessError::Unsupported))?;
        use std::io::Seek;
        file.seek(std::io::SeekFrom::Start(request.offset))?;
        let chunk = request.chunk_bytes.clamp(1, 64 * 1024).min(
            usize::try_from(request.max_bytes)
                .unwrap_or(usize::MAX)
                .max(1),
        );
        let mut bytes = Vec::new();
        let mut buffer = vec![0_u8; chunk];
        let mut truncated = false;
        let mut stopped = None;
        loop {
            self.require(
                authorizer,
                request.principal,
                &diskgraph_core::Permission::ContentRead,
                request.scope_id,
            )?;
            if self.control_store()?.live_permission(
                request.principal,
                &diskgraph_core::Permission::ContentRead,
                request.scope_id,
            )? == Some(false)
            {
                return Err(EngineError::Business(BusinessError::PermissionDenied));
            }
            // 授权或控制连接可以等待；每个数据块前重新检查整次期限与取消。
            if std::time::Instant::now() >= deadline {
                stopped = Some(InspectionStop::Deadline);
                break;
            }
            if let Some(cancel) = request.cancel
                && cancel.load(std::sync::atomic::Ordering::SeqCst)
            {
                stopped = Some(InspectionStop::Cancelled);
                break;
            }
            if bytes.len() as u64 >= request.max_bytes {
                truncated = true;
                break;
            }
            let want = usize::try_from(request.max_bytes - bytes.len() as u64)
                .unwrap_or(usize::MAX)
                .min(chunk);
            let buffer = &mut buffer[..want];
            let read = file.read(buffer)?;
            if read == 0 {
                break;
            }
            bytes
                .try_reserve_exact(read)
                .map_err(|_| EngineError::Business(BusinessError::ResourceExhausted))?;
            bytes.extend_from_slice(&buffer[..read]);
        }
        // The file must still be the object the read started on.
        if !prepared.matches(&file) {
            stopped = Some(InspectionStop::Unstable);
        }
        // 仅测试的末段同步：真实读取与身份检查已经结束，不持控制库 guard。
        #[cfg(test)]
        super::read_terminal_tests::before_reply();
        // EOF、精确范围和原生版本检查都可能耗时；任何中止也不能跳过撤权检查。
        self.require_read_terminal(request, authorizer)?;
        if stopped.is_none() && std::time::Instant::now() >= deadline {
            stopped = Some(InspectionStop::Deadline);
        } else if stopped.is_none()
            && request
                .cancel
                .is_some_and(|cancel| cancel.load(std::sync::atomic::Ordering::SeqCst))
        {
            stopped = Some(InspectionStop::Cancelled);
        }
        Ok(ReadOutcome {
            requested_path: request.path.to_path_buf(),
            offset: request.offset,
            bytes,
            file_len: prepared.metadata.len(),
            truncated,
            stopped,
            observed_at_unix_ms: now_ms(),
        })
    }

    // 复用单个 guard；实际 scope/grant 在 Authorizer 返回后重读，独立连接仍可撤权。
    fn require_read_terminal(
        &self,
        request: &InspectionRequest<'_>,
        authorizer: &dyn diskgraph_core::Authorizer,
    ) -> Result<(), EngineError> {
        let control = self.control_store()?;
        if control.scope(request.scope_id)?.revoked {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        Self::require_with_control(
            &control,
            authorizer,
            request.principal,
            &diskgraph_core::Permission::ContentRead,
            request.scope_id,
        )?;
        if control.live_permission(
            request.principal,
            &diskgraph_core::Permission::ContentRead,
            request.scope_id,
        )? == Some(false)
        {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        Ok(())
    }
}
