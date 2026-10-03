use super::git_metadata_budget::GitMetadataBudget;
use super::git_usage::sample_git_using_budget;
use super::probe_budget::ProbeBudget;
use super::process_usage::{sample_process_usage_using_budget, unobservable};
use super::sampling_clock::now_ms;
use super::{GitSample, ProbeLimits, UsageCoverage, UsageSample};
use std::path::Path;

/// 同一多目标证据任务独占的执行与原始输入预算；不能克隆或失败后补充额度。
/// 来源：原生 Rust DiskGraph 实时证据任务设计 D29，无 Java 对应实现。
/// 这是受信库内采样接口；调用方仍须检查 scope 授权、程序来源和目标路径边界。
pub struct EvidenceProbeSession {
    budget: ProbeBudget,
    metadata: Option<GitMetadataBudget>,
    failure: Option<String>,
}

impl EvidenceProbeSession {
    /// 从创建时固定整次任务期限、输出与取消；Git 输入累计限 64 MiB/32768 条目。
    /// 参数：limits 为既有配置，后续目标不重置期限或额度。
    /// 返回：独占会话或配置、到期、取消错误；不建立任何请求授权。
    pub fn new(limits: &ProbeLimits) -> Result<Self, String> {
        let mut budget = ProbeBudget::new(limits).map_err(|error| error.to_string())?;
        budget.check().map_err(|error| error.to_string())?;
        Ok(Self {
            budget,
            metadata: Some(GitMetadataBudget::default()),
            failure: None,
        })
    }

    /// 在任务剩余额度内采样下一项目，准备、复核及清理失败均关闭整次会话。
    /// 参数：git 为受信程序路径，project 为受信项目目录；本方法不解析远程授权。
    /// 返回：完成清理与终态期限检查的样本，或锁存的首次失败；不会恢复默认额度。
    pub fn sample_git(&mut self, git: &Path, project: &Path) -> Result<GitSample, String> {
        self.ready()?;
        let Some(metadata) = self.metadata.take() else {
            return Err(self.close("evidence session metadata budget unavailable".into()));
        };
        match sample_git_using_budget(
            git,
            project,
            &mut self.budget,
            metadata,
            128 << 20,
            64 << 20,
        ) {
            Ok((sample, remaining)) => {
                self.metadata = Some(remaining);
                Ok(sample)
            }
            Err(error) => Err(self.close(error)),
        }
    }

    /// 在同一任务预算内观察下一组路径，保留正向 partial；不可观察关闭会话。
    /// 参数：lsof 为受信程序路径，paths 为精确对象路径；空请求也先检查会话状态。
    /// 返回：既有覆盖语义的样本；首次失败后任何调用均不启动新程序。
    pub fn sample_process_usage(&mut self, lsof: &Path, paths: &[&Path]) -> UsageSample {
        if let Err(error) = self.ready() {
            return unobservable(now_ms(), error);
        }
        let sample = sample_process_usage_using_budget(lsof, paths, &mut self.budget);
        if let UsageCoverage::Unobservable { reason } = &sample.coverage {
            self.close(reason.clone());
        }
        sample
    }

    fn ready(&mut self) -> Result<(), String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        self.budget
            .check()
            .map_err(|error| self.close(error.to_string()))
    }

    fn close(&mut self, error: String) -> String {
        self.failure.get_or_insert(error).clone()
    }

    /// 读取实际余额用于跨目标累计回归。参数：无。返回：剩余输出与可重用元数据字节/条目。
    #[cfg(test)]
    pub(super) fn resources_for_test(&self) -> (usize, Option<(usize, usize)>) {
        (
            self.budget.remaining_bytes(),
            self.metadata
                .as_ref()
                .map(|metadata| (metadata.remaining_bytes(), metadata.remaining_entries())),
        )
    }

    /// 设置确定性输入额度，不补充生产会话。参数：bytes/entries 为夹具初始额度。返回：无，非法配置失败。
    #[cfg(test)]
    pub(super) fn metadata_limits_for_test(&mut self, bytes: usize, entries: usize) {
        self.metadata = Some(GitMetadataBudget::new(bytes, entries).unwrap());
    }

    /// 推进既有期限用于调用间超期回归。参数：无。返回：无，不创建新时钟或额度。
    #[cfg(test)]
    pub(super) fn expire_for_test(&mut self) {
        self.budget.expire_for_test();
    }
}
