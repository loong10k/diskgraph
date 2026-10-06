# macOS 单次期限内原 owner 恢复 / Deadline-aware retained recovery

`ScanWorkerRecovery::drain_until` 在 macOS 进入原恢复表的单次非阻塞路径，和 Windows 共用真实 owner 归还守卫。过期或状态锁竞争不取 owner、不发信号；原组失败、外部 ECHILD 或 panic 保留原槽。只有完整私有组退出观察和原 leader WNOHANG 消费均完成才返还容量。普通兼容 child 不获得新资格，原 drain 行为保留。

TDD 添加新期限入口、暂接既有 legacy drain 后，真实过期请求测试实际 RED：0/1。该过渡接线不是声称历史公共 API 已存在。修复后目标通过；组失败、末段真实外部 wait 抢占及三次容量重试通过；原生与标准 leader 活动/实际回收缓存通过；panic 和竞争回槽通过。最新并行 native_child 44/0/1，唯一 ignored 辅助夹具由真实父测试执行。真实父驱动 7/0、源码组织 6/0、验收驱动 28/0 和14/0通过。

早期宽范围并行回归出现正常组成员查询不完整：23/1/1。原失败日志保留；精确复查1/0及串行24/0/1不抹掉该不稳定点。Clippy all lint通过，但21项既有Rust dead-code/unused警告仍在，严格全workspace检查不通过。归档只覆盖本次拥有的源码，原vendor、锁及其他源字节保持；Windows510源/Mac620源资格清单尚待本次提交原生CI。

OS单调用和责任归还短状态锁不承诺硬墙钟保证。Linux deadline API、CLI/MCP完整有限退出、通知/目录恢复及默认安装扫描未完成；不丢最后Recovery句柄、不启用危险操作、不勾选生产父项。

The actual expired-deadline regression failed against the legacy drain routed through the new entry, then passed after the fix. Native wait, group-error, external-reap, panic and contention ownership regressions passed; latest parallel native-child result is 44/0/1 with the isolated helper actually executed. Original earlier parallel failure is retained. This is macOS library recovery progress, not full platform or finite frontend-shutdown acceptance. Strict workspace warnings and the remaining product gates stay open.
