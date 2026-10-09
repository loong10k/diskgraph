## Purpose

管理扫描、查询、内容核验和文件操作在并发、断线、崩溃及磁盘不足时的行为，确保索引复用不会造成重复任务，审计与恢复记录不依赖易丢失的客户端状态且自身占用可控。

## ADDED Requirements

### Requirement: RT-09 Maintainable Rust Engine boundaries
The Engine crate SHALL keep lib.rs and mod.rs to module declarations and explicit reexports.

#### Scenario: Enforce every constraint of RT-09
- **WHEN** the implementation is built, modified, or used
- **THEN** Every production source file SHALL define at most one object, including private records, traits and aliases, and SHALL contain fewer than 500 physical lines. Real implementations SHALL be grouped by responsibility; a file split SHALL NOT introduce placeholder implementations or duplicate state owners. Types SHALL have Chinese documentation with actual native provenance, and public functions/methods SHALL document their parameters and return semantics in Chinese. Production wildcard imports SHALL be absent.

The structural change SHALL preserve existing root exports, the public content/live_evidence/verify module paths, function signatures, tuple aliases, wire fields, error conversions, default values, SQL/JSON constants and observable behavior. Engine SHALL remain the single owner of its graph/control connections, cancellation flags and progress state. Internal visibility MAY change only as needed for the same crate-level collaboration; no new externally accessible fields or helpers SHALL be introduced.

#### Scenario: Production source organization
- **WHEN** the source-layout gate parses every Engine production module, including modules for other target platforms
- **THEN** entries contain only declarations/reexports, each file has at most one object and fewer than 500 lines, Chinese contracts are present, and wildcard imports, unmounted source files and empty/placeholder function implementations are rejected
- **AND** test-only modules are distinguished from production instead of being used to hide production objects

#### Scenario: Structural move preserves runtime contracts
- **WHEN** Engine implementation and return objects move into responsibility modules
- **THEN** the existing graph-before-control lock order, same-guard publication/retention authorization, fenced staging/publication, per-generation cancellation flags, progress cleanup and terminal authorization checks remain unchanged
- **AND** callers compile against the same public paths, the existing behavioral regressions pass, and enabled native platform gates execute at the resulting source SHA

#### Scenario: FFI source organization preserves the binding contract
- **WHEN** the FFI source-layout gate parses all production modules before and after extracting the existing entry implementation
- **THEN** lib.rs contains declarations, explicit exports, exactly one required UniFFI scaffolding macro and exactly two literal ABI includes of the real api_exports.rs/job_handle.rs implementations; these includes retain the old lexical namespace participating in binding checksums, and the gate parses both actual sources once as production rather than hiding objects behind macros; includes in blocks, expressions, inline modules or any other source file are rejected, even when the included basename matches a permitted ABI file
- **AND** real API functions, realm resolution, scan execution, JobHandle, private job state and aliases have separate responsibility files, each with at most one object and fewer than 500 lines; only exact cfg_attr(doc, doc = literal) Chinese rustdoc supplements are accepted where runtime metadata must retain old docstrings
- **AND** Chinese documentation states actual Rust provenance and callable parameter/return semantics; test-only modules and explicit path attributes cannot hide or duplicate production sources
- **AND** public root paths, exported signatures and UniFFI checksum values remain unchanged, including worker cancellation, panic containment, per-database realms, session closure and terminal authorization; structural movement does not certify GUI scheduling, Room coexistence or device behavior

### Requirement: RT-08 Runner lifetime follows its owner
The system SHALL stop scheduling at its next admission check when a runner handle is dropped, including a check after a blocking queue read and before each claim. An idle worker SHALL release its Engine and SQLite connections; drop SHALL not block the caller on an in-flight scan. Work already admitted MAY complete under its existing budgets and lease. Explicit stop MAY wait for current cooperative work to complete. This SHALL not be represented as instantaneous cancellation of an active scan.

