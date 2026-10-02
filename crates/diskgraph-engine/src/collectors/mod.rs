//! collectors 的稳定模块路径，仅声明与明确重导出。

mod ecosystem;
mod project_batch;
mod project_collector;
#[cfg(test)]
mod tests;

pub use project_batch::ProjectBatch;
pub use project_collector::{COLLECTOR_ID, COLLECTOR_VERSION, RULE_VERSION, collect_projects};
