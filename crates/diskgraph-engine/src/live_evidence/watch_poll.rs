//! 比较全树轮询样本并提示事件预算溢出。

use super::{FsEvent, FsEventKind, WatchReport, WatchSnapshot};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::UNIX_EPOCH;

/// 比较全树轮询样本并提示事件预算溢出。
/// 参数：root 为观察根，previous 为原样本，max_events 为诊断阈值。
/// 返回：变化报告并更新 previous；溢出不代表完整订阅。
/// Takes a fresh snapshot of `root` and diffs it against `previous`, which
/// is updated in place. Entries whose metadata cannot be read are skipped;
/// a formerly observed entry can therefore appear Removed. Unavailable
/// modification time uses zero. This poll does not certify full coverage.
pub fn poll_changes(root: &Path, previous: &mut WatchSnapshot, max_events: usize) -> WatchReport {
    let mut fresh: WatchSnapshot = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                stack.push(path);
                continue;
            }
            let mtime = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos() as i64)
                .unwrap_or(0);
            fresh.insert(path, (metadata.len(), mtime));
        }
    }
    let mut events = Vec::new();
    for (path, state) in &fresh {
        match previous.get(path) {
            None => events.push(FsEvent {
                path: path.clone(),
                kind: FsEventKind::Appeared,
            }),
            Some(old) if old != state => events.push(FsEvent {
                path: path.clone(),
                kind: FsEventKind::Modified,
            }),
            Some(_) => {}
        }
    }
    for path in previous.keys() {
        if !fresh.contains_key(path) {
            events.push(FsEvent {
                path: path.clone(),
                kind: FsEventKind::Removed,
            });
        }
    }
    *previous = fresh;
    let overflow = events.len() > max_events;
    WatchReport {
        events,
        overflow,
        rescan_needed: overflow,
    }
}