#### Scenario: Idle runner released
- **WHEN** a host drops an idle runner and its final Engine reference
- **THEN** the worker stops at its next scheduling boundary and releases the Engine instead of retaining it indefinitely

#### Scenario: Stop during queue lock contention
- **WHEN** a runner is stopped while waiting for the control lock to read its queue
- **THEN** it checks stop after that read, leaves queued jobs unclaimed, and releases its Engine

#### Scenario: Execution unwind stops its lease keeper
- **WHEN** 已认领且仍有真实有效权限的执行任务在发布前抛出 panic，执行栈释放其停止通道
- **THEN** keeper 区分正常轮询超时和通道断连，断连立即停止续租并允许作用域线程实际 join；原 panic 不被吞掉，不要求撤权或额外取消才能返回。该代次取消/进度句柄释放，未提交的结果不发布；这不证明 pinned scanner 的底层 detached 线程已退出。

### Requirement: RT-01 Durable jobs and leases
长任务 SHALL 具有持久 ID、所有者、进度和终态；同 scope 的冲突扫描合并或排队，租约不能只依赖 PID，崩溃后可识别并恢复或明确失败。

#### Scenario: Post-commit WAL maintenance does not hold a running scan lease
- **WHEN** 扫描图库事务已完成提交，随后需要执行可能较慢的 WAL checkpoint
- **THEN** 原子发布回调只覆盖必要的提交和授权检查，不在持有控制库 fence 事务时执行提交后的显式 checkpoint；Engine 先持久结算任务终态，再进行该维护
- **AND** 维护失败不得改写已提交的任务事实；提交前的租约、取消、认证期限检查仍保持，可信旧发布入口继续保留其 WAL 清理行为

#### Scenario: Concurrent sync requests
- **WHEN** 两个客户端同时触发相同范围同步
- **THEN** 获得同一活动作业或明确排队结果，不同时发布相互覆盖快照。

#### Scenario: Git publication and recovery refer to one committed graph result
- **WHEN** 固定目标的 Git 采集批次准备发布
- **THEN** 采集批次、新 revision、真实归属、完整来源选择、latest 的基线 CAS 和唯一 job 发布回执在一个图库事务提交；图库最后一次写入后仍检查原认证到期、取消、租约与运行期限，拒绝时连同回执全部回滚，不改写基线 revision。
- **AND** 新选择保留其他目标的 active 来源，同目标旧 Git 断言仅作为 dependency_only 来源；有界读取或来源闭包不足不得发布为完整选择。

#### Scenario: A committed Git result survives request expiry before control settlement
- **WHEN** 图库回执已提交但控制库尚未保存 Completed，原请求随后到期、被撤权或收到取消
- **THEN** 恢复仅核对不可变回执与原固定输入、实际 job/server/scope/snapshot/revision/run 及发布代次，并按控制库条件更新结算已提交事实；此结算不重新采样、不产生第二次发布，也不以新请求续期原任务。
- **AND** 存活 running 租约不得抢占；没有合法回执的任务仍执行原授权和取消拒绝。该协议不宣称两个数据库具有原子事务或断电不丢图库 NORMAL 提交的保证。

#### Scenario: Pending Git inputs and receipts remain valid during history pruning
- **WHEN** Git 任务 queued 或 running 时执行显式历史回收
- **THEN** 保留其固定基线；发布回执不得随历史级联删除而使原 job 可重复发布。旧二进制不能写入新输入或回执协议，升级前具有一致性备份，迁移失败不启用新服务。

#### Scenario: Git terminal diagnostics remain safe and durable across reconnection
- **WHEN** Git 任务因实际执行/发布失败或真实取消进入终态，或排队任务因撤权、显式取消或范围撤销被终结
- **THEN** 固定阶段与白名单错误码和真实终态在同一控制事务保存；重新连接后可查询，不保存路径、HEAD、引用、原始 Git 输出或错误文本。排队取消为 admission/cancelled，运行任务仅当前有效 owner/fence 可结算。
- **AND** 未知或过长诊断、损坏输入、失效 owner 不得被猜测成合法状态；诊断写入失败回滚终态更新。旧扫描任务字段和取消语义保持不变。

