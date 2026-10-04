//! 源普通文件的流式捕获与内容复核；来源：D35 Git 原生源能力，不保留正文 Vec。
use super::git_metadata_budget::GitMetadataBudget;
use super::git_metadata_version::GitMetadataVersion;
use super::git_private_directory::GitPrivateDirectory;
use super::git_source_directory::GitSourceDirectory;
use super::probe_budget::ProbeBudget;
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

/// 已捕获文件的完整原生版本与字节指纹；来源：原生 Rust scoped Git 捕获。
pub(super) struct GitSourceFile {
    version: GitMetadataVersion,
    digest: [u8; 32],
}
impl GitSourceFile {
    /// 参数：parent/name 为源能力，target/private 为唯一私有 owner，budget/probe 为原预算。
    /// 返回：已保真写入的源版本；读取、权限、精度、容量与源变化失败拒绝。
    pub(super) fn capture(
        parent: &GitSourceDirectory,
        name: &OsStr,
        target: &Path,
        private: &mut GitPrivateDirectory,
        budget: &mut GitMetadataBudget,
        probe: &mut ProbeBudget,
    ) -> Result<Self, String> {
        budget.charge_entry(probe)?;
        let mut file = parent.open_file(name)?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        // 模式、长度和 Unix 版本必须来自同一次 fstat，不能拼接两次观察。
        let version = initial_version(&file, &metadata)?;
        let length = metadata.len();
        if length > budget.remaining_bytes() as u64 {
            probe.mark_resource_limit();
            return Err("git metadata byte limit exceeded".into());
        }
        let digest = private.write_stream(target, length, probe, |output, probe| {
            let digest = read(&mut file, length, budget, probe, |bytes| {
                output.write_all(bytes).map_err(|e| e.to_string())
            })?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                // 仅保留 Git 观察的执行位及读写模式，不复制 setuid/setgid/sticky 能力。
                output
                    .set_permissions(std::fs::Permissions::from_mode(
                        metadata.permissions().mode() & 0o777,
                    ))
                    .map_err(|e| e.to_string())?;
            }
            output
                .set_times(std::fs::FileTimes::new().set_modified(version.modified()))
                .map_err(|e| e.to_string())?;
            if output
                .metadata()
                .and_then(|m| m.modified())
                .map_err(|e| e.to_string())?
                != version.modified()
            {
                return Err("unsupported private Git source timestamp precision".into());
            }
            Ok(digest)
        })?;
        // Windows writer 关闭时才可能结算 LastWriteTime，复用 owner 的关闭后精确时间校验。
        #[cfg(windows)]
        private.set_modified(target, version.modified(), probe)?;
        if !version.same_initial(&self::version(&file)?) {
            return Err("scoped Git source changed during capture".into());
        }
        Ok(Self { version, digest })
    }
    /// 参数：parent/name 为同原根重开的源能力，budget/probe 为原累计预算；返回：身份版本和正文指纹保持时成功。
    pub(super) fn verify(
        &self,
        parent: &GitSourceDirectory,
        name: &OsStr,
        budget: &mut GitMetadataBudget,
        probe: &mut ProbeBudget,
    ) -> Result<(), String> {
        budget.charge_entry(probe)?;
        let mut file = parent.open_file(name)?;
        if !self.version.same_initial(&version(&file)?) {
            return Err("scoped Git source version changed".into());
        }
        let length = file.metadata().map_err(|e| e.to_string())?.len();
        let digest = read(&mut file, length, budget, probe, |_| Ok(()))?;
        if digest != self.digest || !self.version.same_initial(&version(&file)?) {
            return Err("scoped Git source bytes changed".into());
        }
        Ok(())
    }
}
fn version(file: &File) -> Result<GitMetadataVersion, String> {
    initial_version(file, &file.metadata().map_err(|e| e.to_string())?)
}
fn initial_version(
    file: &File,
    metadata: &std::fs::Metadata,
) -> Result<GitMetadataVersion, String> {
    #[cfg(unix)]
    {
        let _ = file;
        GitMetadataVersion::from_metadata(metadata)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let state = crate::windows_file_state::WindowsFileState::capture(file)
            .map_err(|e| e.to_string())?;
        state.validate(false).map_err(|e| e.to_string())?;
        if state.placeholder() {
            return Err("unsupported scoped Git placeholder".into());
        }
        let observed = state.observation(0, 0, diskgraph_core::WindowsTreeAlignment::Unverified);
        if metadata.len() != state.len
            || metadata.last_write_time() != observed.last_write_time as u64
            || metadata.creation_time() != observed.creation_time as u64
            || metadata.file_attributes() != observed.attributes
            || crate::windows_file_state::WindowsFileState::capture(file)
                .map_err(|e| e.to_string())?
                != state
        {
            return Err("scoped Git source changed during initial observation".into());
        }
        Ok(GitMetadataVersion::from_windows_state(
            &state,
            metadata.modified().map_err(|e| e.to_string())?,
        ))
    }
}

fn read(
    file: &mut File,
    length: u64,
    budget: &mut GitMetadataBudget,
    probe: &mut ProbeBudget,
    mut consume: impl FnMut(&[u8]) -> Result<(), String>,
) -> Result<[u8; 32], String> {
    if length > budget.remaining_bytes() as u64 {
        probe.mark_resource_limit();
        return Err("git metadata byte limit exceeded".into());
    }
    let mut remaining = length;
    let mut digest = Sha256::new();
    let mut chunk = [0u8; 4096];
    while remaining > 0 {
        budget.check(probe)?;
        let allowed = chunk
            .len()
            .min(remaining as usize)
            .min(budget.remaining_bytes());
        if allowed == 0 {
            probe.mark_resource_limit();
            return Err("git metadata byte limit exceeded".into());
        }
        let count = file
            .read(&mut chunk[..allowed])
            .map_err(|e| e.to_string())?;
        if count == 0 {
            return Err("scoped Git source shortened during capture".into());
        }
        budget.charge_bytes(count, probe)?;
        consume(&chunk[..count])?;
        digest.update(&chunk[..count]);
        remaining -= count as u64;
    }
    budget.check(probe)?;
    if file.metadata().map_err(|e| e.to_string())?.len() != length {
        return Err("scoped Git source length changed".into());
    }
    Ok(digest.finalize().into())
}

#[cfg(test)]
mod tests;
