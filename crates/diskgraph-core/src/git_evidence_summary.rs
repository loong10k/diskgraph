use crate::git_evidence_codec::{field, object};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

/// 可导出的本地 Git 观察摘要，不含 HEAD、引用或程序原始输出；来源：原生 Rust EV-02 / EC-02。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GitEvidenceSummary {
    dirty_count: u64,
    stash_count: u64,
    ahead_of_upstream: Option<u64>,
    behind_upstream: Option<u64>,
    sampled_at_unix_ms: u64,
    local_coverage_complete: bool,
    remote_state_known: bool,
    upstream_comparison_known: bool,
    notes: Vec<String>,
}
impl GitEvidenceSummary {
    /// 参数：本地安全计数、成对可选 upstream 比较和观察时间；返回：有限摘要或无效观察。
    /// unknown upstream 保持 null，完整本地覆盖不代表网络远端已验证。
    pub fn new(
        dirty_count: u64,
        stash_count: u64,
        ahead_of_upstream: Option<u64>,
        behind_upstream: Option<u64>,
        sampled_at_unix_ms: u64,
    ) -> Result<Self, &'static str> {
        if sampled_at_unix_ms == 0
            || [
                Some(dirty_count),
                Some(stash_count),
                ahead_of_upstream,
                behind_upstream,
                Some(sampled_at_unix_ms),
            ]
            .into_iter()
            .flatten()
            .any(|v| v > i64::MAX as u64)
            || ahead_of_upstream.is_some() != behind_upstream.is_some()
        {
            return Err("invalid Git evidence summary");
        }
        let known = ahead_of_upstream.is_some();
        Ok(Self {
            dirty_count,
            stash_count,
            ahead_of_upstream,
            behind_upstream,
            sampled_at_unix_ms,
            local_coverage_complete: true,
            remote_state_known: false,
            upstream_comparison_known: known,
            notes: if known {
                Vec::new()
            } else {
                vec!["local_upstream_unavailable".into()]
            },
        })
    }
    /// 参数：无；返回：观察到的 dirty 条目数。
    pub fn dirty_count(&self) -> u64 {
        self.dirty_count
    }
    /// 参数：无；返回：观察到的 stash 条目数。
    pub fn stash_count(&self) -> u64 {
        self.stash_count
    }
    /// 参数：无；返回：本地已知 upstream ahead 或未知。
    pub fn ahead_of_upstream(&self) -> Option<u64> {
        self.ahead_of_upstream
    }
    /// 参数：无；返回：本地已知 upstream behind 或未知。
    pub fn behind_upstream(&self) -> Option<u64> {
        self.behind_upstream
    }
    /// 参数：无；返回：实际观察时间，Unix 毫秒。
    pub fn sampled_at_unix_ms(&self) -> u64 {
        self.sampled_at_unix_ms
    }
    /// 参数：无；返回：限定本地采样方法完成其有限覆盖。
    pub fn local_coverage_complete(&self) -> bool {
        self.local_coverage_complete
    }
    /// 参数：无；返回：false，采样不执行远端网络验证。
    pub fn remote_state_known(&self) -> bool {
        self.remote_state_known
    }
    /// 参数：无；返回：本地两个 upstream 计数是否实际已知。
    pub fn upstream_comparison_known(&self) -> bool {
        self.upstream_comparison_known
    }
    /// 参数：无；返回：服务定义的有限未知原因码，绝无原始程序输出。
    pub fn notes(&self) -> &[String] {
        &self.notes
    }
    /// 参数：实际固定目标；返回：canonical 方法/结果观察摘要，不是工作树或正文输入指纹。
    /// 短 TTL 与来源终验仍必需，本摘要不能授予永久有效性或删除权限。
    pub fn observation_fingerprint(&self, input: &crate::GitEvidenceJobInput) -> String {
        let mut hash = Sha256::new();
        hash.update(b"diskgraph-git-observation-v1\0");
        hash.update(input.digest().as_bytes());
        hash.update(serde_json::to_vec(self).expect("finite Git summary serializes"));
        format!("{:x}", hash.finalize())
    }
}
impl<'de> Deserialize<'de> for GitEvidenceSummary {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let o = object(
            &value,
            &[
                "dirty_count",
                "stash_count",
                "ahead_of_upstream",
                "behind_upstream",
                "sampled_at_unix_ms",
                "local_coverage_complete",
                "remote_state_known",
                "upstream_comparison_known",
                "notes",
            ],
        )
        .map_err(serde::de::Error::custom)?;
        let summary = Self::new(
            field(o, "dirty_count").map_err(serde::de::Error::custom)?,
            field(o, "stash_count").map_err(serde::de::Error::custom)?,
            field(o, "ahead_of_upstream").map_err(serde::de::Error::custom)?,
            field(o, "behind_upstream").map_err(serde::de::Error::custom)?,
            field(o, "sampled_at_unix_ms").map_err(serde::de::Error::custom)?,
        )
        .map_err(serde::de::Error::custom)?;
        if field::<bool>(o, "local_coverage_complete").map_err(serde::de::Error::custom)?
            != summary.local_coverage_complete
            || field::<bool>(o, "remote_state_known").map_err(serde::de::Error::custom)?
                != summary.remote_state_known
            || field::<bool>(o, "upstream_comparison_known").map_err(serde::de::Error::custom)?
                != summary.upstream_comparison_known
            || field::<Vec<String>>(o, "notes").map_err(serde::de::Error::custom)? != summary.notes
        {
            return Err(serde::de::Error::custom(
                "invalid Git coverage or unsafe notes",
            ));
        }
        Ok(summary)
    }
}
