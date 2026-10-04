## Purpose

使 PruneX 和其他原生宿主复用同一 Rust 领域、存储及受控执行能力，同时尊重桌面与移动端权限差异、URI 资源语义和 SQLite 链接约束，避免把编译成功等同于真实平台能力。

## ADDED Requirements

### Requirement: PF-07 Cached native job responses obey live grants
系统 SHALL 对运行中、已完成及合并原生作业的进度与结果重新检查当前任务查看和元数据授权；不得用启动授权快照或缓存结果绕过单项撤权。没有持久策略的可信内部兼容路径保持独立，不进入远程服务。

#### Scenario: Individual permission revocation
- **WHEN** 保留索引写权限而单独撤销元数据读取或任务查看
- **THEN** 运行中进度与新返回的结果被拒绝；已经完成句柄的轮询和缓存结果同样被拒绝，不要求同时撤销两个权限才生效。

### Requirement: PF-01 Shared native API
Rust/Swift/Kotlin 入口 SHALL 使用同一版本化核心服务与授权语义，支持后台作业、分页、取消及结构化错误，不要求 PruneX 经 MCP 子进程调用本机引擎。

#### Scenario: UI cancellation
- **WHEN** Swift 或 Kotlin 宿主取消长扫描
- **THEN** 界面线程不阻塞，作业按契约停止且不错误推进 latest。

#### Scenario: Kotlin desktop library loading
- **WHEN** Kotlin/JVM 宿主在 macOS ARM64/Intel、Linux x64/ARM64 和 Windows x64 加载对应本机 Rust 动态库
- **THEN** 实际扫描 Unicode/空格目录、分页、轮询、旧 API 和关闭重开均通过；临时库可正常清理
- **AND** 此门禁不替代 Android ABI、Room、SAF 或设备验收

### Requirement: PF-02 Separate storage ownership
图索引及服务端操作记录 SHALL 由 Rust 管理，PruneX 自身会话和界面状态由其业务存储管理；同一进程原生数据库依赖需验证链接及生命周期，不因使用两个文件就假定无冲突。

#### Scenario: PruneX missing
- **WHEN** 服务端没有 PruneX 或其数据库
- **THEN** 仍可验证授权并查询操作/恢复记录。

#### Scenario: Two graph files under one parent directory
- **WHEN** 原生宿主同时打开同一目录下两个不同图库路径
- **THEN** 两者使用独立 control/job 存储归属，或在旧库归属不明时明确拒绝；一个任务的完成不得指向另一个图库的 revision。

#### Scenario: GRDB and Rust dynamic library in one process
- **WHEN** macOS Swift 宿主动态链接 DiskGraph，并在同一进程使用固定版本 GRDB 的连接池并发读写宿主数据库，同时扫描及查询独立图索引
- **THEN** 宿主业务行与图节点均完整、两库可关闭后重开，Rust 动态库不得向宿主导出 SQLite 符号
- **AND** 此证据仅覆盖该动态链接组合，静态嵌入、Room 与移动设备需独立验收

### Requirement: PF-03 Platform capability matrix
macOS/Linux/Windows 的路径、身份、占用、回收和操作能力 SHALL 分别实测声明；操作不支持的平台返回 unsupported，不采用危险通用 Shell 降级。

#### Scenario: Windows identity unavailable
- **WHEN** Windows 扫描无法提供可靠卷身份
- **THEN** 历史比较与修改按限制拒绝，不假装跨平台完整支持。

### Requirement: PF-04 Android provider semantics
Android SHALL 接受宿主授权的 SAF/适用媒体观察及 URI，保留 provider 能力、未知大小与授权生命周期；不得把 URI 转成猜测的原生路径或承诺其他应用私有目录清理。

#### Scenario: Provider revokes access
- **WHEN** 扫描或计划后 URI 授权撤销
- **THEN** 后续访问失败并保留 partial/denied，不扩大到文件系统路径。

### Requirement: PF-05 iOS restricted scope
iOS SHALL 从最初契约就限制在 App 自有及用户授予的文档范围，宿主管理安全作用域生命周期、文档协调与恢复能力；不承诺全盘扫描或跨 App 卸载。

#### Scenario: Selected document lifecycle
- **WHEN** 用户选择文档后访问结束或授权失效
- **THEN** 宿主释放访问资源，后续作业重验或拒绝，不保留无限访问假设。

### Requirement: PF-06 Native service lifetime
Native hosts SHALL have a persistent read-only service session. Closing the session SHALL deny subsequent work and cancel active session jobs. Query results SHALL use authorized revision ownership and bounded reads. Existing stateless FFI signatures SHALL remain trusted local compatibility entry points and SHALL not be used for a remote identity.

#### Scenario: Host closes while a scan is running
- **WHEN** a host closes its native service or releases its final scan handle
- **THEN** cancellation is requested cooperatively and new session queries are refused; no detached job is silently treated as a completed host request.

#### Scenario: Finished result waits for its actual coordinator exit
- **WHEN** 工作线程已经保存业务结果，但真实线程仍在执行线程局部析构或退出清理
- **THEN** 非阻塞 poll 可以报告业务结果；阻塞 result 等待唯一实际 coordinator JoinHandle 完成，不能把 finished 标志、消息送达或数据库终态作为线程已经退出的证明。并发等待共享同一 join 结果，不重复消费句柄。

