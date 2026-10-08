#![allow(dead_code, non_camel_case_types)]
extern crate self as libc;
pub type c_int=i32;
pub const RUSAGE_SELF:i32=0; pub const RUSAGE_CHILDREN:i32=-1;
pub struct rusage { pub ru_maxrss:i64 }
pub unsafe fn getrusage(_:i32,_:*mut rusage)->i32 {-1}
#[path="/Users/wandl/workspaces/workspace-loong10k/diskgraph/crates/diskgraph-engine/tests/benchmark_support/peak_memory.rs"] mod peak_memory;
use peak_memory::{rss,child_rss};
#[test] fn failed_host_measurement_is_unknown(){assert_eq!(serde_json::to_value(rss()).unwrap(),serde_json::Value::Null);}
#[test] fn failed_child_measurement_is_unknown(){assert_eq!(serde_json::to_value(child_rss()).unwrap(),serde_json::Value::Null);}
