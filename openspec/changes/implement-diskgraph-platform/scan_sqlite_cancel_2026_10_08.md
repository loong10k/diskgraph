# 扫描原 fence 写锁等待取消

## 红灯与实现

隔离真实控制库另连接持有 BEGIN IMMEDIATE；采样线程已完成原只读权限检查并进入 fence 准入，取消原执行。旧实现直到竞争写锁释放才返回 Conflict，300ms 内返回断言失败（1失败），并非环境故障。

ControlWriteDeadline 的显式事务边界增加借用执行检查器，每次真实 BUSY 重试前检查，不给 SQLite 注册借用或不安全回调。新增 with_scan_observation_fence_until 复用原 owner/lease/fence/权限 SQL 和原期限。既有 with_job_fence_until 接口保留，内部使用无额外条件的检查器；写工作仍只执行一次。采样桥接保存原 EngineError，避免将 keeper 的 PermissionDenied 转成存储 Conflict 或预算错误。BEGIN/COMMIT 失败仍清理 progress、回滚并恢复 busy timeout。

## 当前本地验证

- scan_observation_deadline_tests：6通过，包含普通取消、原 keeper 拒权、持锁期限、持久撤权及 fence 分类。
- control_write_deadline_tests：5通过；新增独立连接始终持有写锁时第二次检查器调用停止实际 BUSY 重试，并验证后续写入及731ms配置恢复。
- Store完整测试：358通过、9忽略；忽略项不计为验收。
- Engine source_layout：6通过；Engine/Store all-targets Clippy -D warnings、格式检查、OpenSpec strict通过。
- 原始日志：docs/benchmarks/scan_sqlite_cancel_2026_10_08。

## 限制

原生单次 I/O 仍不可抢占；短 SQL VM 的取消未由此宣称完成。当前远程 CI 的5dc825d不含本次修改；目标平台须在后续同SHA验证，整体生产门禁不勾选。
