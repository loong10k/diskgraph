# 普通 revision reader 终检锁竞争修复

## 复现与实现

Linux stable CI 37744394886（5dc825d）真实 socket 多主体授权测试中，合法 node 查询收到 budget_exceeded。当前普通 reader 在终检能力回调后 try_lock 失败即拒绝，未等待短暂竞争。隔离发布夹具中，能力回调确认另线程持有真实控制锁，持有20ms时旧实现返回 BudgetExceeded（1失败）。

候选实现让锁获取与 SQL 观察共享现有50ms窗口，并取原请求期限的较小值；获取锁后不刷新窗口。实时权限、撤权见证、归属终检与过期检查保留。专用 TUI 非阻塞路径未修改。

## 验证

- revision_reader_lock_gap_tests：2通过，含短竞争成功、500ms持锁在300ms内拒绝，以及首次授权锁间撤权拒绝。
- terminal_capability_tests：14通过。
- source_layout：6通过。
- Engine all-targets Clippy -D warnings、Engine fmt检查、OpenSpec strict验证通过。
- 原始日志见 docs/benchmarks/terminal_reader_contention_2026_10_08。

## 尚未完成

本地夹具验证不替代实际 socket 三平台运行；当前5dc825d CI不含此修复。整体生产就绪门禁不勾选。本次没有放宽预算或改变拒权测试断言。

当前 macOS Intel 包同一轮 CI再次出现 recovery slot open ENOENT；诊断显示目录 held_path_same 且条目 regular，仍需独立定位，不能归因于不存在目录或由本次 reader修复宣称解决。
