# 原 Windows private Git Recovery 显式重试验收

本轮只修正测试宿主遵守既有单次恢复合同，不改变生产 ID 查询或删除能力。原 native RED 保留于 windows_test_lifecycle_4000：实际 panic 后目录/进程恢复第一次返回 OS87，测试直接 unwrap 失败。

当前同一个 Recovery 在固定 10 秒期限内显式 drain_until；Pending/错误保留原诊断，每次核验原槽仍占用、新 session 明确 ResourceExhausted。不能根据 OS87、锁释放或 Pending 声明删除。结束仍须真实原 wait、Job0、原目录 absent、原容量归零，永久失败/到期仍失败。

本机 macOS 仅完成源码 AST 门禁6/0、Clippy/fmt/OpenSpec 严格验证；Windows 模块不在本机编译执行，不声称 native GREEN。当前 CI 的Windows测试还在运行。监督实际启动/IPC/有限前台退出门禁继续打开。
