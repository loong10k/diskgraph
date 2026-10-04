use crate::ProcessEvidenceJobInput;
use crate::process_evidence_codec::{field, key, object};
use serde::{Deserialize, Deserializer, Serialize};

/// 图库事务提交的唯一任务结果回执；来源：原生 Rust D42 / RT-01 / EV-05。
/// 这是已提交事实，不能作为新的内容读取或权限委派。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProcessJobPublicationReceipt {
    schema_version: u32,
    job_id: String,
    input_sha256: String,
    input: ProcessEvidenceJobInput,
    snapshot_id: String,
    revision_id: String,
    run_id: String,
    publishing_fence: u64,
    committed_at_unix_ms: u64,
}
impl ProcessJobPublicationReceipt {
    /// 参数：实际任务、原固定输入、同图库快照/revision/run 和 (fence, Unix毫秒)；返回：有限已提交事实或拒绝。
    pub fn new(
        job_id: String,
        input: ProcessEvidenceJobInput,
        snapshot_id: String,
        revision_id: String,
        run_id: String,
        publishing: (u64, u64),
    ) -> Result<Self, &'static str> {
        if !key(&job_id)
            || !key(&snapshot_id)
            || !key(&revision_id)
            || !key(&run_id)
            || publishing.0 == 0
            || publishing.0 > i64::MAX as u64
            || publishing.1 == 0
            || publishing.1 > i64::MAX as u64
        {
            return Err("invalid job publication receipt");
        }
        Ok(Self {
            schema_version: 1,
            job_id,
            input_sha256: input.digest(),
            input,
            snapshot_id,
            revision_id,
            run_id,
            publishing_fence: publishing.0,
            committed_at_unix_ms: publishing.1,
        })
    }
    /// 参数：无；返回：唯一持久任务 ID。
    pub fn job_id(&self) -> &str {
        &self.job_id
    }
    /// 参数：无；返回：原固定请求 SHA256，不是文件内容摘要。
    pub fn input_sha256(&self) -> &str {
        &self.input_sha256
    }
    /// 参数：无；返回：不可变固定目标与服务端限额。
    pub fn input(&self) -> &ProcessEvidenceJobInput {
        &self.input
    }
    /// 参数：无；返回：实际服务器标识。
    pub fn server_id(&self) -> &crate::ServerId {
        self.input.server_id()
    }
    /// 参数：无；返回：实际范围标识。
    pub fn scope_id(&self) -> &crate::ScopeId {
        self.input.scope_id()
    }
    /// 参数：无；返回：原基线标识，不按新 latest 解释。
    pub fn base_revision_id(&self) -> &str {
        self.input.base_revision_id()
    }
    /// 参数：无；返回：真实复用文件快照。
    pub fn snapshot_id(&self) -> &str {
        &self.snapshot_id
    }
    /// 参数：无；返回：已发布 revision，恢复不能按新 fence 拼接。
    pub fn revision_id(&self) -> &str {
        &self.revision_id
    }
    /// 参数：无；返回：真实已发布采集运行。
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
    /// 参数：无；返回：实际发布时的代次。
    pub fn publishing_fence(&self) -> u64 {
        self.publishing_fence
    }
    /// 参数：无；返回：记录的提交观察时间，Unix毫秒。
    pub fn committed_at_unix_ms(&self) -> u64 {
        self.committed_at_unix_ms
    }
}
impl<'de> Deserialize<'de> for ProcessJobPublicationReceipt {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let o = object(
            &value,
            &[
                "schema_version",
                "job_id",
                "input_sha256",
                "input",
                "snapshot_id",
                "revision_id",
                "run_id",
                "publishing_fence",
                "committed_at_unix_ms",
            ],
        )
        .map_err(serde::de::Error::custom)?;
        if field::<u32>(o, "schema_version").map_err(serde::de::Error::custom)? != 1 {
            return Err(serde::de::Error::custom("unsupported publication receipt"));
        }
        let receipt = Self::new(
            field(o, "job_id").map_err(serde::de::Error::custom)?,
            field(o, "input").map_err(serde::de::Error::custom)?,
            field(o, "snapshot_id").map_err(serde::de::Error::custom)?,
            field(o, "revision_id").map_err(serde::de::Error::custom)?,
            field(o, "run_id").map_err(serde::de::Error::custom)?,
            (
                field(o, "publishing_fence").map_err(serde::de::Error::custom)?,
                field(o, "committed_at_unix_ms").map_err(serde::de::Error::custom)?,
            ),
        )
        .map_err(serde::de::Error::custom)?;
        if field::<String>(o, "input_sha256").map_err(serde::de::Error::custom)?
            != receipt.input_sha256
        {
            return Err(serde::de::Error::custom(
                "publication receipt digest mismatch",
            ));
        }
        Ok(receipt)
    }
}
