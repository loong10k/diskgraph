//! verify 的稳定模块路径，仅声明与明确重导出。

mod one_digest;
mod verification;
mod verify_budget;
mod verify_summary;

pub use verification::{verify_same_rows, verify_same_rows_with_limits};
pub use verify_budget::VerifyBudget;
pub use verify_summary::VerifySummary;
