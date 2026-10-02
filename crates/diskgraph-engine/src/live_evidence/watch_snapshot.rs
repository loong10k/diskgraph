//! 保留调用方拥有的路径到长度及修改时间采样映射。

use std::collections::BTreeMap;
use std::path::PathBuf;

/// 保留调用方拥有的路径到长度及修改时间采样映射。
/// 来源：原生 Rust diskgraph-engine::live_evidence::WatchSnapshot。
/// The caller-held snapshot of one watched tree: path → (length, mtime ns).
pub type WatchSnapshot = BTreeMap<PathBuf, (u64, i64)>;
