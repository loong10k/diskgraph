use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// 每主体 SSE 槽位，离开连接作用域时自动释放。
pub(crate) struct SseSlot {
    counts: Arc<Mutex<HashMap<String, usize>>>,
    principal: String,
}

impl SseSlot {
    pub fn acquire(counts: Arc<Mutex<HashMap<String, usize>>>, principal: String) -> Option<Self> {
        {
            let mut state = counts.lock().ok()?;
            let count = state.entry(principal.clone()).or_default();
            if *count >= 4 {
                return None;
            }
            *count += 1;
        }
        Some(Self { counts, principal })
    }
}

impl Drop for SseSlot {
    fn drop(&mut self) {
        if let Ok(mut state) = self.counts.lock()
            && let Some(count) = state.get_mut(&self.principal)
        {
            *count = count.saturating_sub(1);
            if *count == 0 {
                state.remove(&self.principal);
            }
        }
    }
}
