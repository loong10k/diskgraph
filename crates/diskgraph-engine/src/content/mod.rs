//! content 的稳定模块路径，仅声明与明确重导出。

mod conservative_probe;
mod digest_content;
mod digest_outcome;
mod export_policy;
mod inspection_clock;
mod inspection_request;
mod inspection_stop;
mod placeholder_probe;
mod read_content;
mod read_outcome;
mod suspect_groups;
#[cfg(test)]
mod tests;

pub use conservative_probe::ConservativeProbe;
pub use digest_outcome::DigestOutcome;
pub use diskgraph_disktree::HydrationGuard;
pub use export_policy::ExportPolicy;
pub use inspection_request::InspectionRequest;
pub use inspection_stop::InspectionStop;
pub use placeholder_probe::PlaceholderProbe;
pub use read_outcome::ReadOutcome;
pub use suspect_groups::suspects_of;
#[cfg(unix)]
mod unix_identity;
#[cfg(unix)]
pub(crate) use unix_identity::{
    ensure_inside_scope, ensure_plain_file, file_identity, identity_stable,
};
