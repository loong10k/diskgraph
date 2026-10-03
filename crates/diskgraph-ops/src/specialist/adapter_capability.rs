//! adapter_capability：既有文件操作职责的原生 Rust 实现。

/// 宿主可以显式允许的专家工具能力及最低版本记录。
/// 来源：DiskGraph 原生 Rust `diskgraph_ops::specialist::AdapterCapability`，保留既有语义。
/// One specialist capability the host may allow: what it is, which program
/// provides it, and what that program must at least be.
#[derive(Clone, Copy, Debug)]
pub struct AdapterCapability {
    /// Stable id used in plans and logs, e.g. `cargo-clean`.
    pub id: &'static str,
    pub display: &'static str,
    /// The program's well-known name. The deployment resolves it to a full
    /// path once, at registration; the runner never searches a PATH.
    pub program: &'static str,
    /// Minimum version accepted by `probe`.
    pub min_version: &'static str,
}

/// The capabilities this build knows about. Knowing is not allowing.
pub const CARGO_CLEAN: AdapterCapability = AdapterCapability {
    id: "cargo-clean",
    display: "Cargo build directory cleanup",
    program: "cargo",
    min_version: "1.70.0",
};

pub const DOCKER_INVENTORY: AdapterCapability = AdapterCapability {
    id: "docker-inventory",
    display: "Docker disk usage inventory",
    program: "docker",
    min_version: "24.0.0",
};

pub const DOCKER_CLEAN: AdapterCapability = AdapterCapability {
    id: "docker-clean",
    display: "Docker exact-object cleanup",
    program: "docker",
    min_version: "24.0.0",
};

/// All capabilities, for listing surfaces.
pub const ALL_CAPABILITIES: &[AdapterCapability] = &[CARGO_CLEAN, DOCKER_INVENTORY, DOCKER_CLEAN];
