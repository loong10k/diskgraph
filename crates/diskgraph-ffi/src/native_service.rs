use crate::native_lifecycle::NativeLifecycle;
#[cfg(test)]
use crate::native_scan_gate::NativeScanProgressHook;
use crate::{JobHandle, NativeServiceError, native_reply, open_engine};
use diskgraph_engine::Engine;
use diskgraph_store::SqliteSnapshotStore;
use serde_json::{Value, json};
#[cfg(test)]
use std::sync::Mutex;
use std::sync::{Arc, atomic::AtomicBool};
#[cfg(test)]
#[path = "native_reply_budget_tests.rs"]
mod native_reply_budget_tests;

/// 可信本地的持久原生会话：共享 Engine，每次请求重新授权，关闭时取消关联任务。
/// 来源：DiskGraph 原生 Rust UniFFI 绑定；无 Java 对应对象。
#[derive(uniffi::Object)]
pub struct NativeService {
    pub(crate) engine: Arc<Engine>,
    lifecycle: Arc<NativeLifecycle>,
    closed: Arc<AtomicBool>,
    #[cfg(test)]
    scan_progress_hook: Mutex<Option<NativeScanProgressHook>>,
}

#[uniffi::export]
impl NativeService {
    /// 打开一个数据库的可信本地会话；失败返回类型错误，不启用半初始化服务。
    #[cfg_attr(
        doc,
        doc = "打开一个数据库的可信本地会话；失败返回类型错误，不启用半初始化服务。\n打开可信本地持久会话。\n参数：database_path 为目标图库路径。\n返回：共享会话或类型化初始化错误，不返回半初始化对象。"
    )]
    #[uniffi::constructor]
    pub fn new(database_path: String) -> Result<Arc<Self>, NativeServiceError> {
        let engine = open_engine(&database_path)
            .map_err(|message| NativeServiceError::Unavailable { reason: message })?;
        let closed = Arc::new(AtomicBool::new(false));
        let limit =
            diskgraph_engine::EngineConfig::default().max_active_jobs_per_principal as usize;
        Ok(Arc::new(Self {
            engine: Arc::new(engine),
            lifecycle: Arc::new(NativeLifecycle::new(closed.clone(), limit)),
            closed,
            #[cfg(test)]
            scan_progress_hook: Mutex::new(None),
        }))
    }

    /// 关闭会话并请求取消所有尚存作业句柄；之后的请求明确失败。
    #[cfg_attr(
        doc,
        doc = "关闭会话并请求取消所有尚存作业句柄；之后的请求明确失败。\n关闭会话并请求取消所有现存关联作业。\n参数：无额外输入，使用当前会话作业登记。\n返回：无；后续请求明确失败。"
    )]
    pub fn shutdown(&self) {
        self.lifecycle.close();
    }

    /// 从后台扫描本地目录，返回立即可轮询和取消的句柄。
    #[cfg_attr(
        doc,
        doc = "从后台扫描本地目录，返回立即可轮询和取消的句柄。\n启动本机根目录扫描并复用同根活动句柄。\n参数：root_path 为可规范化的扫描根路径。\n返回：共享作业句柄或已关闭、路径及状态错误。"
    )]
    pub fn spawn_scan(&self, root_path: String) -> Result<Arc<JobHandle>, NativeServiceError> {
        let _admission = self.lifecycle.admit()?;
        #[cfg(test)]
        crate::native_admission_tests::before_canonicalize();
        self.lifecycle.ensure_open()?;
        self.lifecycle.reap()?;
        let root = std::path::Path::new(&root_path)
            .canonicalize()
            .map_err(|error| NativeServiceError::Unavailable {
                reason: error.to_string(),
            })?;
        let engine = self.engine.clone();
        let root_path = root
            .to_str()
            .ok_or_else(|| NativeServiceError::Unavailable {
                reason: "unsupported: pinned scanner requires a lossless Unicode root".into(),
            })?
            .to_owned();
        #[cfg(test)]
        let scan_progress_hook = self.scan_progress_hook.lock().unwrap().take();
        self.lifecycle.register(root, || {
            crate::scan_coordinator::try_spawn_job(move |cancel, progress| {
                #[cfg(test)]
                {
                    crate::run_scan_on_engine(engine, &root_path, cancel, &|value, engine| {
                        progress(value.clone(), engine);
                        if let Some(hook) = &scan_progress_hook {
                            hook(&value);
                        }
                    })
                }
                #[cfg(not(test))]
                {
                    crate::run_scan_on_engine(engine, &root_path, cancel, progress)
                }
            })
        })
    }

    /// 按节点 ID 查询；snapshot 的真实 scope 和实时数据库权限决定访问。
    #[cfg_attr(
        doc,
        doc = "按节点 ID 查询；snapshot 的真实 scope 和实时数据库权限决定访问。\n按实际快照归属与实时权限读取一个节点。\n参数：snapshot_id 为目标快照，node_id 为目标节点。\n返回：有限节点 JSON envelope 或失败。"
    )]
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
    #[cfg_attr(
        doc,
        doc = "按稳定顺序读取目录页，保留 offset 兼容输入，返回实际截断信息。\n按稳定顺序读取目录页并报告实际截断。\n参数：snapshot_id 为快照，parent_id 为父节点，offset/limit 为分页范围。\n返回：含子节点、下一页与预算诊断的 JSON envelope。"
    )]
    pub fn children_json(
        &self,
        snapshot_id: String,
        parent_id: u64,
        offset: u64,
        limit: u32,
    ) -> String {
        native_reply::respond(self.query_until_then(
            &snapshot_id,
            |store, deadline| {
                crate::native_listing::session_children(
                    store,
                    &snapshot_id,
                    parent_id,
                    offset,
                    limit,
                    deadline,
                )
            },
            || {},
        ))
    }

    /// 返回有界候选及覆盖/字节缺口；结果只用于审阅，不构成文件操作授权。
    #[cfg_attr(
        doc,
        doc = "返回有界候选及覆盖/字节缺口；结果只用于审阅，不构成文件操作授权。\n读取供审阅的候选与覆盖和字节缺口。\n参数：snapshot_id 为目标快照，target_bytes 为期望总字节数。\n返回：候选及截断诊断 JSON envelope，不构成操作授权。"
    )]
    pub fn candidates_json(&self, snapshot_id: String, target_bytes: u64) -> String {
        native_reply::respond(self.query_revision_until_then(&snapshot_id,|store, revision, deadline| {
            let answer = store.candidate_selection_for_revision_until(revision,target_bytes,diskgraph_core::QueryBudget::default(), deadline).map_err(|error|error.to_string())?;
            Ok(json!({"candidates":answer.candidates.into_iter().map(|(node,evidence)|json!({"node":node,"evidence":evidence})).collect::<Vec<_>>(),"review_only":true,"complete":answer.complete,"coverage_complete":answer.coverage_complete,"coverage_observed":answer.coverage_observed,"truncated":answer.truncated.map(|reason|reason.wire_name()),"selected_bytes":answer.selected_bytes.to_string(),"remaining_bytes":answer.remaining_bytes.to_string()}))
        }, || {}))
    }
}

