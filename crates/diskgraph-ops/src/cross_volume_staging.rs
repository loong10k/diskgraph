//! cross_volume_staging：既有文件操作职责的原生 Rust 实现。
#[cfg(target_os = "macos")]
use crate::bound_path;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::cross_volume_copy::CrossVolumeCopy;
#[cfg(target_os = "macos")]
use crate::metadata_fidelity;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::ops_error::OpsError;
#[cfg(target_os = "macos")]
use crate::source_evidence::identity_of;
#[cfg(target_os = "macos")]
use crate::verified_source;
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::path::Path;

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl CrossVolumeCopy {
    /// 在独占暂存文件中完成复制验证。
    /// 参数：source 为源路径；expected_identity 为可选预期身份；演练上限取源长度。
    /// 返回：验证后的处理字节或明确不支持、变化和 I/O 错误。
    /// Copies the source into staging, then verifies the byte count and that
    /// the source itself did not change underneath the copy.
    #[cfg(all(test, target_os = "macos"))]
    pub(crate) fn stage_and_verify(
        &self,
        source: &Path,
        expected_identity: &Option<String>,
    ) -> Result<u64, OpsError> {
        let budget = std::fs::symlink_metadata(source)?.len();
        self.stage_and_verify_bounded(source, expected_identity, budget, None, &|| Ok(()))
    }

    /// 保持已批准元数据进行有界复制及保真验证。
    /// 参数：source 为源路径；expected_identity 为预期身份；max_bytes 为读取上限；approved 固定批准版本；check_live 复核实时授权。
    /// 返回：完整已验证暂存字节或前置条件、保真及平台错误。
    #[cfg(target_os = "macos")]
    pub(crate) fn stage_and_verify_bounded(
        &self,
        source: &Path,
        expected_identity: &Option<String>,
        max_bytes: u64,
        approved: Option<&std::fs::Metadata>,
        check_live: &dyn Fn() -> Result<(), OpsError>,
    ) -> Result<u64, OpsError> {
        use sha2::Digest;
        use std::io::{Read, Seek, Write};
        let _hydration = diskgraph_engine::content::HydrationGuard::enter()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        check_live()?;
        let before = std::fs::symlink_metadata(source)?;
        if !before.is_file() || before.file_type().is_symlink() {
            return Err(OpsError::Stale("source is not a plain file".into()));
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let source_handle = bound_path::BoundPath::open(source)?;
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let mut input = source_handle.read()?;
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let mut input = std::fs::File::open(source)?;
        // 批准元数据固定预期版本；此处不把纳秒表示或持有句柄声明为原子内容快照。
        let initial = input.metadata()?;
        if let Some(approved) = approved {
            use std::os::unix::fs::MetadataExt;
            if initial.dev() != approved.dev()
                || initial.ino() != approved.ino()
                || initial.len() != approved.len()
                || initial.mtime() != approved.mtime()
                || initial.mtime_nsec() != approved.mtime_nsec()
                || initial.ctime() != approved.ctime()
                || initial.ctime_nsec() != approved.ctime_nsec()
            {
                return Err(OpsError::Stale(
                    "source differs from approved copy version".into(),
                ));
            }
        }
        if initial.len() > max_bytes {
            return Err(OpsError::Stale(
                "source exceeds approved copy byte budget".into(),
            ));
        }
        if expected_identity
            .as_ref()
            .is_some_and(|expected| identity_of(source, &initial).as_ref() != Some(expected))
        {
            return Err(OpsError::Stale("source changed before copy".into()));
        }
        metadata_fidelity::preflight(&input)?;
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let mut output = self.staged_handle.create()?;
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let mut output = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&self.staged)?;
        if identity_of(source, &initial) != identity_of(source, &before) {
            return Err(OpsError::Stale("source was replaced before copy".into()));
        }
        let mut source_hash = sha2::Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        let mut copied = 0_u64;
        while copied < initial.len() {
            check_live()?;
            if std::time::Instant::now() >= deadline {
                return Err(OpsError::Stale("copy deadline exceeded".into()));
            }
            use std::os::unix::fs::MetadataExt;
            let current = input.metadata()?;
            if current.len() != initial.len()
                || current.mtime() != initial.mtime()
                || current.mtime_nsec() != initial.mtime_nsec()
                || current.ctime() != initial.ctime()
                || current.ctime_nsec() != initial.ctime_nsec()
            {
                return Err(OpsError::Stale("source changed during bounded copy".into()));
            }
            let want = usize::try_from(
                initial
                    .len()
                    .saturating_sub(copied)
                    .min(max_bytes.saturating_sub(copied)),
            )
            .unwrap_or(buffer.len())
            .min(buffer.len());
            if want == 0 {
                return Err(OpsError::Stale("copy byte budget exhausted".into()));
            }
            let count = input.read(&mut buffer[..want])?;
            if count == 0 {
                break;
            }
            output.write_all(&buffer[..count])?;
            source_hash.update(&buffer[..count]);
            copied = copied.saturating_add(count as u64);
        }
        output.set_permissions(initial.permissions())?;
        check_live()?;
        output.set_times(
            std::fs::FileTimes::new()
                .set_accessed(initial.accessed()?)
                .set_modified(initial.modified()?),
        )?;
        output.sync_all()?;
        output.rewind()?;
        let mut staged_hash = sha2::Sha256::new();
        let mut verified_bytes = 0u64;
        loop {
            check_live()?;
            if std::time::Instant::now() >= deadline {
                return Err(OpsError::Stale("verification deadline exceeded".into()));
            }
            let count = output.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            staged_hash.update(&buffer[..count]);
            verified_bytes = verified_bytes.saturating_add(count as u64);
            if verified_bytes > initial.len() {
                return Err(OpsError::Stale("staging grew during verification".into()));
            }
        }
        let after = input.metadata()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if initial.len() != after.len()
                || initial.mtime() != after.mtime()
                || initial.mtime_nsec() != after.mtime_nsec()
                || initial.ctime() != after.ctime()
                || initial.ctime_nsec() != after.ctime_nsec()
            {
                return Err(OpsError::Stale("source changed during copy".into()));
            }
        }
        if source_hash.finalize() != staged_hash.finalize() {
            return Err(OpsError::Stale("staged content digest mismatch".into()));
        }
        let check_metadata = || {
            check_live()?;
            if std::time::Instant::now() >= deadline {
                return Err(OpsError::Stale("copy metadata deadline exceeded".into()));
            }
            Ok(())
        };
        check_metadata()?;
        metadata_fidelity::copy_attributes(&input, &output, &check_metadata)?;
        {
            use std::os::fd::AsRawFd;
            // 原生复制仅处理有界预检的 ACL 和 stat；xattr 使用上面的有界复制。
            let result = unsafe {
                libc::fcopyfile(
                    input.as_raw_fd(),
                    output.as_raw_fd(),
                    std::ptr::null_mut(),
                    libc::COPYFILE_STAT | libc::COPYFILE_ACL,
                )
            };
            if result != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        output.set_times(
            std::fs::FileTimes::new()
                .set_accessed(initial.accessed()?)
                .set_modified(initial.modified()?),
        )?;
        output.sync_all()?;
        if output.metadata()?.modified()? != initial.modified()?
            || output.metadata()?.permissions() != initial.permissions()
        {
            return Err(OpsError::Stale(
                "required copy metadata verification failed".into(),
            ));
        }
        check_metadata()?;
        metadata_fidelity::verify(&input, &output)?;
        check_metadata()?;
        let staged_len = output.metadata()?.len();
        if copied != staged_len {
            return Err(OpsError::Stale("the staged copy is incomplete".into()));
        }
        // The source must still be the object the plan described: a file that
        // changed while it was being read invalidates the transfer (OP-05).
        let metadata = std::fs::symlink_metadata(source)?;
        if let Some(expected) = expected_identity {
            let actual = identity_of(source, &metadata);
            if actual.as_deref() != Some(expected.as_str()) {
                return Err(OpsError::Stale("the source changed during the copy".into()));
            }
        }
        if metadata.len() != staged_len {
            return Err(OpsError::Stale(
                "the source changed size during the copy".into(),
            ));
        }
        *self
            .verified
            .lock()
            .map_err(|_| OpsError::Stale("transfer state poisoned".into()))? =
            Some((output.try_clone()?, output.metadata()?));
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            *self
                .verified_source
                .lock()
                .map_err(|_| OpsError::Stale("transfer state poisoned".into()))? =
                Some(verified_source::VerifiedSource {
                    path: source_handle,
                    file: input.try_clone()?,
                    metadata: initial,
                });
        }
        Ok(staged_len)
    }

    /// 在独占暂存文件中完成复制验证。
    /// 参数：source 为源路径；expected_identity 为可选预期身份；演练上限取源长度。
    /// 返回：验证后的处理字节或明确不支持、变化和 I/O 错误。
    /// Other platforms have no validated metadata-preserving copy path.
    #[cfg(not(target_os = "macos"))]
    pub(crate) fn stage_and_verify(
        &self,
        _source: &Path,
        _expected_identity: &Option<String>,
    ) -> Result<u64, OpsError> {
        Err(OpsError::Stale(
            "unsupported: copy metadata fidelity has not been verified on this platform".into(),
        ))
    }

    /// 保持已批准元数据进行有界复制及保真验证。
    /// 参数：source 为源路径；expected_identity 为预期身份；max_bytes 为读取上限；approved 固定批准版本；check_live 复核实时授权。
    /// 返回：完整已验证暂存字节或前置条件、保真及平台错误。
    #[cfg(not(target_os = "macos"))]
    pub(crate) fn stage_and_verify_bounded(
        &self,
        source: &Path,
        expected: &Option<String>,
        _max_bytes: u64,
        _approved: Option<&std::fs::Metadata>,
        _check: &dyn Fn() -> Result<(), OpsError>,
    ) -> Result<u64, OpsError> {
        self.stage_and_verify(source, expected)
    }
}
