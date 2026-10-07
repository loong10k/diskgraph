# 4000a85 原生失败与测试生命周期顺序

run 37549794669 实际终态：Linux arm64 stable 和 x86 MSRV 成功；Linux x86 stable 全量 Test 成功、随后 paired performance 的旧基线 200k 失败。Windows stable job112562289975 为 567 passed/3 failed/3 ignored，MSRV job112562289888 也为 567/3/3。完整原终态日志压缩保留，不据部分通过宣称全平台完成。

两个共同失败分别是 source root 允许第一次改名后、原 GitView 来源捕获还存活时第二次恢复得到 OS32；以及 Deadline 主错误携带未完成清理时原会话/Recovery 尚未排空就断言心跳停止。仅调整测试生命周期：原 terminal 验证保留，complete/drop 原 view 与预算后恢复隔离名称；原 Deadline 主错误断言保留，结束原 NativeProbeTestBudget、既有外部 Recovery 实际 drain 后观察原心跳。没有改生产 sharing flags、错误匹配、期限、Job 或 cleanup 算法。

本机 macOS arm64：目标各1/0、作用域13/0、探针19/0、源码规范6/0、Clippy/fmt/OpenSpec严格验证通过。Windows 修复后原生结果仍待 CI。Deadline 的兼容测试 drain 仍可能等待，不代表产品前端已实现有限退出。

仍未解决：Windows stable 原 private directory recovery 的未知 NT INVALID_PARAMETER/OS87；MSRV observation hook 在原3秒 token 到期前未达到；Linux 旧性能基线失败。没有把 OS87 当作 absent、删除原owner、放宽 token 或接受性能部分结果。监督启动/IPC/CLI/MCP 实际退出与全平台生产门禁继续打开。

receipt.json 记录原日志正文及归档 SHA256、当前修复源码指纹；压缩不改日志内容。
