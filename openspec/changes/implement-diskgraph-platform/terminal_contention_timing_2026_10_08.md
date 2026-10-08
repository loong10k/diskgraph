# 普通 reader 终检锁竞争诊断（2026-10-08）

对应 bounded-queries 的 Ordinary revision readers tolerate brief terminal control contention 场景。

当前 macOS CI 的短竞争返回 BudgetExceeded，但旧日志只有请求 sleep 20ms，没有实际持锁时间、回调线程启动耗时或终检耗时，不能区分调度延迟与查询实现错误。本次只补充现有真实锁竞争回归的测量和失败信息，不改生产逻辑，不放宽原 50ms 观察窗口、1s 请求期限或成功断言。

本机 macOS ARM64 目标测试 3/3；四个并行独立测试进程共执行 16 次竞争回归，16/16 通过；Engine lib/tests Clippy 通过。最终目标测试短竞争实际持锁 25.07ms、终检 26.30ms，长竞争终检 50.29ms 返回 BudgetExceeded，测量时锁尚未释放（actual_hold=None）。None 表示尚未观察到释放，不是零耗时。

原 CI 失败仍保持未关闭。此次本机通过不能证明 macOS Intel、Linux、Windows 或拥挤 CI 下稳定；下一次相同提交 CI 应根据新诊断定位真实超时阶段。现有负向撤权优先、期限耗尽拒绝与长竞争回归均保留。没有新增或勾选生产就绪任务。

原始压缩日志及源码摘要见 docs/benchmarks/terminal_contention_timing_2026_10_08/。

## 下一轮 macOS CI 失败隔离

现有失败后串行诊断从 Intel 扩展到 macOS 各矩阵，保留原五个诊断用例，并加入本轮实际失败的短终检锁竞争与历史尺寸用例。每个用例仍要求真实 exact 测试名称、1 passed/0 failed/0 ignored，原完整 workspace 失败不会被诊断成功覆盖。诊断工件名称包含 OS、工具链和 run，避免多个矩阵覆盖同名输出。原 30 分钟 workspace 门槛、16 分钟诊断门槛和全部业务时间额度保持不变。

该配置变更只经过本地 YAML/内嵌 Python 语法及用例清单检查；实际诊断结果必须等待新提交 CI，不能据配置存在关闭原 macOS 失败。