### Requirement: RT-02 Cancellation and backpressure
任务 SHALL 有有界队列、并发数、时间和输出预算；取消信号与已发生副作用分别记录，网络断线不能导致静默丢失业务状态。

#### Scenario: Trusted cancellation remains limited to its owned generation
- **WHEN** 可信本地 coordinator 已认领任务的实际 owner/fence，随后其访问授权被撤销或宿主请求取消
- **THEN** 仅拒绝能力的取消信号不依赖已撤销的结果读取权限；条件更新只作用于同一 running owner/fence，不续租、不发布、不授予查询权限。旧代次的请求不能取消重新认领的新代次或按 job ID 找到的最新取消标志。
- **AND** 请求取消与授权拒绝分别记录；既有授权失败不因共享取消标志而被改写为成功或无条件 Cancelled。协作 keeper 沿用20ms检查，claim 到取消标志登记之间的请求也不能丢失。

#### Scenario: Slow consumer
- **WHEN** 消费者持续慢于扫描数据生产
- **THEN** 背压限制内存，取消后释放资源并保存可查询状态。

#### Scenario: Runner admits a bounded page before request authorization
- **WHEN** 活动队列包含多个主体的旧来源未知、认证到期或可正常执行任务
- **THEN** 每个后台调度 tick 最多物化并尝试认领 64 个候选，按既有创建时间及任务 ID 顺序读取；不先加载全队列或在单一写事务中逐个解码全部请求权限。严格认领逐项将不可执行候选落失败终态后继续，真实取消保持 cancelled，存活 owner 的租约不被抢占。
- **AND** 65 个不同主体的旧来源未知任务后跟随一个有效任务时，第一轮只终结 64 个且不执行第 66 个，第二轮能终结剩余旧任务并实际执行有效任务；原可信完整队列查询签名保留。有限返回行数不代表整库 I/O 或持锁时间的严格常数上限，既有 scope/取消回收成本和新增运行权限检查须单独观测。

### Requirement: RT-03 Authoritative audit and recovery
服务端 SHALL 持久保存批准依据、逐项意图/结果、拒绝、恢复状态与关键身份；恢复记录的保留不依赖客户端或可重建图索引，日志不得记录秘密或无界正文。

#### Scenario: Index history purged
- **WHEN** 按策略淘汰旧图快照
- **THEN** 仍能查询其关联操作和可恢复对象，并保留审阅时必要证据摘要。

### Requirement: RT-04 Capacity budgets
系统 SHALL 对数据库、WAL、staging、日志、备份及回收区分别计量和设置策略，达到容量阈值时暂停或拒绝新任务，不自动永久删除用户资源或恢复数据来腾空间。

#### Scenario: Disk full during operation
- **WHEN** 写执行记录或暂存内容时空间不足
- **THEN** 在不能可靠记账时停止新副作用，保留可核对状态并报告原因。

### Requirement: RT-05 Capability diagnostics
doctor/status SHALL 展示平台、协议、依赖、权限、数据版本、任务积压与容量限制；诊断默认只读，不自动提权、安装、改权限、解锁活跃任务或清理文件。

#### Scenario: Doctor finds missing dependency
- **WHEN** 诊断发现 Docker 不可用或数据库锁定
- **THEN** 提供具体原因和建议，不自动安装或强制移除锁。

### Requirement: RT-06 Executable limits and fenced leases
容量门禁 SHALL 阻止超限任务入队和发布；任务 SHALL 使用条件认领、30 秒租约、5 秒续租及递增 fencing token。扫描器保持原 pin，运行预算通过现有进度接口每 20ms 检查并协作取消。

#### Scenario: Capacity refusal
- **WHEN** 容量报告拒绝新工作
- **THEN** 不创建新任务，不自动删除历史。

