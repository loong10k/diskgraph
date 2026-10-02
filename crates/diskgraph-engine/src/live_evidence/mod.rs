//! live_evidence 的稳定模块路径，仅声明与明确重导出。

mod fs_event;
mod fs_event_kind;
mod git_sample;
mod git_usage;
mod process_holder;
mod process_output;
#[cfg(test)]
mod process_output_tests;
mod process_usage;
mod sampling_clock;
#[cfg(test)]
mod tests;
mod usage_coverage;
mod usage_sample;
mod watch_poll;
mod watch_report;
mod watch_snapshot;

pub use fs_event::FsEvent;
pub use fs_event_kind::FsEventKind;
pub use git_sample::GitSample;
pub use git_usage::sample_git;
pub use process_holder::ProcessHolder;
pub use process_usage::sample_process_usage;
pub use usage_coverage::UsageCoverage;
pub use usage_sample::UsageSample;
pub use watch_poll::poll_changes;
pub use watch_report::WatchReport;
pub use watch_snapshot::WatchSnapshot;
