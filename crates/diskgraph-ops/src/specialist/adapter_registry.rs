//! adapter_registry：既有文件操作职责的原生 Rust 实现。
use crate::OpsError;
use crate::specialist::adapter_capability::ALL_CAPABILITIES;
use crate::specialist::adapter_capability::AdapterCapability;
use std::path::Path;
use std::path::PathBuf;

/// 专家工具显式允许列表及宿主固定的可执行程序路径。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::specialist::AdapterRegistry`，保留既有语义。
/// The allow-list. A capability absent from it cannot be probed, planned, or
/// run, whatever an agent asks for (EC-03).
pub struct AdapterRegistry {
    pub(super) allowed: Vec<(&'static str, PathBuf)>,
}

impl AdapterRegistry {
    /// 创建显式能力允许列表。
    /// 参数：allowed 为能力标识和宿主解析的程序路径列表。
    /// 返回：只允许这些条目的注册表。
    /// Opens a registry that allows exactly the named capabilities, resolved
    /// to the program paths the host configuration recorded.
    pub fn allowing(allowed: &[(&'static str, PathBuf)]) -> Self {
        Self {
            allowed: allowed.to_vec(),
        }
    }

    /// 创建默认拒绝的能力注册表。
    /// 参数：无。
    /// 返回：不允许任何能力的注册表。
    /// A registry that allows nothing; the safe default.
    pub fn empty() -> Self {
        Self {
            allowed: Vec::new(),
        }
    }

    /// 解析已知且显式允许的工具能力。
    /// 参数：id 为能力标识。
    /// 返回：能力静态记录和固定程序路径，未知或未允许则拒绝。
    /// The capability record and its resolved program, or a refusal that
    /// names the allow-list as the reason.
    pub fn capability(&self, id: &str) -> Result<(&'static AdapterCapability, &Path), OpsError> {
        let known = ALL_CAPABILITIES
            .iter()
            .find(|capability| capability.id == id)
            .ok_or_else(|| {
                OpsError::NotAuthorized(format!("no such specialist capability: {id}"))
            })?;
        let (_, program) = self
            .allowed
            .iter()
            .find(|(allowed_id, _)| allowed_id == &known.id)
            .ok_or_else(|| {
                OpsError::NotAuthorized(format!(
                    "specialist capability {id} is not on this deployment's allow-list"
                ))
            })?;
        Ok((known, program.as_path()))
    }

    /// 检查能力是否在允许列表。
    /// 参数：id 为能力标识。
    /// 返回：可解析且允许时为 true。
    /// True when the capability may be used at all.
    pub fn is_allowed(&self, id: &str) -> bool {
        self.capability(id).is_ok()
    }
}
