//! 有界摘要与确认检查。

use super::inspection_clock::now_ms;
use super::{DigestOutcome, InspectionRequest, InspectionStop, PlaceholderProbe};
use crate::{Engine, EngineError};
use diskgraph_core::BusinessError;
use sha2::Digest;
use std::io::Read;

impl Engine {
    /// 以旧兼容接口计算有界完整摘要。
    /// 参数：request/probe/authorizer 为内容检查上下文。
    /// 返回：确认摘要或中止状态；撤权保留旧 permission_denied 错误。
    /// Digests one authorized object in bounded chunks with cancellation and
    /// identity checks between chunks (CT-03, task 8.4). An unstable or
    /// cancelled digest is reported as such and must never upgrade a
    /// duplicate suspect.
    pub fn digest_bounded(
        &self,
        request: &InspectionRequest<'_>,
        probe: &dyn PlaceholderProbe,
        authorizer: &dyn diskgraph_core::Authorizer,
    ) -> Result<DigestOutcome, EngineError> {
        let outcome = self.digest_bounded_until(
            request,
            probe,
            authorizer,
            std::time::Instant::now() + std::time::Duration::from_secs(30),
        )?;
        // 旧接口保留撤权错误语义；批量核验用 until 接口保留已读取成本。
        if outcome.stopped == Some(InspectionStop::PermissionRevoked) {
            return Err(EngineError::Business(BusinessError::PermissionDenied));
        }
        Ok(outcome)
    }
}
impl Engine {
    /// 在共享期限内核验内容并保留失败读取成本。
    /// 参数：request/probe/authorizer 为上下文，deadline 为绝对期限。
    /// 返回：实际成本与可选确认摘要；部分/不稳定/撤销不得确认。
    /// 在共享绝对期限之前核验内容；失败和撤权后仍返回实际读取成本。
    pub fn digest_bounded_until(
        &self,
        request: &InspectionRequest<'_>,
        probe: &dyn PlaceholderProbe,
        authorizer: &dyn diskgraph_core::Authorizer,
        deadline: std::time::Instant,
    ) -> Result<DigestOutcome, EngineError> {
        let _hydration = crate::scoped_content::ScopedContent::hydration_guard()?;
        let record = self.require_content_initial_until(request, authorizer, deadline)?;
        let root = record
            .root
            .to_native_path()
            .map_err(|_| EngineError::Business(BusinessError::Unsupported))?;
        let mut prepared = crate::scoped_content::ScopedContent::open(&root, request.path, probe)?;
        if prepared.file.is_none() {
            // 占位诊断也可能调用 provider 并等待；提前返回不能绕过终态撤权。
            // until 接口保留零读取成本，旧兼容接口继续映射 permission_denied。
            let stopped = match self.require_read_terminal(request, authorizer) {
                Err(EngineError::Business(BusinessError::PermissionDenied)) => {
                    InspectionStop::PermissionRevoked
                }
                Err(error) => return Err(error),
                Ok(()) if std::time::Instant::now() >= deadline => InspectionStop::Deadline,
                Ok(())
                    if request
                        .cancel
                        .is_some_and(|cancel| cancel.load(std::sync::atomic::Ordering::SeqCst)) =>
                {
                    InspectionStop::Cancelled
                }
                Ok(()) => InspectionStop::Placeholder,
            };
            return Ok(DigestOutcome {
                requested_path: request.path.to_path_buf(),
                digest_hex: String::new(),
                bytes_digested: 0,
                stopped: Some(stopped),
                observed_at_unix_ms: now_ms(),
            });
        }
        let mut file = prepared
            .file
            .take()
            .ok_or(EngineError::Business(BusinessError::Unsupported))?;
        let chunk = request.chunk_bytes.clamp(1, 64 * 1024).min(
            usize::try_from(request.max_bytes)
                .unwrap_or(usize::MAX)
                .max(1),
        );
        let mut hasher = sha2::Sha256::new();
        let mut total = 0_u64;
        let mut stopped = None;
        let mut buffer = vec![0_u8; chunk];
        loop {
            if std::time::Instant::now() >= deadline {
                stopped = Some(InspectionStop::Deadline);
                break;
            }
            if self
                .require(
                    authorizer,
                    request.principal,
                    &diskgraph_core::Permission::ContentRead,
                    request.scope_id,
                )
                .is_err()
            {
                stopped = Some(InspectionStop::PermissionRevoked);
                break;
            }
            match self.control_store().and_then(|store| {
                store
                    .live_permission(
                        request.principal,
                        &diskgraph_core::Permission::ContentRead,
                        request.scope_id,
                    )
                    .map_err(EngineError::from)
            }) {
                Ok(Some(false)) => {
                    stopped = Some(InspectionStop::PermissionRevoked);
                    break;
                }
                Err(_) => {
                    stopped = Some(InspectionStop::ReadError);
                    break;
                }
                _ => {}
            }
            // 授权或实时策略查询可能等待；耗尽期限后不得再开始一个读取块。
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
            if total >= request.max_bytes {
                if total < prepared.metadata.len() {
                    stopped = Some(InspectionStop::ByteLimit);
                }
                break;
            }
            let remaining = usize::try_from(request.max_bytes - total).unwrap_or(usize::MAX);
            let want = remaining.min(buffer.len());
            let read = match file.read(&mut buffer[..want]) {
                Ok(read) => read,
                Err(_) => {
                    stopped = Some(InspectionStop::ReadError);
                    break;
                }
            };
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
            total += read as u64;
            // 每块重新观察身份和版本；发现变化则作废摘要，但元数据不是原子快照。
            if !prepared.matches(&file) {
                stopped = Some(InspectionStop::Unstable);
                break;
            }
        }
        if stopped.is_none() && (total != prepared.metadata.len() || !prepared.matches(&file)) {
            stopped = Some(InspectionStop::Unstable);
        }
        if stopped.is_none() {
            // EOF/精确预算和最终元数据查询也可能耗时；包括空文件，确认前重验授权。
            if self
                .require(
                    authorizer,
                    request.principal,
                    &diskgraph_core::Permission::ContentRead,
                    request.scope_id,
                )
                .is_err()
            {
                stopped = Some(InspectionStop::PermissionRevoked);
            } else {
                // 可信兼容入口可能没有持久 policy，但数据库 scope 撤销仍然有效。
                match self.control_store().and_then(|store| {
                    store
                        .live_permission(
                            request.principal,
                            &diskgraph_core::Permission::ContentRead,
                            request.scope_id,
                        )
                        .map_err(EngineError::from)
                }) {
                    Ok(Some(false)) => stopped = Some(InspectionStop::PermissionRevoked),
                    Err(_) => stopped = Some(InspectionStop::ReadError),
                    _ => {}
                }
            }
            if stopped.is_none() && std::time::Instant::now() >= deadline {
                stopped = Some(InspectionStop::Deadline);
            } else if stopped.is_none()
                && request
                    .cancel
                    .is_some_and(|cancel| cancel.load(std::sync::atomic::Ordering::SeqCst))
            {
                stopped = Some(InspectionStop::Cancelled);
            }
        }
        Ok(DigestOutcome {
            requested_path: request.path.to_path_buf(),
            digest_hex: if stopped.is_some() {
                String::new()
            } else {
                hex::encode(hasher.finalize_reset())
            },
            bytes_digested: total,
            stopped,
            observed_at_unix_ms: now_ms(),
        })
    }
}