#### Scenario: Coordinator retains its actual inner Engine runner
- **WHEN** 协调线程观察到已提交的 Completed 作业，或者在持有内部 Engine runner 时遇到撤权、错误、超时或 unwind
- **THEN** 正常与异常退出均保留并实际 join 内部 runner 的唯一句柄，包括其线程局部析构；业务终态和 is_finished 提示不能代替实际退出。join 不持有状态或数据库锁。
- **AND** 原请求取消与授权拒绝以独立的原始共享信号传入 Engine，仅在成功 claim 后绑定该 owner/fencing 代次；不能以裸 job ID 取消另一活 owner，也不能改写已经提交的 revision、owner、fencing 或终态。明确拒权优先于同时发生的调用者取消；独立 I/O 或身份冲突不伪装为拒权。
- **AND** 正常完成的 join 不额外请求停止；异常清理仅请求协作停止。本项仍不是 pinned 上游扫描器的物理线程退场验收，不承诺严格扫描内存或调度期限。

#### Scenario: Close races with native scan admission
- **WHEN** spawn_scan 已进入会话但仍在规范化路径，另一线程关闭会话并等待 coordinator 退出
- **THEN** 准入计数在路径 I/O 前登记；关闭后拒绝新准入，先前准入重验关闭状态后不能创建新任务。等待覆盖准入与会话拥有的 coordinator，超时报告未退场并保留等待能力；不在状态或数据库锁内 join。
- **AND** 既有 shutdown 和 Drop 仅请求取消、保持非阻塞；最后订阅者释放仍请求取消，独立 worker 记录不得延长订阅者生命周期。coordinator 等待不是 pinned scanner 的物理 drain 验收，不据此关闭全平台父项。

#### Scenario: Native coordinator retention is bounded without breaking coalescing
- **WHEN** 宿主重复启动扫描，只轮询业务结果而没有调用阻塞结果或 drain
- **THEN** 每受管会话准入和保留 worker 记录分别受既有 Engine 默认主体活动任务额度约束；同根合并先于新 worker 容量检查。真实 join 由宿主明确拥有的独立 join owner 在 registry 锁外执行，回收只读取共享的实际 joined 结果并按原记录身份移除；drain、准入和 poll 的调用栈不得执行阻塞 join。不能以 finished、is_finished 或弱订阅消失代替实际退出，也不能让普通顺序第九次扫描永久拒绝。
- **AND** 仍未退场的线程和未结束的路径准入耗尽额度时明确背压；poll 不负责 join，不新增全局清理线程，不改变 EngineConfig Default，不宣称同步 I/O 或操作系统线程退出具有硬实时期限。

#### Scenario: Host owns coordinator and manager finalization separately
- **WHEN** 后台宿主通过受管 Rust 构造入口取得服务与独立 owner，服务引用可以先被释放
- **THEN** owner 保留唯一 manager JoinHandle，服务和 manager 不反向强持 owner。原绝对期限的 coordinator drain 仅等待实际 coordinator joined 结果；非 UI 宿主的显式 blocking finalization 另行实际 join manager，其线程局部析构未完成时不能报告 manager 已退场。
- **AND** 新 Rust owner 不可 Clone、Send 或 Sync，后台宿主最后释放 owner 时执行相同 finalization 兜底；原 Service/JobHandle shutdown 和 Drop 仍仅请求取消、非阻塞。构造失败不能遗留已启动的无主线程，manager 异常退出不得使 pending 记录假装成功或遗弃待回收句柄。

#### Scenario: Compatibility constructor does not create an unowned manager
- **WHEN** 可信本地调用者继续使用原 new 或静态 FFI 签名，没有取得独立 owner
- **THEN** 不暗中创建无最终 owner 的 manager，原阻塞 result 仍实际等待 coordinator；无法提供有界 drain 的兼容模式明确返回 unsupported，不进入同步 try-join。
- **AND** 此兼容边界不豁免原生宿主完整生命周期验收；Swift/Kotlin、GUI 和设备宿主尚须接入明确 owner 与非 UI finalization，Rust 层通过不代表 PF-06 或上游 scanner 的物理退场完成。

#### Scenario: Scope revoked during host lifetime
- **WHEN** the registered scope is revoked after the native service opens
- **THEN** subsequent node/page queries use current authorization and cannot return the revoked revision.

#### Scenario: Historical query ignores unrelated corrupt node
- **WHEN** a host asks for one locator's growth or a bounded candidate page
- **THEN** unrelated nodes in either revision are not decoded.

#### Scenario: Coalesced native scans
- **WHEN** two native handles request the same active scope scan
- **THEN** both can await the shared durable job and obtain its same revision, without a second ownership-claim failure
- **AND** results resolve the published job/fencing generation rather than an unrelated later scope revision

#### Scenario: Managed owner cannot escape its host execution scope
- **WHEN** 后台 Rust 宿主进入受管服务回调，并取得共享服务与不可 Clone/Send/Sync 的 owner 能力
- **THEN** owner 能力借用库内普通栈上的唯一实际线程所有者，不能通过安全 Rust 返回、写入 static/thread_local 或传入另一线程。能力显式 finalize 或提前 Drop 仍执行相同真实 finalization；受管阻塞入口返回前必须回收 manager。
- **AND** 即使宿主对借用能力调用 mem::forget，库内独立栈守卫仍在正常返回或 unwind 时关闭准入并实际 join，不能遗弃真实句柄或报告伪完成。清理失败必须保留错误，不替代原 panic；不持有数据库或 registry 锁 join。
- **AND** 本约束防止 owner 逃逸至静态 TLS，不能判断调用者是否在 TLS 析构、DllMain 或 UI 中直接调用整个阻塞入口。这些调用上下文仍不受支持，需真实宿主后台线程验收。旧 UniFFI 签名和可信本地兼容构造入口保持不变，不保留新增受管 tuple 构造的逃逸后门。