#### Scenario: Owner expired
- **WHEN** owner 租约过期、没有已提交的不可变扫描回执并被新 owner 接管
- **THEN** 新 owner 从头扫描，旧 owner 不得发布。

#### Scenario: Scan publication survives a control settlement crash
- **WHEN** Index 或 Sync 已提交图库事务，但在控制库结算完成前进程崩溃
- **THEN** 新 owner 只在原租约失效后核对不可变扫描回执与原 job、主体、scope/server、fence 及真实 revision/snapshot，结算已提交事实，不重新扫描或再次发布。
- **AND** 回执与正式扫描结果同事务写入，拒绝或回滚不留下回执；回执不能被更新、删除或随历史级联回收。缺失回执仍执行原重扫协议，损坏回执明确拒绝，不能降级为缺失。

#### Scenario: Scan recovery preserves admission and terminal execution contracts
- **WHEN** 扫描对账检查已提交回执，且其他调用持有同 Engine 图写互斥锁
- **THEN** 通过有期限的独立只读连接查询已提交账本，不在认领前无限等待共享写锁；原取消、撤权与扫描准备的认领后窗口继续有效。
- **AND** 已完成任务的普通或严格执行重试保留原 Conflict，不重新扫描；显式结算与状态查询保持幂等并保留原结果。

#### Scenario: Synchronous CLI recovers an expired owner
- **WHEN** a CLI explicitly waits for one job, its foreign owner expires without a long-lived runner, and no committed publication receipt exists
- **THEN** that CLI conditionally reclaims only that job, rescans with a new fencing generation and leaves unrelated queued jobs untouched
- **AND** it does not preempt a live lease; failed or cancelled terminal states return a nonzero outcome

#### Scenario: Cancelled or revoked expired owner without a runner
- **WHEN** the job being explicitly waited on has an expired owner and a persisted cancellation or revoked scope, with no committed publication receipt
- **THEN** the waiter conditionally settles only that job as cancelled and reports an incomplete outcome without another 120-second wait
- **AND** live leases and unrelated expired/unclaimable jobs remain unchanged

#### Scenario: Cancellation from another process
- **WHEN** 一个 Engine 取消另一个 Engine 正在扫描的任务
- **THEN** 取消意图持久化且扫描与发布 fence 均能观察，返回成功不允许随后发布。

#### Scenario: Revoked expired job precedes eligible work
- **WHEN** 被撤权 scope 的 running 任务租约过期且队列中还有其他 scope 的任务
- **THEN** 前者持久终结或跳过，其他任务仍可被认领执行。

#### Scenario: Recovered job leaves old staging
- **WHEN** 新 fencing owner 从头扫描并成功发布
- **THEN** 已确认失效的旧代次 staging 被安全回收，当前 owner 的 staging 不被误删。

#### Scenario: Scan observation control contention consumes the original scan deadline
- **WHEN** periodic native scan observation waits for the shared control lock or a competing SQLite writer
- **THEN** lock acquisition, live cancellation/revocation checks and the fencing transaction inherit the original absolute scan deadline, returning BudgetExceeded on expiry
- **AND** expiry does not refresh the budget or admit staging/publication; current owner, durable request authority, scope and permission checks remain required

#### Scenario: A cancellation arrives while scan observation waits for the shared control lock
- **WHEN** observation has actually encountered the held shared control mutex and the original execution cancellation flag becomes set
- **THEN** it stops before that mutex is released, retaining the first original keeper failure when present, instead of waiting until the full scan deadline
- **AND** the original scan/token deadlines and durable authorization/fencing checks are not weakened; synchronous filesystem and SQLite operations are not claimed forcibly interruptible

### Requirement: RT-07 Explicit history reclamation
The CLI SHALL preview `snapshots prune --scope S --keep-last N` without deletion and SHALL require `--apply` to remove old history. Reclamation SHALL preserve latest revisions, pins, and data needed by operation or recovery records. Ambiguous legacy references SHALL cause conservative retention.

#### Scenario: Preview followed by apply
- **GIVEN** three unpinned revisions and no operation references
- **WHEN** keep-last is one
- **THEN** preview changes no data and apply preserves the current revision


