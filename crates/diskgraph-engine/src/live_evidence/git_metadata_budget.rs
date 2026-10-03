use super::probe_budget::ProbeBudget;

/// 单次 Git 元数据捕获与复核共用的原始输入额度；来源：原生 Rust live_evidence 元数据采样。
/// 管道输出另由 ProbeBudget 计费，避免大 index 消耗子命令的 1 MiB 输出额度。
pub(super) struct GitMetadataBudget {
    remaining_bytes: usize,
    remaining_entries: usize,
}

impl Default for GitMetadataBudget {
    fn default() -> Self {
        Self {
            remaining_bytes: 64 << 20,
            remaining_entries: 32_768,
        }
    }
}

impl GitMetadataBudget {
    /// 建立夹具的元数据限额。参数：max_bytes 为两轮累计原始字节，max_entries 为文件及目录项数。返回：预算或不可表示错误。
    #[cfg(test)]
    pub(super) fn new(max_bytes: usize, max_entries: usize) -> Result<Self, String> {
        if max_bytes > 64 << 20 || max_entries > 32_768 {
            return Err("unsupported git metadata limits".into());
        }
        Ok(Self {
            remaining_bytes: max_bytes,
            remaining_entries: max_entries,
        })
    }

    /// 核查整次采样的期限和取消。参数：probe 为共享采样预算。返回：可继续或明确中止错误。
    pub(super) fn check(&mut self, probe: &mut ProbeBudget) -> Result<(), String> {
        probe.check().map_err(|error| error.to_string())
    }

    /// 对保留的原始元数据字节计费。参数：bytes 为本次实际读取长度，probe 为整次期限。返回：成功或累计超限。
    pub(super) fn charge_bytes(
        &mut self,
        bytes: usize,
        probe: &mut ProbeBudget,
    ) -> Result<(), String> {
        self.check(probe)?;
        self.remaining_bytes = self
            .remaining_bytes
            .checked_sub(bytes)
            .ok_or("git metadata byte limit exceeded")?;
        Ok(())
    }

    /// 对单个元数据文件或目录条目计费。参数：probe 为整次期限。返回：成功或条目超限。
    pub(super) fn charge_entry(&mut self, probe: &mut ProbeBudget) -> Result<(), String> {
        self.check(probe)?;
        self.remaining_entries = self
            .remaining_entries
            .checked_sub(1)
            .ok_or("git metadata entry limit exceeded")?;
        Ok(())
    }

    /// 查询读取前剩余字节，供已知文件长度提前拒绝。参数：无。返回：剩余原始输入字节。
    pub(super) fn remaining_bytes(&self) -> usize {
        self.remaining_bytes
    }

    /// 读取真实剩余条目用于跨项目累计回归。参数：无。返回：尚未消费的输入条目。
    #[cfg(test)]
    pub(super) fn remaining_entries(&self) -> usize {
        self.remaining_entries
    }
}
