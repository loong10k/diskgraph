# 历史 revision 归属隔离 / Historical revision ownership isolation

图库 schema v15 保留原 `revision_ownership` 和快照，增加 `revision_access_denials` 审计记录与固定授权视图。Engine 启动时，对有损根展示别名组重新检查已绑定 revision：唯一根节点的明确编码与原始字节必须匹配实际 scope，缺失或不一致的记录隔离。隔离记录不会出现在对外 scope 历史/latest，也不能成为读取、比较或采集目标。

旧库升级沿用 SQLite 一致性备份；迁移或固定视图核验失败不会返回可用的新 Engine。升级时停止旧服务，不并行运行绕过新授权视图的旧版本。备份路径位于数据目录的 `migration_backups`，文件名目标版本为 `pre-v15.bak`。

可信内部 `revision_ownership_for_audit` 可读取原归属，审计原因存于 `revision_access_denials.reason`，当前值为 `root_identity_unconfirmed`。原映射不会因显示匹配自动解除隔离；管理员须重新索引实际授权根，生成具有原始定位证明的新 revision。此流程不自动删除历史、不启用文件写操作。

Schema v15 retains snapshots and original ownership rows. It records unconfirmed root identities separately and excludes them through a fixed authorization view. During Engine startup, revisions in lossy display-alias groups require one root node whose explicit encoding and raw bytes match the registered scope. Missing or mismatched proof denies access while retaining audit data.

SQLite consistent backups precede upgrades. Failed migration or view validation prevents opening a usable Engine. Stop older services during upgrade; older binaries that bypass this view must not run alongside the new service. Reindex the authorized root to produce a new revision with native identity proof; display matching cannot clear isolation.

当前候选验证状态：macOS 隔离行为测试 8 项通过，相关历史与关系回归合计 40 项通过；Linux/Windows 原生结果、完整 workspace 与最终查询性能仍需验收。此文档不构成生产就绪证明。

Candidate validation: eight isolated behavior tests and 40 related history/relation regressions pass on macOS. Native Linux/Windows results, the complete workspace, and final query performance remain pending. This document is not production-readiness evidence.

## 运行中撤权与注册事务 / Runtime isolation and registration transactions

scope 注册会重新核验原生根别名组。对外查询在最终返回前使用新读连接重新读取授权视图；旧消费者连接的读事务不能维持已隔离 revision 的访问资格。关系、显示树以及历史比较的两侧均执行末段核验，沿用原查询预算或既有末段授权窗口，不重新签发数据预算。原归属仍供可信内部审计读取。

注册 scope 与三项默认 grant 在控制库同一个 `BEGIN IMMEDIATE` 事务中写入。请求权限在取得锁后重新检查，持久管理员授权与固定 policy epoch 在事务内检查，图库负向隔离先于控制库提交。提交前失败回滚 scope/grants；已经发生的图库隔离保留为 fail-closed 状态。COMMIT 后连接清理失败返回 `RegistrationCommitted`，携带已提交 scope 与原错误，不投射为认证失败。五秒提交准入窗口不是文件系统、Mutex 或图库回调的硬墙钟返回保证。

Runtime scope registration reconciles native root aliases. External reads recheck the authorization view before returning, using a fresh read connection so a consumer-held snapshot cannot preserve access to a quarantined revision. Tree, relation and both historical inputs retain the original data budget or existing terminal authorization window. Trusted audit access still retains the original ownership row.

Scope and default grants share a control-database `BEGIN IMMEDIATE` transaction. Request authority is checked after lock acquisition; persistent admin authority and a fixed policy epoch are checked inside the transaction. Graph denial precedes control commit. Pre-commit failures roll back scope and grants; graph denial already applied remains fail-closed. A post-COMMIT connection-cleanup failure reports `RegistrationCommitted` with the committed scope and original error. The five-second commit-admission window does not guarantee a hard return deadline for filesystem calls, Mutex acquisition or graph callbacks.

新增隔离夹具覆盖查询中注册导致隔离的 28 种路径；FFI 原始根身份兼容夹具六项回归已在本机通过。证据目录为 `docs/benchmarks/runtime_authorization_54d9/`。本机 FFI 全库结果为 39 通过、52 失败；41 项明确 Unsupported，其余失败仍需逐项追踪，不能以目标测试通过替代全库验收。Windows 清理错误只增加阶段诊断，尚无新原生运行证明，也未将 OS 87 当作删除成功。

The runtime isolation fixture covers 28 read/registration paths. Six FFI native-root compatibility regressions pass locally. Full local FFI testing reports 39 passed and 52 failed, including 41 explicit Unsupported results; the remaining failures require individual tracing. Targeted passes do not replace full-suite acceptance. Windows cleanup now identifies its failing phase; native validation is pending and OS error 87 is never interpreted as deletion success.

旧 `explain_entity`、`related`、`related_page` 公共签名现使用独立窄读连接和首末实时授权。实体、关系、证据以及分页结果结构保持一致；新增默认一秒查询期限，慢调用会失败。Rust 弃用标记会产生编译警告，使用 `deny(deprecated)` 的下游需迁移或显式允许兼容调用。旧签名仍没有完整节点/字节额度，新外部请求须使用 `explain_bounded_until` / `related_bounded_until`，不能把旧接口终检修复视为完整资源预算验收。

The legacy public `explain_entity`, `related`, and `related_page` signatures now use an independent narrow reader with initial and terminal live authorization. Entity, edge, evidence and pagination result structures are unchanged. A new default one-second query deadline can fail slower calls; deprecation warnings require migration or an explicit compatibility allowance in downstream crates using `deny(deprecated)`. These legacy signatures still lack total node/byte budgets. New external requests must use `explain_bounded_until` / `related_bounded_until`; terminal authorization is not full resource-budget acceptance.

历史比较每轮末段归属观察在能力回调之后开始 50ms 窗口，同轮两侧共享，最多三轮。该观察不刷新原查询期限；迟到结果仍执行原 Timeout/partial 规则。三个窗口不代表整次仅 50ms，也不提供可抢占同步回调的硬返回期限。

History comparisons begin each bounded 50ms ownership observation after the capability callback, sharing it across both revisions, for at most three rounds. The original query deadline stays unchanged; late results retain Timeout/partial semantics. This is not a hard return deadline including synchronous callbacks.
