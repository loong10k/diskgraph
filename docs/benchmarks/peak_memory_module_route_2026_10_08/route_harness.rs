#![allow(dead_code)]
mod benchmark_support {
#[path="/Users/wandl/workspaces/workspace-loong10k/diskgraph/crates/diskgraph-engine/tests/benchmark_support/peak_memory.rs"] mod peak_memory;
pub(super) use peak_memory::{child_rss, combined_rss, rss};
}
use benchmark_support::{child_rss, combined_rss, rss};
#[test] fn exported_memory_route_is_callable() {assert!(rss().unwrap() > 0);assert_eq!(combined_rss(Some(1),None),None);let _=child_rss();}
