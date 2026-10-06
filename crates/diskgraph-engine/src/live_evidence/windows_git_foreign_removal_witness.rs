use super::git_private_allocation::GitPrivateAllocation;
use super::probe_budget::ProbeBudget;
use super::windows_git_deletion_seal::WindowsGitDeletionSeal;
use super::windows_git_deletion_witness::WindowsGitDeletionWitness;
use super::windows_git_native_id_protocol::WindowsGitNativeIdProtocol;
use super::windows_git_removal_observation::WindowsGitRemovalObservation;
use std::fs::File;
use std::io;

/// 外来枚举对象的只读删除观察责任；来源：PF-06/Windows 完整 ID 通知，无 Java 对等对象。
/// 不登记、不修改、不删除外来对象；原只读句柄与异步通知随清理 owner 保留。
pub(super) struct WindowsGitForeignRemovalWitness {
    identity: GitPrivateAllocation,
    file: Option<File>,
    observation: Option<WindowsGitRemovalObservation>,
}

impl WindowsGitForeignRemovalWitness {
    /// 参数：parent 为原持有目录，id 为完整枚举身份，probe 为原预算，owner 为持久槽。
    /// 返回：只读身份核验和删除前订阅结果；取得通知责任后任何错误仍保留外槽。
    pub(super) fn prepare_into(
        parent: &File,
        id: &[u8; 16],
        probe: &mut ProbeBudget,
        owner: &mut Option<Self>,
    ) -> io::Result<()> {
        probe.check().map_err(io::Error::other)?;
        let parent_identity = GitPrivateAllocation::from_file(parent).map_err(io::Error::other)?;
        let protocol = WindowsGitNativeIdProtocol::from_hint(parent)?;
        // 用原卷完整 ID 只读打开，不请求 DELETE，也不借用同名路径替代原身份。
        let file = WindowsGitDeletionWitness::open_raw_id(parent, id, &protocol)?;
        let identity = GitPrivateAllocation::from_file(&file).map_err(io::Error::other)?;
        if !parent_identity.same_volume(&identity) || !identity.windows_matches_file_id(id) {
            return Err(io::Error::other(
                "foreign removal witness identity mismatch",
            ));
        }
        *owner = Some(Self {
            identity,
            file: Some(file),
            observation: None,
        });
        let original = owner.as_mut().expect("stored foreign observation owner");
        WindowsGitRemovalObservation::prepare_into(
            parent,
            &original.identity,
            probe,
            &mut original.observation,
        )
    }

    /// 参数：id 为同一原枚举身份、probe 为原预算；返回：真正最终移除时 true，否则原错误。
    /// 原对象仍移动、存在其他链接或外部句柄未关闭时，不释放游标和恢复责任。
    pub(super) fn confirm(&mut self, id: &[u8; 16], probe: &mut ProbeBudget) -> io::Result<bool> {
        probe.check().map_err(io::Error::other)?;
        if !self.identity.windows_matches_file_id(id) {
            return Err(io::Error::other("foreign observation identity changed"));
        }
        let observation = self
            .observation
            .as_mut()
            .ok_or_else(|| io::Error::other("foreign removal pre-observation unavailable"))?;
        if let Some(file) = self.file.as_ref() {
            // 删除由外部 actor 执行；只核验原句柄的零链接/pending seal，不签发删除请求。
            WindowsGitDeletionSeal::verify(file, &self.identity)?;
            self.file = None;
        }
        observation.confirm(&self.identity, probe)
    }
}
