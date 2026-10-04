use crate::process_evidence_codec::{field, object};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
/// PID 与实际启动上下文的完整进程身份；来源：原生 Rust D42 / EV-06，不含 command/argv/env。
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProcessStartupIdentity {
    Mac {
        pid: u32,
        start_seconds: u64,
        start_microseconds: u32,
        visibility_domain_sha256: [u8; 32],
    },
    Linux {
        pid: u32,
        start_ticks: u64,
        visibility_domain_sha256: [u8; 32],
    },
    Windows {
        pid: u32,
        creation_ticks: u64,
        visibility_domain_sha256: [u8; 32],
    },
}
impl ProcessStartupIdentity {
    /// 参数：无；返回：内核启动时间和可见域完整可表达，零值不当作已确认上下文。
    pub fn validate(&self) -> Result<(), &'static str> {
        let valid = match self {
            Self::Mac {
                start_seconds,
                start_microseconds,
                ..
            } => *start_seconds > 0 && *start_microseconds < 1_000_000,
            Self::Linux { start_ticks, .. } => *start_ticks > 0,
            Self::Windows { creation_ticks, .. } => *creation_ticks > 0,
        };
        if !valid
            || self.pid() == 0
            || self.pid() > i32::MAX as u32
            || self.visibility_domain_sha256().iter().all(|b| *b == 0)
        {
            return Err("invalid process startup identity");
        }
        Ok(())
    }
    /// 参数：无；返回：实际正整数 PID，不能单独用作持久进程身份。
    pub fn pid(&self) -> u32 {
        match self {
            Self::Mac { pid, .. } | Self::Linux { pid, .. } | Self::Windows { pid, .. } => *pid,
        }
    }
    /// 参数：无；返回：由后端证明的服务器内可见域摘要，不能按任务随机产生。
    pub fn visibility_domain_sha256(&self) -> &[u8; 32] {
        match self {
            Self::Mac {
                visibility_domain_sha256,
                ..
            }
            | Self::Linux {
                visibility_domain_sha256,
                ..
            }
            | Self::Windows {
                visibility_domain_sha256,
                ..
            } => visibility_domain_sha256,
        }
    }
    /// 参数：实际服务器；返回：包括启动上下文的有限实体键，不是进程名称。
    pub fn canonical_key(&self, server: &crate::ServerId) -> String {
        let mut hash = Sha256::new();
        hash.update(b"diskgraph-process-startup-v1\0");
        hash.update(server.as_str().as_bytes());
        hash.update(serde_json::to_vec(self).expect("finite startup serializes"));
        format!("process-{:x}", hash.finalize())
    }
    /// 参数：方法；返回：启动身份平台与固定观察方法一致。
    pub fn matches_method(&self, method: crate::ProcessObservationMethod) -> bool {
        matches!(
            (self, method),
            (
                Self::Mac { .. },
                crate::ProcessObservationMethod::MacLibprocV1
            ) | (
                Self::Linux { .. },
                crate::ProcessObservationMethod::LinuxProcfsV1
            ) | (
                Self::Windows { .. },
                crate::ProcessObservationMethod::WindowsRestartManagerV1
            )
        )
    }
}
impl<'de> Deserialize<'de> for ProcessStartupIdentity {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(deserializer)?;
        let kind = v
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or_else(|| serde::de::Error::custom("missing startup kind"))?;
        let identity = match kind {
            "mac" => {
                let o = object(
                    &v,
                    &[
                        "kind",
                        "pid",
                        "start_seconds",
                        "start_microseconds",
                        "visibility_domain_sha256",
                    ],
                )
                .map_err(serde::de::Error::custom)?;
                Self::Mac {
                    pid: field(o, "pid").map_err(serde::de::Error::custom)?,
                    start_seconds: field(o, "start_seconds").map_err(serde::de::Error::custom)?,
                    start_microseconds: field(o, "start_microseconds")
                        .map_err(serde::de::Error::custom)?,
                    visibility_domain_sha256: field(o, "visibility_domain_sha256")
                        .map_err(serde::de::Error::custom)?,
                }
            }
            "linux" => {
                let o = object(
                    &v,
                    &["kind", "pid", "start_ticks", "visibility_domain_sha256"],
                )
                .map_err(serde::de::Error::custom)?;
                Self::Linux {
                    pid: field(o, "pid").map_err(serde::de::Error::custom)?,
                    start_ticks: field(o, "start_ticks").map_err(serde::de::Error::custom)?,
                    visibility_domain_sha256: field(o, "visibility_domain_sha256")
                        .map_err(serde::de::Error::custom)?,
                }
            }
            "windows" => {
                let o = object(
                    &v,
                    &["kind", "pid", "creation_ticks", "visibility_domain_sha256"],
                )
                .map_err(serde::de::Error::custom)?;
                Self::Windows {
                    pid: field(o, "pid").map_err(serde::de::Error::custom)?,
                    creation_ticks: field(o, "creation_ticks").map_err(serde::de::Error::custom)?,
                    visibility_domain_sha256: field(o, "visibility_domain_sha256")
                        .map_err(serde::de::Error::custom)?,
                }
            }
            _ => return Err(serde::de::Error::custom("unsupported process startup kind")),
        };
        identity.validate().map_err(serde::de::Error::custom)?;
        Ok(identity)
    }
}
