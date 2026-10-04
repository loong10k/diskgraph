## Purpose

管理扫描、查询、内容核验和文件操作在并发、断线、崩溃及磁盘不足时的行为，确保索引复用不会造成重复任务，审计与恢复记录不依赖易丢失的客户端状态且自身占用可控。

## ADDED Requirements

### Requirement: RT-09 Maintainable Rust Engine boundaries
The Engine crate SHALL keep lib.rs and mod.rs to module declarations and explicit reexports. Every production source file SHALL define at most one object, including private records, traits and aliases, and SHALL contain fewer than 500 physical lines. Real implementations SHALL be grouped by responsibility; a file split SHALL NOT introduce placeholder implementations or duplicate state owners. Types SHALL have Chinese documentation with actual native provenance, and public functions/methods SHALL document their parameters and return semantics in Chinese. Production wildcard imports SHALL be absent.

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
- **WHEN** owner 租约过期并被新 owner 接管
- **THEN** 新 owner 从头扫描，旧 owner 不得发布。

#### Scenario: Synchronous CLI recovers an expired owner
- **WHEN** a CLI explicitly waits for one job and its foreign owner expires without a long-lived runner
- **THEN** that CLI conditionally reclaims only that job, rescans with a new fencing generation and leaves unrelated queued jobs untouched
- **AND** it does not preempt a live lease; failed or cancelled terminal states return a nonzero outcome

#### Scenario: Cancelled or revoked expired owner without a runner
- **WHEN** the job being explicitly waited on has an expired owner and a persisted cancellation or revoked scope
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

### Requirement: RT-07 Explicit history reclamation
The CLI SHALL preview `snapshots prune --scope S --keep-last N` without deletion and SHALL require `--apply` to remove old history. Reclamation SHALL preserve latest revisions, pins, and data needed by operation or recovery records. Ambiguous legacy references SHALL cause conservative retention.

#### Scenario: Preview followed by apply
- **GIVEN** three unpinned revisions and no operation references
- **WHEN** keep-last is one
- **THEN** preview changes no data and apply preserves the current revision


### Requirement: RT-10 Maintainable MCP source boundaries
The MCP crate SHALL keep its library entry to standard module declarations and explicit public reexports. Existing public module paths, McpConfig/McpService/STDIO_PRINCIPAL/serve_stdio exports, method signatures, defaults and wire fields SHALL remain compatible. Real configuration, service state, dispatch, scope access, query adapters and stdio framing SHALL have responsibility modules without duplicate Engine or request-state owners. Production files SHALL contain fewer than 500 physical lines and at most one object, with Chinese documentation stating actual native provenance and public parameter/return semantics; wildcard imports and placeholder implementations SHALL be absent.

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
