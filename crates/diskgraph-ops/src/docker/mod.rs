//! docker：兼容原有公开路径的声明与重导出。
mod cleanup;
mod cleanup_result;
mod command;
mod docker_inventory;
mod docker_object;
mod inventory_query;
mod usage_check;
mod vm_caveat;
pub use cleanup::docker_cleanup;
pub use cleanup_result::CleanupResult;
pub use docker_inventory::DockerInventory;
pub use docker_object::DockerObject;
pub use inventory_query::docker_inventory;
pub use usage_check::{UsageCheck, usage_check};
pub use vm_caveat::VM_CAVEAT;
#[cfg(test)]
mod tests;
