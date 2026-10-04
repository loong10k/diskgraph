use crate::git_evidence_codec::{field, object};
use crate::{GitEvidenceLimits, RevisionId, ScopeId, ServerId};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

/// 固定实际资源与服务端限额的不可变 Git 任务输入；来源：原生 Rust SC-06 / EC-02。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GitEvidenceJobInput {
    schema_version: u32,
    server_id: ServerId,
    scope_id: ScopeId,
    base_revision_id: RevisionId,
    node_id: u64,
    limits: GitEvidenceLimits,
}
impl GitEvidenceJobInput {
    /// 参数：真实本机归属、已授权基线、正整数节点和服务端限额；返回：固定输入或不合法引用。
    pub fn new(
        server_id: ServerId,
        scope_id: ScopeId,
        base_revision_id: String,
        node_id: u64,
        limits: GitEvidenceLimits,
    ) -> Result<Self, &'static str> {
        let server_id = ServerId::new(server_id.as_str()).map_err(|_| "invalid Git server")?;
        let scope_id = ScopeId::new(scope_id.as_str()).map_err(|_| "invalid Git scope")?;
        if node_id == 0 || node_id > i64::MAX as u64 {
            return Err("invalid Git target node");
        }
        let base_revision_id =
            RevisionId::new(base_revision_id).map_err(|_| "invalid Git base revision")?;
        Ok(Self {
            schema_version: 1,
            server_id,
            scope_id,
            base_revision_id,
            node_id,
            limits,
        })
    }
    /// 参数：无；返回：服务端实际标识。
    pub fn server_id(&self) -> &ServerId {
        &self.server_id
    }
    /// 参数：无；返回：已注册实际范围。
    pub fn scope_id(&self) -> &ScopeId {
        &self.scope_id
    }
    /// 参数：无；返回：固定基线标识，不按 latest 重定位。
    pub fn base_revision_id(&self) -> &str {
        self.base_revision_id.as_str()
    }
    /// 参数：无；返回：固定目录节点 ID。
    pub fn node_id(&self) -> u64 {
        self.node_id
    }
    /// 参数：无；返回：原服务端限额，执行不得重置。
    pub fn limits(&self) -> &GitEvidenceLimits {
        &self.limits
    }
    /// 参数：无；返回：规范化字段编码的 SHA256 请求指纹，不宣称文件正文摘要。
    pub fn digest(&self) -> String {
        // 结构 Serialize 的字段顺序固定；无 HashMap 顺序或额外模型字段。
        let encoded = serde_json::to_vec(self).expect("finite Git input serializes");
        format!("{:x}", Sha256::digest(encoded))
    }
}
impl<'de> Deserialize<'de> for GitEvidenceJobInput {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let o = object(
            &value,
            &[
                "schema_version",
                "server_id",
                "scope_id",
                "base_revision_id",
                "node_id",
                "limits",
            ],
        )
        .map_err(serde::de::Error::custom)?;
        if field::<u32>(o, "schema_version").map_err(serde::de::Error::custom)? != 1 {
            return Err(serde::de::Error::custom("unsupported Git input version"));
        }
        Self::new(
            field(o, "server_id").map_err(serde::de::Error::custom)?,
            field(o, "scope_id").map_err(serde::de::Error::custom)?,
            field(o, "base_revision_id").map_err(serde::de::Error::custom)?,
            field(o, "node_id").map_err(serde::de::Error::custom)?,
            field(o, "limits").map_err(serde::de::Error::custom)?,
        )
        .map_err(serde::de::Error::custom)
    }
}
