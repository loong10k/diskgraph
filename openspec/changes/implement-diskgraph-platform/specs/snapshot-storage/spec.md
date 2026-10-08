## Purpose

保证快照和关系索引在迁移、并发读取、崩溃、满盘及历史淘汰时维持可解释的一致性；保留当前版本契约并防止可重建图索引的清理破坏保护策略或文件恢复记录。

## ADDED Requirements

### Requirement: ST-06 Maintainable Rust storage boundaries
The storage crate SHALL keep lib.rs and mod.rs to module declarations and public reexports. Each production Rust source file SHALL define no more than one type and SHALL contain real implementation rather than placeholders. Public interfaces SHALL keep their existing root exports and wire/SQLite semantics during the structural change. Production wildcard imports SHALL be absent. Types and public methods SHALL have Chinese documentation with actual provenance and parameter/return semantics; a native Rust implementation SHALL not invent a Java counterpart.

#### Scenario: Storage entry and source structure
- **WHEN** the source-layout gate parses production storage modules
- **THEN** entry files only declare modules and reexports, each type has its own file, wildcard imports and placeholder bodies are rejected, and required documentation is present

#### Scenario: Existing storage consumers
- **WHEN** existing engines, bindings, queries, migration and fenced-publication tests use storage after the split
- **THEN** unchanged public exports, SQL/data formats, transaction order, authorization and query budgets retain their prior behavior

### Requirement: ST-01 Immutable atomic publication
系统 SHALL 将未完成批次与可查询版本隔离，原子发布完整或明确标记的 partial 版本；失败和取消不得无声替换最新完成版本。

#### Scenario: Crash during publication
- **WHEN** 进程在发布边界崩溃后重新打开数据库
- **THEN** 旧完成版本仍可读；新版本要么完整发布要么不可见。

### Requirement: ST-02 Explicit v1 migration
系统 SHALL 保留 v1 数据与接口语义，使用显式迁移、备份和版本检查；未保存的身份/覆盖标 unknown，legacy 文本证据不得自动升级为可信关系。

#### Scenario: Insufficient migration space
- **WHEN** 迁移所需额外空间不足
- **THEN** 在修改正式数据前拒绝，原库继续可读。

#### Scenario: Old binary on new schema
- **WHEN** 不兼容旧程序打开升级库
- **THEN** 明确拒绝，不损坏数据或自动降级。

### Requirement: ST-03 Consistent paging
分页 SHALL 固定 revision、过滤与排序，使用稳定继续游标；版本淘汰后返回 revision_expired，不切换到最新版本。

#### Scenario: Snapshot removed between pages
- **WHEN** 请求下一页时目标 revision 已淘汰
- **THEN** 返回显式过期状态而非混合快照结果。

### Requirement: ST-04 Retention dependencies
索引治理 SHALL 统计主库、WAL、staging、备份及历史依赖，保留 pin 和引用批次；重建索引不得删除 scope 授权、用户保护、批准或恢复记录。

#### Scenario: Index rebuild
- **WHEN** 管理员重建某范围索引
- **THEN** 控制记录保留；旧操作计划失效或按绑定版本重验，不重置授权。

### Requirement: ST-05 Local ownership
本机及服务器索引 SHALL 由同一 Rust 存储语义管理，服务器数据库不作为网络共享文件提供给客户端；PruneX 不直接写图索引 schema。

#### Scenario: Two hosts
- **WHEN** 本地智能体连接服务器
- **THEN** 通过查询协议获取结果，而不是挂载并并发写服务器 SQLite。

### Requirement: ST-05 Owned history and consistent upgrade backup
The system SHALL persist revision server/scope ownership, list and resolve latest revisions by ownership, and backfill legacy ownership only for one exact matching registered locator. Unbound history SHALL be denied externally. Graph and control schema upgrades SHALL first use SQLite consistent backups including committed WAL frames. Explicit pruning SHALL preserve the latest revision, pins and operation/recovery references; preview SHALL be the default.

#### Scenario: Ambiguous display root
- **WHEN** distinct lossless scope locators share the same legacy display root
- **THEN** the old revision remains unbound and externally unreadable

#### Scenario: Retained history
- **WHEN** pruning old revisions with keep-last one
- **THEN** pinned and latest revisions survive and no referenced operation/recovery data is removed

### Requirement: Migration recovery backups are never overwritten
Graph and control database migrations SHALL reserve a new backup destination atomically and use SQLite consistent backup before migrating. Existing recovery files SHALL remain unchanged across retries and concurrent reservation conflicts.

#### Scenario: Retry finds an existing recovery backup
- **WHEN** a prior recovery backup occupies the preferred migration filename
- **THEN** the migration writes its consistent backup to a distinct newly reserved filename
- **AND** the prior recovery bytes remain unchanged
- **AND** migration failure does not return a usable store or overwrite previous recovery material

#### Scenario: Staging search inserts reuse one prepared statement per batch
- **WHEN** a scan appends a batch of nodes and Unicode normalized search fields to invisible staging
- **THEN** the search insert is compiled once per batch and executed with bound values for every node; increasing batch rows does not multiply statement preparation
- **AND** Unicode lowercase fields, staging ordering, atomic rollback and per-node cancellation checks remain unchanged

#### Scenario: Snapshot publication refuses disconnected parent cycles
- **WHEN** a completed graph has one matching root and existing parent IDs but also contains a self-parent or disconnected multi-node parent cycle
- **THEN** publication returns InvalidGraph before publishing any snapshot, ownership or latest pointer
- **AND** parent validation runs in linear graph traversal work, accepts valid deep trees in arbitrary node order, and does not recurse on the native call stack

#### Scenario: Unix 观测批次复用实际点查与写入语句
- **WHEN** 同一 checked 事务暂存多个 Unix 文件观测
- **THEN** 节点目标点查和观测 INSERT 各仅编译一次并复用参数绑定
- **AND** 保留每项前后授权/取消/期限检查、COUNT(*)=1 歧义拒绝和整批失败回滚；不得为性能移除原生身份校验

#### Scenario: Reuse the admitted staging encoding
- **WHEN** 扫描为当前有界批次计算节点原生观测的实际编码字节预算
- **THEN** 暂存写入复用同一份不可变已校验编码；节点 JSON、Unicode 搜索折叠、原始定位、时间和观测字段不得在预算准入后被替换
- **AND** 每项取消检查、提交前检查、fencing 命名空间和批次事务保持；取消或写入冲突回滚节点与搜索两表，旧借用接口保持兼容。缓存只存当前配置批次，不声明严格 RSS 上限。