### Requirement: RT-10 Maintainable MCP source boundaries
The MCP crate SHALL keep its library entry to standard module declarations and explicit public reexports.

#### Scenario: Enforce every constraint of RT-10
- **WHEN** the implementation is built, modified, or used
- **THEN** Existing public module paths, McpConfig/McpService/STDIO_PRINCIPAL/serve_stdio exports, method signatures, defaults and wire fields SHALL remain compatible. Real configuration, service state, dispatch, scope access, query adapters and stdio framing SHALL have responsibility modules without duplicate Engine or request-state owners. Production files SHALL contain fewer than 500 physical lines and at most one object, with Chinese documentation stating actual native provenance and public parameter/return semantics; wildcard imports and placeholder implementations SHALL be absent.

#### Scenario: Thin service entry preserves the security contract
- **WHEN** the existing MCP library configuration and service implementation move into real modules
- **THEN** the original per-call deadline, profile/schema checks, actual revision ownership, request authority, Git status projection, final authorization and response budgets remain in their existing order
- **AND** trusted local/remote startup, runner ownership, cursor bindings, nullable results, stdout framing and protocol error behavior remain unchanged
- **AND** only crate-internal visibility needed by existing collaborators may change; no new public fields, wrappers or state owners are introduced

#### Scenario: Incremental structure gate states its coverage
- **WHEN** the source-layout gate checks the thin library and every new entry implementation module
- **THEN** it resolves their actual standard module files and rejects hidden inline/path/include implementations, wildcard imports, multiple objects, missing Chinese contracts and empty/todo/unimplemented bodies
- **AND** original service tests retain their assertions in mounted test modules; publicly imported paths and real transport/authorization regressions pass
- **AND** any legacy transport/protocol modules outside that increment are named explicitly, remain registered and do not count as satisfying the whole-crate production-file requirement until they too have been refactored and verified


#### Scenario: Cleanup unwind preserves the original recovery owner
- **WHEN** explicit registry cleanup unwinds after taking a retained child out of its slot and before successful cleanup
- **THEN** the original child returns to the same retained slot before the panic reaches the host, with the original panic payload unchanged
- **AND** the live unreaped child continues to consume the original capacity; a subsequent host drain actually recovers it before releasing the slot
- **AND** cleanup and child destruction occur outside the registry state lock; this scenario does not establish a finite shutdown deadline


#### Scenario: Unborn or invalid native leader never waits for another child
- **WHEN** a native leader has an unborn zero PID or a nonpositive invalid PID and its wait operation is called
- **THEN** it returns an invalid-input error before entering the OS wait operation, including on repeated calls
- **AND** a separate real child can still be waited by its own original owner; valid positive original PID waits and cached exit results remain unchanged


#### Scenario: Fencing generation cannot overflow or change storage type
- **WHEN** queued 或租约已过期任务的 fencing_token 已达 SQLite 有符号整数上限、为负数或不是整数
- **THEN** 条件认领拒绝，不修改 state、owner、heartbeat、lease 或 token，不先提交无效代次后再因解码失败报错。
- **AND** 合法末个代次从上限减一递增到上限仍可认领；该代次失效后不能回绕、提升为浮点数或重用零代次。正常跨进程竞争、实时授权和旧 owner 禁写合同保持不变。

#### Scenario: Windows directory lease retries transient sharing contention within the original budget
- **WHEN** a directory component cannot open because a concurrent native handle causes ERROR_SHARING_VIOLATION
- **THEN** only that typed sharing conflict is retried under the same absolute ProbeBudget deadline and cancellation, without changing the requested access/share modes or reopening through links
- **AND** success still validates the returned original component identity/type/volume; expiry or cancellation terminates retries, and any other native error returns immediately
- **AND** an isolated actual write-access directory handle must establish the sharing conflict; after its release the same lease operation succeeds, while the original implementation fails at the actual sharing-conflict assertion

