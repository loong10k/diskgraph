use diskgraph_disktree_core::scan::ScanSnapshot;
use std::collections::HashMap;
use std::sync::Mutex;

/// 扫描进度仅保留活动认领代次，退出和失败时释放，旧 owner 不能污染新代次。
pub(crate) struct ScanProgressGuard<'a> {
    pub(crate) entries: &'a Mutex<HashMap<(String, u64), ScanSnapshot>>,
    pub(crate) key: (String, u64),
}
impl Drop for ScanProgressGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(&self.key);
        }
    }
}
