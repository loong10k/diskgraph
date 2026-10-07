# 历史 revision 归属隔离 / Historical revision ownership isolation

图库 schema v15 保留原 `revision_ownership` 和快照，增加 `revision_access_denials` 审计记录与固定授权视图。Engine 启动时，对有损根展示别名组重新检查已绑定 revision：唯一根节点的明确编码与原始字节必须匹配实际 scope，缺失或不一致的记录隔离。隔离记录不会出现在对外 scope 历史/latest，也不能成为读取、比较或采集目标。

旧库升级沿用 SQLite 一致性备份；迁移或固定视图核验失败不会返回可用的新 Engine。升级时停止旧服务，不并行运行绕过新授权视图的旧版本。备份路径位于数据目录的 `migration_backups`，文件名目标版本为 `pre-v15.bak`。

可信内部 `revision_ownership_for_audit` 可读取原归属，审计原因存于 `revision_access_denials.reason`，当前值为 `root_identity_unconfirmed`。原映射不会因显示匹配自动解除隔离；管理员须重新索引实际授权根，生成具有原始定位证明的新 revision。此流程不自动删除历史、不启用文件写操作。

Schema v15 retains snapshots and original ownership rows. It records unconfirmed root identities separately and excludes them through a fixed authorization view. During Engine startup, revisions in lossy display-alias groups require one root node whose explicit encoding and raw bytes match the registered scope. Missing or mismatched proof denies access while retaining audit data.

SQLite consistent backups precede upgrades. Failed migration or view validation prevents opening a usable Engine. Stop older services during upgrade; older binaries that bypass this view must not run alongside the new service. Reindex the authorized root to produce a new revision with native identity proof; display matching cannot clear isolation.

当前候选验证状态：macOS 隔离行为测试 8 项通过，相关历史与关系回归合计 40 项通过；Linux/Windows 原生结果、完整 workspace 与最终查询性能仍需验收。此文档不构成生产就绪证明。

Candidate validation: eight isolated behavior tests and 40 related history/relation regressions pass on macOS. Native Linux/Windows results, the complete workspace, and final query performance remain pending. This document is not production-readiness evidence.
