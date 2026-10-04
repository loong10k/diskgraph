# PF-06 受管宿主作用域生命周期

当前 `NativeServiceOwner` 拥有静态生命周期且 Drop 同步 join。不可 Send/Sync 不能防止安全 Rust 将其放入 thread_local；Windows TLS 析构持有 loader lock，等待另一线程退出存在结构性死锁风险。此判断不是当前 Windows 测试失败的运行时证明。

## 接口与所有权

新增 `NativeService::with_owner<R>(path, host: impl for<'host> FnOnce(Arc<NativeService>, NativeServiceOwner<'host>) -> R) -> Result<R, NativeServiceError>`。固定 R 与私有不变生命周期标记阻止能力逃逸。删除本轮新增且未导出 UniFFI 的 `new_with_owner`；原 new、stateless 与 19 项 wire ABI 保持。

私有 `NativeServiceHostGuard` 在库函数普通栈上拥有唯一 manager JoinHandle 和 lifecycle。公开 owner 仅借用该 guard，不提供公开构造、Clone/Send/Sync 或句柄导出。callback 的正常返回后显式 finalize 并传播清理错误；unwind 时守卫仍关闭和实际回收，再延续原 panic。

公开能力的显式 finalize 与提前 Drop 仍委托同一个 guard 完成真实 join；不能把早 Drop 降为只取消。重复 finalize 幂等。即便能力 mem::forget，函数仍保留独立 guard 并在 scope 结束回收。Service 可先释放，guard 不依赖 Service 强引用，退出后返回的 Service Arc 已关闭。

保持现有 coordinator manager、真实 join 共享状态、准入上限及 generation/fencing 机制，不新增全局 reaper，不 detach，不设置伪退出标志，不以 is_finished/TLS 消息代替实际 join。

## 验收顺序

1. 原 owning API 的安全 static TLS 存储编译成功作为结构性缺口证据，不把接口缺失编译失败当作行为 RED。
2. 保留已验收真实 TLS finalize 与 owner Drop 用例，迁移到 callback 后所有参与线程在 TLS 进入前启动并确认。
3. 增加真实 manager TLS 阻塞时忘记借用能力的运行回归：callback 的结果已产生，但 with_owner 不能在主动释放和实际 manager join 前返回。
4. 新 API 的正向外部消费者编译通过；返回 owner、static TLS 存储、static Fn/type erase、跨线程和嵌套作用域交换能力的反例因生命周期或 Send 约束拒绝，不因 API 缺失拒绝。
5. 正常返回、提前 Drop、显式 finalize、callback unwind、manager panic、Service 先释放和构造失败均按真实句柄/业务状态验收。
6. 保留原 19 项 UniFFI 校验值、FFI 回归及同 SHA Windows stable/MSRV 与三桌面原生 CI。独立审查后记录实际结果。

## 能力边界

类型系统阻止静态 TLS 保存 owner 能力；不识别直接从 TLS 析构、DllMain 或 UI 调用整个 blocking API。长生命周期宿主应在后台线程普通函数体内执行 callback 的命令循环。调用上下文和正式宿主包仍需原生验收，不据此勾选 9.2、9.8、15.4 或整个 PF-06。

此项只覆盖协调层 manager 和内部 Engine runner。上游 pinned scanner 的实际物理退出和源读取隔离、移动 provider、签名及设备验收保持未完成。
