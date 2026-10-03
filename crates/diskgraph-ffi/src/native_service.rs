use crate::{JobHandle, NativeServiceError, native_reply, open_engine};
use diskgraph_engine::Engine;
use diskgraph_store::SqliteSnapshotStore;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};
type NativeJobs = Vec<(std::path::PathBuf, Weak<JobHandle>)>;
#[cfg(test)]
#[path = "native_reply_budget_tests.rs"]
mod native_reply_budget_tests;

/// 可信本地的持久原生会话：共享 Engine，每次请求重新授权，关闭时取消关联任务。
#[derive(uniffi::Object)]
pub struct NativeService {
    pub(crate) engine: Arc<Engine>,
    // None 表示已经关闭。锁同时覆盖任务登记与关闭，避免漏掉竞争中新建的任务。
    jobs: Mutex<Option<NativeJobs>>,
    closed: Arc<AtomicBool>,
}

#[uniffi::export]
impl NativeService {
    /// 打开一个数据库的可信本地会话；失败返回类型错误，不启用半初始化服务。
    #[uniffi::constructor]
    pub fn new(database_path: String) -> Result<Arc<Self>, NativeServiceError> {
        let engine = open_engine(&database_path)
            .map_err(|message| NativeServiceError::Unavailable { reason: message })?;
        Ok(Arc::new(Self {
            engine: Arc::new(engine),
            jobs: Mutex::new(Some(Vec::new())),
            closed: Arc::new(AtomicBool::new(false)),
        }))
    }

    /// 关闭会话并请求取消所有尚存作业句柄；之后的请求明确失败。
    pub fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Ok(mut state) = self.jobs.lock()
            && let Some(jobs) = state.take()
        {
            for (_, job) in jobs {
                if let Some(job) = job.upgrade() {
                    job.cancel();
                }
            }
        }
    }

    /// 从后台扫描本地目录，返回立即可轮询和取消的句柄。
    pub fn spawn_scan(&self, root_path: String) -> Result<Arc<JobHandle>, NativeServiceError> {
        let root = std::path::Path::new(&root_path)
            .canonicalize()
            .map_err(|error| NativeServiceError::Unavailable {
                reason: error.to_string(),
            })?;
        let mut guard = self
            .jobs
            .lock()
            .map_err(|_| NativeServiceError::Unavailable {
                reason: "session state poisoned".into(),
            })?;
        let jobs = guard
            .as_mut()
            .ok_or_else(|| NativeServiceError::Unavailable {
                reason: "session closed".into(),
            })?;
        if self.closed.load(Ordering::SeqCst) {
            return Err(NativeServiceError::Unavailable {
                reason: "session closed".into(),
            });
        }
        jobs.retain(|(_, job)| job.strong_count() > 0);
        // 同目录在本会话中共用句柄；释放一个订阅者不会取消其他订阅者。
        if let Some(handle) = jobs
            .iter()
            .filter(|(path, _)| path == &root)
            .find_map(|(_, job)| job.upgrade().filter(|job| !job.is_finished()))
        {
            return Ok(handle);
        }
        let engine = self.engine.clone();
        let root_path = root
            .to_str()
            .ok_or_else(|| NativeServiceError::Unavailable {
                reason: "unsupported: pinned scanner requires a lossless Unicode root".into(),
            })?
            .to_owned();
        let handle = crate::spawn_job(move |cancel, progress| {
            crate::run_scan_on_engine(engine, &root_path, cancel, progress)
        });
        jobs.push((root, Arc::downgrade(&handle)));
        Ok(handle)
    }

    /// 按节点 ID 查询；snapshot 的真实 scope 和实时数据库权限决定访问。
    pub fn node_json(&self, snapshot_id: String, node_id: u64) -> String {
        native_reply::respond(self.query_until_then(
            &snapshot_id,
            |store, deadline| {
                let mut budget = diskgraph_core::QueryReadBudget::new(
                    diskgraph_core::QueryBudget::default(),
                    deadline,
                )
                .map_err(|error| error.to_string())?;
                store
                    .node_with_budget(&snapshot_id, node_id, &mut budget)
                    .map(|node| json!(node))
                    .map_err(|error| error.to_string())
            },
            || {},
        ))
    }

    /// 按稳定顺序读取目录页，保留 offset 兼容输入，返回实际截断信息。
    pub fn children_json(
        &self,
        snapshot_id: String,
        parent_id: u64,
        offset: u64,
        limit: u32,
    ) -> String {
        native_reply::respond(self.query(&snapshot_id,|store| {
            let limit = crate::bounded_limit(limit)?.min(100);
            let (nodes,next,unknown) = store.children_page(&snapshot_id,parent_id,None,offset,u64::from(limit)).map_err(|error|error.to_string())?;
            let budget=diskgraph_core::QueryBudget::default().max_response_bytes.saturating_sub(2048);
            let mut items=Vec::new();
            let mut bytes=0usize;
            let mut clipped=false;
            for node in nodes {
                let cost=serde_json::to_vec(&node).map_err(|error|error.to_string())?.len();
                if bytes.saturating_add(cost)>budget { clipped=true;break; }
                bytes+=cost;
                items.push(node);
            }
            if clipped && items.is_empty() { return Err("budget_exceeded: a node cannot fit in the response".into()); }
            let next=if clipped { Some(offset.saturating_add(items.len() as u64)) } else { next };
            Ok(json!({"items":items,"next_offset":next,"unknown_size_count":unknown,"complete":next.is_none(),"truncated":if clipped {Some("response_byte_limit")} else {None}}))
        }))
    }

    /// 返回有界候选及覆盖/字节缺口；结果只用于审阅，不构成文件操作授权。
    pub fn candidates_json(&self, snapshot_id: String, target_bytes: u64) -> String {
        native_reply::respond(self.query_until_then(&snapshot_id,|store, deadline| {
            let answer = store.candidate_selection_until(&snapshot_id,target_bytes,diskgraph_core::QueryBudget::default(), deadline).map_err(|error|error.to_string())?;
            Ok(json!({"candidates":answer.candidates.into_iter().map(|(node,evidence)|json!({"node":node,"evidence":evidence})).collect::<Vec<_>>(),"review_only":true,"complete":answer.complete,"coverage_complete":answer.coverage_complete,"truncated":answer.truncated.map(|reason|reason.wire_name()),"selected_bytes":answer.selected_bytes.to_string(),"remaining_bytes":answer.remaining_bytes.to_string()}))
        }, || {}))
    }
}

