//! queries 的稳定模块路径，仅声明与明确重导出。

mod explore_summary;
mod impact_entry;
mod impact_query;
mod impact_result;
mod propagation;
mod query_context;
mod search_query;
#[cfg(test)]
mod tests;

pub use explore_summary::ExploreSummary;
pub use explore_summary::explore;
pub use impact_entry::ImpactEntry;
pub use impact_query::{impact, impact_bounded, impact_bounded_with_neighbors};
pub use impact_result::ImpactResult;
pub use propagation::Propagation;
pub use propagation::impact_propagation;
pub use query_context::{cursor_context, incompatibility_name};
pub use search_query::search_nodes;
