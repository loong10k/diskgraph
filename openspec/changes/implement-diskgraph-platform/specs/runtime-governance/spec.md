## Purpose

管理扫描、查询、内容核验和文件操作在并发、断线、崩溃及磁盘不足时的行为，确保索引复用不会造成重复任务，审计与恢复记录不依赖易丢失的客户端状态且自身占用可控。

## ADDED Requirements

### Requirement: RT-01 Durable jobs and leases
长任务 SHALL 具有持久 ID、所有者、进度和终态；同 scope 的冲突扫描合并或排队，租约不能只依赖 PID，崩溃后可识别并恢复或明确失败。

#### Scenario: Concurrent sync requests
- **WHEN** 两个客户端同时触发相同范围同步
- **THEN** 获得同一活动作业或明确排队结果，不同时发布相互覆盖快照。

### Requirement: RT-02 Cancellation and backpressure
任务 SHALL 有有界队列、并发数、时间和输出预算；取消信号与已发生副作用分别记录，网络断线不能导致静默丢失业务状态。

#### Scenario: Slow consumer
- **WHEN** 消费者持续慢于扫描数据生产
- **THEN** 背压限制内存，取消后释放资源并保存可查询状态。

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