### Requirement: Startup admission preserves the original deadline before database creation
The CLI SHALL recheck the original absolute startup deadline after recovery-slot admission and before constructing the Engine. Admission SHALL NOT refresh that deadline.

#### Scenario: Native slot open failure retains its primary error
- **WHEN** opening a fixed recovery slot fails
- **THEN** diagnostics may observe the original held directory entry using no-follow metadata only, reporting a coarse type and a separate diagnostic errno without paths or file content
- **AND** the original error is returned without retry, slot deletion, namespace repair or new capacity; a later metadata observation is not treated as proof of the entry state at the failed open

#### Scenario: Admission consumes the remaining startup budget
- **WHEN** recovery admission starts before the original deadline but returns after it
- **THEN** startup refuses with the original budget-exceeded failure before creating the data directory or either database
- **AND** any acquired reservation follows the existing pre-birth abort path; unconfirmed cleanup must not be reported as successful retirement
- **AND** this check does not claim hard interruption of filesystem or SQLite operations or complete supervisor integration

### Requirement: Failed performance measurement process cleanup
性能验收命令普通非零退出时，验收器 MUST 向其独立进程组发送终止信号并保留原退出码；不得仅等待直接进程后继续，将仍运行的同组后代计入后续测量。该机制不声明可以回收逃离进程组的后代。

#### Scenario: Successful load command retains explicitly enabled stage diagnostics
- **WHEN** trusted local diagnostics enable scan phase timing and an individual CLI command succeeds
- **THEN** the load harness preserves only validated fixed stage names, node counts and elapsed milliseconds, with bounded input and at most one latest record per stage
- **AND** arbitrary stderr, paths and content are not relayed; diagnostics remain disabled by default and do not change command results, workload coverage or acceptance deadlines

#### Scenario: Nonzero measurement leaves a same-group helper
- **WHEN** 自建测量进程启动同组helper后以非零状态退出
- **THEN** 验收器保留尚未回收的leader身份，向原进程组发送SIGKILL，再回收leader并保存原失败状态；拒绝将该测量视为成功。发送信号不等于已确认全部后代退休

### Requirement: Engine startup carries the original absolute deadline
Deadline-aware Engine constructors SHALL check the caller's original deadline before data directory creation, between persistent initialization stages and after revision ownership reconciliation. CLI startup SHALL use these constructors on each platform. Existing trusted constructors retain compatibility. Synchronous filesystem and SQLite operations are not forcibly preempted; completed stages are not rolled back or deleted on timeout.

#### Scenario: Original startup deadline has already elapsed
- **WHEN** deadline-aware construction begins at or after its original deadline
- **THEN** it returns BudgetExceeded without creating the data directory

#### Scenario: Persistent initialization consumes the remaining startup budget
- **WHEN** a persistent initialization stage completes after the original deadline
- **THEN** the next check refuses further stages and does not return a usable Engine

### Requirement: MCP startup preserves the original constructor deadline
MCP binary startup SHALL carry the original deployment deadline through Engine initialization and trusted-local bootstrap. Remote startup SHALL NOT bootstrap a local administrator. Legacy trusted library constructors retain compatibility.

#### Scenario: Local or remote service startup is already expired
- **WHEN** deadline-aware MCP startup begins at or after the original deadline
- **THEN** it returns BudgetExceeded before database creation, without returning a service or starting a runner

#### Scenario: Local bootstrap exhausts the startup budget
- **WHEN** the terminal startup budget check fails after local policy bootstrap
- **THEN** startup refuses to return a service and preserves already persisted policy rather than deleting it


### Requirement: RT-07 Scope registration uses one admission deadline
Scope 注册 SHALL 使用原五秒期限覆盖初始持久授权、共享图锁和控制锁等待；等待不得无限阻塞，也不得在获取后一律重建期限。能力回调仍在锁外执行；路径解析和外部同步回调不声明硬实时中断能力。