impl NativeService {
    fn query(
        &self,
        snapshot: &str,
        read: impl FnOnce(&SqliteSnapshotStore) -> Result<Value, String>,
    ) -> Result<String, String> {
        self.query_then(snapshot, read, || {})
    }

    fn query_then(
        &self,
        snapshot: &str,
        read: impl FnOnce(&SqliteSnapshotStore) -> Result<Value, String>,
        before_reply: impl FnOnce(),
    ) -> Result<String, String> {
        self.query_until_then(snapshot, |store, _| read(store), before_reply)
    }

    fn query_until_then(
        &self,
        snapshot: &str,
        read: impl FnOnce(&SqliteSnapshotStore, std::time::Instant) -> Result<Value, String>,
        before_reply: impl FnOnce(),
    ) -> Result<String, String> {
        let deadline = diskgraph_core::query_deadline(diskgraph_core::QueryBudget::default())
            .map_err(|error| error.to_string())?;
        native_reply::query(
            &self.engine,
            snapshot,
            deadline,
            self.closed.clone(),
            read,
            before_reply,
        )
    }
}

impl Drop for NativeService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shutdown_after_final_authorization_cannot_return_success() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let path = data
            .path()
            .join("graph.sqlite")
            .to_string_lossy()
            .into_owned();
        let scan: Value = serde_json::from_str(&crate::scan_native_json(
            path.clone(),
            root.path().to_string_lossy().into_owned(),
        ))
        .unwrap();
        let snapshot = scan["data"]["snapshot_id"].as_str().unwrap();
        let service = NativeService::new(path).unwrap();
        let answer = service.query_then(
            snapshot,
            |_| Ok(json!("final response")),
            || service.shutdown(),
        );
        assert!(
            answer.is_err(),
            "closed session returned success: {answer:?}"
        );
    }

    #[test]
    fn shutdown_does_not_wait_for_a_read_or_publish_its_response() {
        let root = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let path = data
            .path()
            .join("graph.sqlite")
            .to_string_lossy()
            .into_owned();
        let scan: Value = serde_json::from_str(&crate::scan_native_json(
            path.clone(),
            root.path().to_string_lossy().into_owned(),
        ))
        .unwrap();
        let snapshot = scan["data"]["snapshot_id"].as_str().unwrap().to_owned();
        let service = NativeService::new(path).unwrap();
        let entered = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        let worker_service = service.clone();
        let worker_entered = entered.clone();
        let worker_release = release.clone();
        let worker = std::thread::spawn(move || {
            worker_service.query(&snapshot, |_| {
                worker_entered.wait();
                worker_release.wait();
                Ok(json!("must not escape after shutdown"))
            })
        });
        entered.wait();
        let (sender, receiver) = std::sync::mpsc::channel();
        let closing = service.clone();
        let closer = std::thread::spawn(move || {
            closing.shutdown();
            sender.send(()).unwrap();
        });
        let prompt = receiver
            .recv_timeout(std::time::Duration::from_millis(500))
            .is_ok();
        release.wait();
        closer.join().unwrap();
        assert!(prompt, "shutdown waited for a shared query lock");
        assert!(worker.join().unwrap().is_err());
    }
}
