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
        let mut prepared = crate::scoped_content::ScopedContent::open(&root, request.path, probe)?;
        if prepared.file.is_none() {
            return Ok(ReadOutcome {
                requested_path: request.path.to_path_buf(),
                offset: request.offset,
                bytes: Vec::new(),
                file_len: prepared.metadata.len(),
                truncated: false,
                stopped: Some(InspectionStop::Placeholder),
                observed_at_unix_ms: now_ms(),
            });
        }
        // 文件先于 prepared 析构，原生父目录与属性租约覆盖整个读取过程。
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
            if bytes.len() as u64 >= request.max_bytes {
                truncated = true;
                break;
            }
            if let Some(cancel) = request.cancel
                && cancel.load(std::sync::atomic::Ordering::SeqCst)
            {
                stopped = Some(InspectionStop::Cancelled);
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
}