#### Scenario: Graph writer remains occupied during registration
- **WHEN** 真实共享图库写锁一直被其他调用持有，注册请求原五秒期限耗尽
- **THEN** 请求在锁仍被持有时返回预算错误，不写 scope 或 grants；后续正常注册不继承过期截止时间。


### Requirement: RT-08 Native query policy capture is bounded and request-local
可信本机 FFI 查询 SHALL 在原请求期限内取得一次不可变能力快照，不在数据读取或终态重新全库加载策略。终态仍 SHALL 以数据库实时授权、归属、撤权与会话关闭状态验证响应，不得把请求快照当作绕过实时策略的许可。

#### Scenario: Control owner outlives a native query deadline
- **WHEN** 原生查询在初始能力快照阶段遇到持续持有的控制锁，原期限耗尽
- **THEN** 持锁者尚未释放时查询返回预算错误，不读取节点，也不进入新的等待窗口。


### Requirement: RT-09 Request policy reads only the authenticated principal
MCP/FFI 的有期限请求授权快照 SHALL 通过参数化、可索引的主体条件读取，保留策略版本、撤销、默认拒绝与 token 能力交集。可信内部全策略接口保持兼容；窄接口不得授权其他主体。

#### Scenario: Many unrelated subjects do not amplify a point query policy capture
- **WHEN** 目标主体权限不变，无关主体 grant 从 100 增至 20000
- **THEN** 请求能力构建的 SQLite VM 工作量保持索引查找量级，不解码无关记录；所选主体的允许/拒绝、策略升级和撤销语义与原完整策略一致。

### Requirement: Final response observes withdrawal throughout capability callbacks
最终响应授权 SHALL 在所有宿主能力回调前绑定真实主体和各侧 scope 的负向撤权见证，在回调之后及终态读取之后复核。已观察的撤权优先于预算错误；无可靠原生见证的平台 SHALL 拒绝未知授权代次变化，不以恢复后的实时 grant 抹去窗口内撤权。

#### Scenario: A capability callback revokes and restores the original grant
- **WHEN** 最终响应能力回调通过独立控制连接撤销原元数据 grant 并恢复，然后返回原能力快照的 Allowed
- **THEN** 当前 grant 确实已恢复，最终响应仍拒绝；原生见证报告 PermissionDenied，未知代次变化报告 Conflict，不返回完整或部分数据

#### Scenario: Scan cancellation interrupts control transaction admission
- **WHEN** a scan observation waits for a real competing SQLite writer during its original fence transaction and the original execution is cancelled
- **THEN** transaction boundary retries check that execution's cancellation without waiting for the writer release or refreshing the scan deadline
- **AND** the original keeper stop reason is preserved, no fence work is replayed, and rollback, progress handler and busy timeout cleanup still run

### Requirement: 显式扫描诊断提供有限分项耗时

扫描诊断 MUST 仅在 DISKGRAPH_SCAN_DIAGNOSTICS=1 时额外记录原生观测与编码、staging 写入两项累计墙钟耗时。写入耗时包含锁等待和 fence 检查，不得称为纯 SQL 或 CPU 时间。诊断不得改变原期限、授权、取消和性能门槛。

#### Scenario: 负载工具只转存已知分项
- **WHEN** 开启诊断并收到分项记录、重复记录、未知标签、超长数值或带额外正文的记录
- **THEN** 仅保留两种已知分项的最后一条严格匹配记录，每个数值最多20位；与既有七种阶段合计最多九条
- **AND** 未开启诊断时不转存任何分项记录，日志不包含路径、主体或正文

#### Scenario: Unix 原恢复域并发首次认领
- **WHEN** 多个实际进程在同一个稳定受信原目录中认领固定恢复槽
- **THEN** 已有槽以 no-follow 普通打开，缺失槽才独占创建，创建冲突仅重开同名原目录项一次
- **AND** 原期限、普通文件/owner/0600/单链接、独占锁和 RESERVED/ACTIVE 拒绝继续生效，不修复目录或异常记录，不把父目录丢失当作可恢复成功
