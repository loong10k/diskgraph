# 原 Engine 与 Recovery 绑定、原监督槽退休子组件

正式规格：implement-diskgraph-platform/frontend_recovery_supervisor.md。原 Arc 身份绑定逐一检查 Engine 已配置的扫描/Windows 探针池；原绑定期限与 ACTIVE 记录校验失败、缺失/外国/额外 Recovery、没有受管理资源的 Engine 均整体返回原材料，不 seal、不丢弃原资源。现有旧 API 行为不变。

退休关闭原资源新准入，Arc::try_unwrap 原子取得 Engine 唯一所有权；外部强引用存在时返回原同一 Arc 并保持 Pending。成功后销毁 Engine，旧 Weak 无法升级；Recovery 仍持有原表。两个原表沿同次 deadline 实际 drain_until 返回 Complete 后，才在原槽写 CLEAN/sync_all/精确回读，成功后关闭原锁。原资源 Pending/异常/状态损坏保持原槽占用；不会根据外部布尔消息释放。

本机 macOS arm64：真实绑定 RED 5/2（外国和缺失 Recovery 均被错误接纳）→ GREEN 9/0。覆盖真实未出生预留 Pending、外部 Engine 强引用、旧 Weak 不可再升级、原槽 Busy、实际释放后再认领、过期绑定/退休、损坏记录、Pending 对象 Drop 留未确认、未管理/额外恢复材料拒绝。两项 Windows ProbeHost 原会话/外国池测试仅 cfg(windows)，未在本机运行。

另一个真实 Unix fd 写入异常测试 1/0：只改变原 held fd 的 O_APPEND，实际写入变成 16 字节；精确回读拒绝并保留原锁，同一对象重试不修复损坏状态，Drop 后旧槽仍拒绝认领。没有用布尔假 I/O 失败或替换 owner；此测试不证明原生 sync_all 错误重试。

受影响回归：源码门禁 6/0；私有控制通知 10/0；原槽 7/0（单独 ignored 的 child 入口由父测试实际调用）；原扫描资源回归 7/0；Engine all-targets Clippy、fmt、OpenSpec 严格验证通过。原始日志与源码 SHA256 在 receipt.json。

不宣称原生子进程/Job/I/O 的监督执行、真实 runner join、监督地址空间出生、受保护容量命名空间、CLI/MCP 公共 EOF 或三平台生产门禁完成。库内退休不会自行创建监督进程或改变旧前端无限恢复循环。