impl NativeService {
    /// 在非 UI 后台宿主的普通函数体中运行受管服务，作用域退出前实际回收协调线程。
    /// 参数：database_path 为图库路径，host 取得共享服务和仅在该作用域有效的 owner 能力。
    /// 返回：与 owner 生命周期无关的 R，或构造/最终回收错误；callback panic 在回收后继续传播。
    /// 显式 finalize 和提前 Drop 均执行真实 join；forget 能力也不遗弃库内栈守卫。
    /// 本入口不导出 UniFFI，不支持从 TLS 析构、DllMain 或 UI 调用，不认证 pinned walker 退出。
    pub fn with_owner<R>(
        database_path: String,
        host: impl for<'host> FnOnce(Arc<Self>, crate::NativeServiceOwner<'host>) -> R,
    ) -> Result<R, NativeServiceError> {
        let engine = open_engine(&database_path)
            .map_err(|reason| NativeServiceError::Unavailable { reason })?;
        let closed = Arc::new(AtomicBool::new(false));
        let limit =
            diskgraph_engine::EngineConfig::default().max_active_jobs_per_principal as usize;
        let lifecycle = Arc::new(NativeLifecycle::managed(closed.clone(), limit)?);
        let guard =
            crate::native_service_host_guard::NativeServiceHostGuard::start(lifecycle.clone())?;
        // manager 句柄始终留在此普通栈帧；callback 仅取得借用能力，不能带走所有者。
        let service = Arc::new(Self {
            engine: Arc::new(engine),
            lifecycle,
            closed,
            #[cfg(test)]
            scan_progress_hook: Mutex::new(None),
        });
        let result = host(service, crate::NativeServiceOwner::borrow(&guard));
        guard.finalize_owner()?;
        Ok(result)
    }

    /// 关闭准入并等待 managed 会话的协调线程退出；旧模式明确不支持，UniFFI ABI 不变。
    /// 参数：deadline 为原调用绝对期限；返回：全部准入归还且 coordinator 已 join，或未退场错误。
    /// shutdown/Drop 仍只请求取消；本方法必须在宿主后台线程调用，不代表 pinned scanner drain。
    pub fn drain_coordinators_until(
        &self,
        deadline: std::time::Instant,
    ) -> Result<(), NativeServiceError> {
        self.shutdown();
        self.lifecycle.drain_until(deadline)
    }

    /// 为下一次本会话扫描安装请求局部测试回调，生产构建无此入口。
    #[cfg_attr(
        doc,
        doc = "为下一次本会话扫描安装请求局部测试回调，生产构建无此入口。\n安装下一次扫描的请求局部测试回调。\n参数：hook 为真实进度发布后的测试回调。\n返回：无；仅测试构建包含此方法。"
    )]
    #[cfg(test)]
    pub(crate) fn set_scan_progress_hook(&self, hook: NativeScanProgressHook) {
        *self.scan_progress_hook.lock().unwrap() = Some(hook);
    }

    #[cfg(test)]
    fn query(
        &self,
        snapshot: &str,
        read: impl FnOnce(&SqliteSnapshotStore) -> Result<Value, String>,
    ) -> Result<String, String> {
        self.query_then(snapshot, read, || {})
    }

    #[cfg(test)]
    fn query_then(
        &self,
        snapshot: &str,
        read: impl FnOnce(&SqliteSnapshotStore) -> Result<Value, String>,
        before_reply: impl FnOnce(),
    ) -> Result<String, String> {
        self.query_until_then(snapshot, |store, _| read(store), before_reply)
    }

    fn query_revision_until_then(
        &self,
        snapshot: &str,
        read: impl FnOnce(&SqliteSnapshotStore, &str, std::time::Instant) -> Result<Value, String>,
        before_reply: impl FnOnce(),
    ) -> Result<String, String> {
        let deadline = diskgraph_core::query_deadline(diskgraph_core::QueryBudget::default())
            .map_err(|error| error.to_string())?;
        native_reply::query_with_revision(
            &self.engine,
            snapshot,
            deadline,
            self.closed.clone(),
            read,
            before_reply,
        )
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
