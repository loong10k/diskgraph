use crate::process_evidence_codec::{field, object};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

/// 扫描时捕获的对象历代身份；来源：原生 Rust D42 / FS-02，不是执行现场属性。
/// enum 公开字段不构成认证；构造、解码与存储边界必须验证，平台仍须证明字段来源可靠。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IndexedFileEpoch {
    LinuxHandle {
        device: u64,
        inode: u64,
        filesystem_domain_sha256: [u8; 32],
        handle_type: i32,
        handle_bytes: Vec<u8>,
    },
    MacGeneration {
        device: u64,
        inode: u64,
        filesystem_domain_sha256: [u8; 32],
        generation: u32,
        birth_seconds: i64,
        birth_nanos: u32,
    },
    Windows {
        volume: u64,
        file_id: [u8; 16],
        creation_ticks: i64,
    },
}
impl IndexedFileEpoch {
    /// 参数：无；返回：结构可表达可靠平台身份，未知/零 generation 或裸 inode 明确拒绝。
    /// 此处不证明普通用户后端可获取可靠 generation，也不把 birth/change counter 升格为 epoch。
    pub fn validate(&self) -> Result<(), &'static str> {
        let valid = match self {
            Self::LinuxHandle {
                inode,
                filesystem_domain_sha256,
                handle_type,
                handle_bytes,
                ..
            } => {
                *inode != 0
                    && filesystem_domain_sha256.iter().any(|b| *b != 0)
                    && *handle_type >= 0
                    && !handle_bytes.is_empty()
                    && handle_bytes.len() <= 128
            }
            Self::MacGeneration {
                inode,
                filesystem_domain_sha256,
                generation,
                birth_nanos,
                ..
            } => {
                *inode != 0
                    && filesystem_domain_sha256.iter().any(|b| *b != 0)
                    && *generation != 0
                    && *birth_nanos < 1_000_000_000
            }
            Self::Windows {
                file_id,
                creation_ticks,
                ..
            } => file_id.iter().any(|b| *b != 0) && *creation_ticks > 0,
        };
        if valid {
            Ok(())
        } else {
            Err("unverified indexed file epoch")
        }
    }
    /// 参数：无；返回：完整字段规范编码的身份摘要，不是正文摘要或永久平台保证。
    pub fn digest(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).expect("finite epoch serializes"))
        )
    }
}
impl<'de> Deserialize<'de> for IndexedFileEpoch {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let kind = value
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or_else(|| serde::de::Error::custom("missing epoch kind"))?;
        let epoch = match kind {
            "linux_handle" => {
                let o = object(
                    &value,
                    &[
                        "kind",
                        "device",
                        "inode",
                        "filesystem_domain_sha256",
                        "handle_type",
                        "handle_bytes",
                    ],
                )
                .map_err(serde::de::Error::custom)?;
                Self::LinuxHandle {
                    device: field(o, "device").map_err(serde::de::Error::custom)?,
                    inode: field(o, "inode").map_err(serde::de::Error::custom)?,
                    filesystem_domain_sha256: field(o, "filesystem_domain_sha256")
                        .map_err(serde::de::Error::custom)?,
                    handle_type: field(o, "handle_type").map_err(serde::de::Error::custom)?,
                    handle_bytes: field(o, "handle_bytes").map_err(serde::de::Error::custom)?,
                }
            }
            "mac_generation" => {
                let o = object(
                    &value,
                    &[
                        "kind",
                        "device",
                        "inode",
                        "filesystem_domain_sha256",
                        "generation",
                        "birth_seconds",
                        "birth_nanos",
                    ],
                )
                .map_err(serde::de::Error::custom)?;
                Self::MacGeneration {
                    device: field(o, "device").map_err(serde::de::Error::custom)?,
                    inode: field(o, "inode").map_err(serde::de::Error::custom)?,
                    filesystem_domain_sha256: field(o, "filesystem_domain_sha256")
                        .map_err(serde::de::Error::custom)?,
                    generation: field(o, "generation").map_err(serde::de::Error::custom)?,
                    birth_seconds: field(o, "birth_seconds").map_err(serde::de::Error::custom)?,
                    birth_nanos: field(o, "birth_nanos").map_err(serde::de::Error::custom)?,
                }
            }
            "windows" => {
                let o = object(&value, &["kind", "volume", "file_id", "creation_ticks"])
                    .map_err(serde::de::Error::custom)?;
                Self::Windows {
                    volume: field(o, "volume").map_err(serde::de::Error::custom)?,
                    file_id: field(o, "file_id").map_err(serde::de::Error::custom)?,
                    creation_ticks: field(o, "creation_ticks").map_err(serde::de::Error::custom)?,
                }
            }
            _ => return Err(serde::de::Error::custom("unsupported epoch kind")),
        };
        epoch.validate().map_err(serde::de::Error::custom)?;
        Ok(epoch)
    }
}
