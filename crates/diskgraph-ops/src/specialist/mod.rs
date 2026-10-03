//! specialist：兼容原有公开路径的声明与重导出。
mod adapter_capability;
mod adapter_probe;
mod adapter_registry;
mod adapter_status;
mod cargo_inventory;
mod cleanup_inventory;
mod command_runner;
mod command_spec;
mod inventory_object;
mod run_outcome;
mod sandboxed_runner;
mod specialist_verdict;
pub use adapter_capability::{
    ALL_CAPABILITIES, AdapterCapability, CARGO_CLEAN, DOCKER_CLEAN, DOCKER_INVENTORY,
};
pub use adapter_probe::probe;
pub use adapter_registry::AdapterRegistry;
pub use adapter_status::AdapterStatus;
pub use cargo_inventory::{cargo_inventory, cargo_inventory_with_env};
pub use cleanup_inventory::CleanupInventory;
pub use command_runner::CommandRunner;
pub use command_spec::CommandSpec;
pub use inventory_object::InventoryObject;
pub use run_outcome::RunOutcome;
pub use sandboxed_runner::SandboxedRunner;
pub use specialist_verdict::{SpecialistVerdict, verify_specialist_result};
#[cfg(all(test, unix))]
mod tests;
#[cfg(all(test, windows))]
mod unsupported_windows_tests;
